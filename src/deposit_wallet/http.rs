use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, SystemTime};

use ::url::Url;
use ethers::types::{Address, H256, U256};
use ethers::utils::{keccak256, to_checksum};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, CONTENT_TYPE, RETRY_AFTER};
use reqwest::{Client, Method, StatusCode};
use secrecy::{ExposeSecret, SecretString};
use serde::de::{self, SeqAccess, Visitor};
use serde::Deserialize;
use serde::Deserializer;

use crate::deposit_wallet::{
    build_deposit_wallet_batch_request_from_signed, build_wallet_create_request,
    build_wallet_nonce_request, deposit_wallet_contract_config, DepositWalletContractConfig,
    RelayerSubmitResponse, RelayerTransactionState, SignedDepositWalletBatch, POLYGON_CHAIN_ID,
};
use crate::deposit_wallet::config::deposit_wallet_contract_chain_id;
use crate::error::{RelayerError, Result};

const RELAYER_HOST: &str = "relayer-v2.polymarket.com";
const SUBMIT_PATH: &str = "/submit";
const TRANSACTION_PATH: &str = "/transaction";
const MAX_SUCCESS_BODY_BYTES: usize = 64 * 1024;
const MAX_TRANSACTION_SUCCESS_BODY_BYTES: usize = 256 * 1024;
const MAX_ERROR_BODY_DRAIN_BYTES: usize = 8 * 1024;
#[cfg(not(test))]
const ERROR_BODY_DRAIN_TIMEOUT: Duration = Duration::from_millis(50);
#[cfg(test)]
const ERROR_BODY_DRAIN_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_BACKGROUND_ERROR_BODY_DRAINS: usize = 64;
const RESPONSE_BODY_TOO_LARGE_MESSAGE: &str = "relayer response body exceeded maximum size";
const MAX_TRANSACTION_ID_LEN: usize = 128;
const MAX_TRANSACTION_RESPONSE_ITEMS: usize = 32;
const TRANSACTION_RESPONSE_ITEM_LIMIT_ERROR: &str = "transaction response item limit exceeded";
const TRANSACTION_RESPONSE_DUPLICATE_ID_ERROR: &str = "transaction response duplicate id";
const TRANSACTION_RESPONSE_MISSING_ID_ERROR: &str = "transaction response missing requested id";
const MAX_ERROR_TOKEN_LEN: usize = 96;
const MAX_OWNER_MUTATION_RECORDS: usize = 1024;
const MAX_OWNER_SERIALIZATION_LEASE_SECONDS: u64 = 300;
const MAX_EVIDENCE_CLOCK_SKEW_SECONDS: u64 = 30;

mod auth;
mod permit;
mod read;
mod redaction;
mod response;
mod state;
mod submit_flow;
mod transport;
mod url;

pub use auth::RelayerKeyAuth;
pub use permit::{
    DepositWalletIdlessSubmitReconciliationEvidence, DepositWalletMutationAction,
    DepositWalletMutationEnvironment, DepositWalletMutationGate, DepositWalletMutationPermit,
    DepositWalletMutationScope, DepositWalletOwnerSerializationEvidence,
    DepositWalletSubmitReconciliationEvidence, DepositWalletSubmitReconciliationObservation,
};
pub use read::DepositWalletNonceLease;
pub use response::DepositWalletTransactionReceipt;
pub use url::DepositWalletRelayerUrl;

use state::OwnerMutationStore;
use transport::ErrorBodyDrainLimiter;
use url::validate_relayer_contract_config;

/// HTTP client for deposit-wallet relayer calls.
///
/// Owner mutation state is local to this client allocation: cloned clients share
/// the same in-memory store, but separately constructed clients do not
/// coordinate nonce leases or submit reservations. Production mutation permits
/// remain unavailable in this PR; shared or durable owner state belongs with the
/// later live-execution capability. Consumers should construct one client per
/// relayer/auth/config tuple and clone it for repeated work instead of creating
/// a fresh client per request.
#[derive(Clone)]
pub struct DepositWalletRelayerClient {
    http: Client,
    base_url: DepositWalletRelayerUrl,
    auth: RelayerKeyAuth,
    config: DepositWalletContractConfig,
    mutation_state: Arc<OwnerMutationStore>,
    error_body_drain_limiter: ErrorBodyDrainLimiter,
    clock: Arc<dyn DepositWalletClock>,
}

impl DepositWalletRelayerClient {
    pub fn new(
        base_url: DepositWalletRelayerUrl,
        auth: RelayerKeyAuth,
        config: DepositWalletContractConfig,
    ) -> Result<Self> {
        validate_relayer_contract_config(&base_url, config)?;
        let http = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(30))
            .build()?;

        Ok(Self::from_parts(http, base_url, auth, config, Arc::new(SystemClock)))
    }

    fn from_parts(
        http: Client,
        base_url: DepositWalletRelayerUrl,
        auth: RelayerKeyAuth,
        config: DepositWalletContractConfig,
        clock: Arc<dyn DepositWalletClock>,
    ) -> Self {
        Self {
            http,
            base_url,
            auth,
            config,
            mutation_state: Arc::new(OwnerMutationStore::default()),
            error_body_drain_limiter: ErrorBodyDrainLimiter::new(MAX_BACKGROUND_ERROR_BODY_DRAINS),
            clock,
        }
    }

    pub fn try_mutation_scope(
        &self,
        action: DepositWalletMutationAction,
    ) -> Result<DepositWalletMutationScope> {
        let chain_id = deposit_wallet_contract_chain_id(self.config)?;
        Ok(DepositWalletMutationScope::new(
            chain_id,
            self.config.factory,
            self.config.implementation,
            self.base_url.mutation_environment(),
            action,
        ))
    }

    pub fn mutation_scope(
        &self,
        action: DepositWalletMutationAction,
    ) -> Result<DepositWalletMutationScope> {
        self.try_mutation_scope(action)
    }
}

impl fmt::Debug for DepositWalletRelayerClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DepositWalletRelayerClient")
            .field("base_url", &self.base_url)
            .field("auth", &self.auth)
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

trait DepositWalletClock: Send + Sync {
    fn now_unix_seconds(&self) -> Result<u64>;
}

struct SystemClock;

impl DepositWalletClock for SystemClock {
    fn now_unix_seconds(&self) -> Result<u64> {
        SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .map_err(|_| RelayerError::Other("system time before UNIX epoch".to_string()))
    }
}

#[cfg(test)]
mod tests;
