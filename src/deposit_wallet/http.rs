use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use ::url::Url;
use ethers::types::{Address, U256};
use ethers::utils::{keccak256, to_checksum};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, CONTENT_TYPE, RETRY_AFTER};
use reqwest::{Client, Method, StatusCode};
use secrecy::{ExposeSecret, SecretString};
use serde::de::{self, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};

use crate::deposit_wallet::config::deposit_wallet_contract_chain_id;
use crate::deposit_wallet::{
    build_wallet_nonce_request, deposit_wallet_contract_config, derive_deposit_wallet_address,
    DepositWalletContractConfig, RelayerSubmitResponse, RelayerTransactionState, POLYGON_CHAIN_ID,
    WALLET_TRANSACTION_TYPE,
};
use crate::error::{RelayerError, Result};

const RELAYER_HOST: &str = "relayer-v2.polymarket.com";
const TRANSACTION_PATH: &str = "/transaction";
const DEPLOYED_PATH: &str = "/deployed";
const SUBMIT_PATH: &str = "/submit";
const MAX_SUCCESS_BODY_BYTES: usize = 64 * 1024;
const MAX_TRANSACTION_SUCCESS_BODY_BYTES: usize = 256 * 1024;
const MAX_ERROR_BODY_DRAIN_BYTES: usize = 8 * 1024;
const ERROR_BODY_DRAIN_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_BACKGROUND_ERROR_BODY_DRAINS: usize = 64;
const RESPONSE_BODY_TOO_LARGE_MESSAGE: &str = "relayer response body exceeded maximum size";
const MAX_TRANSACTION_ID_LEN: usize = 128;
const MAX_TRANSACTION_RESPONSE_ITEMS: usize = 32;
const TRANSACTION_RESPONSE_ITEM_LIMIT_ERROR: &str = "transaction response item limit exceeded";
const TRANSACTION_RESPONSE_DUPLICATE_ID_ERROR: &str = "transaction response duplicate id";
const TRANSACTION_RESPONSE_MISSING_ID_ERROR: &str = "transaction response missing requested id";

mod auth;
mod capability;
mod clock;
mod deployed;
mod mutation;
mod read;
mod redaction;
mod response;
mod submit;
mod transport;
mod url;

pub use auth::RelayerKeyAuth;
pub use capability::RelayerReadPermit;
pub use mutation::{
    DepositWalletDryRunEvidence, DepositWalletSubmitReceipt, DryRunCallSummary,
    RelayerMutationMode, RelayerMutationOperation, RelayerMutationPermit, RelayerSubmitOutcome,
};
pub use response::DepositWalletTransactionReceipt;
pub use url::DepositWalletRelayerUrl;

use clock::{RelayerClock, SystemClock};
use transport::ErrorBodyDrainLimiter;
use url::validate_relayer_contract_config;

#[derive(Clone)]
pub struct DepositWalletRelayerClient {
    http: Client,
    base_url: DepositWalletRelayerUrl,
    auth: RelayerKeyAuth,
    config: DepositWalletContractConfig,
    clock: Arc<dyn RelayerClock>,
    mutation_gate: Arc<AtomicBool>,
    error_body_drain_limiter: ErrorBodyDrainLimiter,
}

impl DepositWalletRelayerClient {
    pub fn new(
        base_url: DepositWalletRelayerUrl,
        auth: RelayerKeyAuth,
        config: DepositWalletContractConfig,
    ) -> Result<Self> {
        Self::new_with_mutation_state(base_url, auth, config, false)
    }

    /// Creates a client whose mutation gate starts enabled.
    ///
    /// Operator rollback is one-way: after `disable_mutation()` is called, this
    /// instance and all of its clones are permanently read-only.
    pub fn new_with_mutation_enabled(
        base_url: DepositWalletRelayerUrl,
        auth: RelayerKeyAuth,
        config: DepositWalletContractConfig,
    ) -> Result<Self> {
        Self::new_with_mutation_state(base_url, auth, config, true)
    }

    fn new_with_mutation_state(
        base_url: DepositWalletRelayerUrl,
        auth: RelayerKeyAuth,
        config: DepositWalletContractConfig,
        mutation_enabled: bool,
    ) -> Result<Self> {
        validate_relayer_contract_config(&base_url, config)?;
        let http = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(30))
            .build()?;

        Ok(Self::assemble(
            http,
            base_url,
            auth,
            config,
            Arc::new(SystemClock),
            mutation_enabled,
        ))
    }

    #[cfg(test)]
    fn from_parts(
        http: Client,
        base_url: DepositWalletRelayerUrl,
        auth: RelayerKeyAuth,
        config: DepositWalletContractConfig,
    ) -> Self {
        Self::from_parts_with(
            http,
            base_url,
            auth,
            config,
            Arc::new(SystemClock),
            false,
        )
    }

    #[cfg(test)]
    fn from_parts_with(
        http: Client,
        base_url: DepositWalletRelayerUrl,
        auth: RelayerKeyAuth,
        config: DepositWalletContractConfig,
        clock: Arc<dyn RelayerClock>,
        mutation_enabled: bool,
    ) -> Self {
        Self::assemble(
            http,
            base_url,
            auth,
            config,
            clock,
            mutation_enabled,
        )
    }

    fn assemble(
        http: Client,
        base_url: DepositWalletRelayerUrl,
        auth: RelayerKeyAuth,
        config: DepositWalletContractConfig,
        clock: Arc<dyn RelayerClock>,
        mutation_enabled: bool,
    ) -> Self {
        Self {
            http,
            base_url,
            auth,
            config,
            clock,
            mutation_gate: Arc::new(AtomicBool::new(mutation_enabled)),
            error_body_drain_limiter: ErrorBodyDrainLimiter::new(MAX_BACKGROUND_ERROR_BODY_DRAINS),
        }
    }

    /// Permanently disables live mutation for this instance and all of its clones.
    ///
    /// Read-only observation and dry-run evidence generation remain available.
    pub fn disable_mutation(&self) {
        self.mutation_gate.store(false, Ordering::SeqCst);
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

#[cfg(test)]
mod tests;
