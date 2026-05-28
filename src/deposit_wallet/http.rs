// PR #20 keeps transaction and nonce reads crate-internal until WALLET polling
// evidence and nonce lease semantics land in later stack PRs. The internal HTTP
// harness is still compiled for tests and downstream stack branches.
#![cfg_attr(not(test), allow(dead_code))]

use std::fmt;
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

use crate::deposit_wallet::{
    build_wallet_nonce_request, deposit_wallet_contract_config, DepositWalletContractConfig,
    RelayerSubmitResponse, RelayerTransactionState, POLYGON_CHAIN_ID,
};
use crate::error::{RelayerError, Result};

const RELAYER_HOST: &str = "relayer-v2.polymarket.com";
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
const TRANSACTION_RESPONSE_INVALID_ID_ERROR: &str = "transaction response invalid id";

mod auth;
mod read;
mod redaction;
mod response;
mod transport;
mod url;

pub use auth::RelayerKeyAuth;
pub use response::DepositWalletTransactionReceipt;
pub use url::DepositWalletRelayerUrl;

use transport::ErrorBodyDrainLimiter;
use url::validate_relayer_contract_config;

#[derive(Clone)]
pub struct DepositWalletRelayerClient {
    http: Client,
    base_url: DepositWalletRelayerUrl,
    auth: RelayerKeyAuth,
    config: DepositWalletContractConfig,
    error_body_drain_limiter: ErrorBodyDrainLimiter,
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

        Ok(Self::from_parts(http, base_url, auth, config))
    }

    fn from_parts(
        http: Client,
        base_url: DepositWalletRelayerUrl,
        auth: RelayerKeyAuth,
        config: DepositWalletContractConfig,
    ) -> Self {
        Self {
            http,
            base_url,
            auth,
            config,
            error_body_drain_limiter: ErrorBodyDrainLimiter::new(MAX_BACKGROUND_ERROR_BODY_DRAINS),
        }
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
