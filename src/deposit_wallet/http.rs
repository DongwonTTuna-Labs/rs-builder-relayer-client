use std::fmt;
use std::sync::Arc;
#[cfg(test)]
use std::time::{Duration, SystemTime};

use ::url::Url;
use ethers::types::Address;
#[cfg(test)]
use ethers::types::U256;
use ethers::utils::{keccak256, to_checksum};
use reqwest::header::HeaderValue;
#[cfg(test)]
use reqwest::header::{HeaderMap, HeaderName, CONTENT_TYPE, RETRY_AFTER};
#[cfg(test)]
use reqwest::{Client, Method, StatusCode};
use secrecy::{ExposeSecret, SecretString};
#[cfg(test)]
use serde::de::{self, SeqAccess, Visitor};
#[cfg(test)]
use serde::{Deserialize, Deserializer};

use crate::deposit_wallet::{
    deposit_wallet_contract_config, DepositWalletContractConfig, RelayerTransactionState,
    AMOY_CHAIN_ID, POLYGON_CHAIN_ID,
};
#[cfg(test)]
use crate::deposit_wallet::{build_wallet_nonce_request, RelayerSubmitResponse};
use crate::error::{RelayerError, Result};

const POLYGON_RELAYER_HOST: &str = "relayer-v2.polymarket.com";
const AMOY_RELAYER_HOST: &str = "relayer-v2-staging.polymarket.dev";
#[cfg(test)]
const TRANSACTION_PATH: &str = "/transaction";
#[cfg(test)]
const MAX_SUCCESS_BODY_BYTES: usize = 64 * 1024;
#[cfg(test)]
const MAX_TRANSACTION_SUCCESS_BODY_BYTES: usize = 256 * 1024;
#[cfg(test)]
const MAX_ERROR_BODY_DRAIN_BYTES: usize = 8 * 1024;
#[cfg(test)]
const ERROR_BODY_DRAIN_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(test)]
const MAX_BACKGROUND_ERROR_BODY_DRAINS: usize = 64;
#[cfg(test)]
const RESPONSE_BODY_TOO_LARGE_MESSAGE: &str = "relayer response body exceeded maximum size";
#[cfg(test)]
const MAX_TRANSACTION_ID_LEN: usize = 128;
#[cfg(test)]
const MAX_TRANSACTION_RESPONSE_ITEMS: usize = 32;
#[cfg(test)]
const TRANSACTION_RESPONSE_ITEM_LIMIT_ERROR: &str = "transaction response item limit exceeded";
#[cfg(test)]
const TRANSACTION_RESPONSE_DUPLICATE_ID_ERROR: &str = "transaction response duplicate id";
#[cfg(test)]
const TRANSACTION_RESPONSE_MISSING_ID_ERROR: &str = "transaction response missing requested id";

mod auth;
#[cfg(test)]
mod read;
mod redaction;
mod response;
#[cfg(test)]
mod transport;
mod url;

pub use auth::RelayerKeyAuth;
pub use response::DepositWalletTransactionReceipt;
pub use url::DepositWalletRelayerUrl;

#[cfg(test)]
use transport::ErrorBodyDrainLimiter;
use url::validate_relayer_contract_config;

#[derive(Clone)]
pub struct DepositWalletRelayerClient {
    #[cfg(test)]
    http: Client,
    base_url: DepositWalletRelayerUrl,
    auth: RelayerKeyAuth,
    config: DepositWalletContractConfig,
    #[cfg(test)]
    error_body_drain_limiter: ErrorBodyDrainLimiter,
}

impl DepositWalletRelayerClient {
    pub fn new(
        base_url: DepositWalletRelayerUrl,
        auth: RelayerKeyAuth,
        config: DepositWalletContractConfig,
    ) -> Result<Self> {
        validate_relayer_contract_config(&base_url, config)?;
        #[cfg(test)]
        {
            let http = Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(30))
                .build()?;

            Ok(Self::from_parts(http, base_url, auth, config))
        }

        #[cfg(not(test))]
        {
            Ok(Self {
                base_url,
                auth,
                config,
            })
        }
    }

    #[cfg(test)]
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
