use std::fmt;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use ::url::Url;
use ethers::types::{Address, U256};
use ethers::utils::{keccak256, to_checksum};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, CONTENT_TYPE, RETRY_AFTER};
use reqwest::{Client, Method, StatusCode};
use secrecy::{ExposeSecret, SecretString};
#[cfg(test)]
use serde::de::{self, SeqAccess, Visitor};
#[cfg(test)]
use serde::{Deserialize, Deserializer};

use crate::deposit_wallet::{
    build_deposit_wallet_batch_request_from_signed, build_wallet_create_request,
    build_wallet_nonce_request, deposit_wallet_contract_config, derive_deposit_wallet_address,
    DepositWalletContractConfig, RelayerSubmitResponse, RelayerTransactionState,
    SignedDepositWalletBatch, AMOY_CHAIN_ID, POLYGON_CHAIN_ID,
};
use crate::error::{RelayerError, Result};

const POLYGON_RELAYER_HOST: &str = "relayer-v2.polymarket.com";
const AMOY_RELAYER_HOST: &str = "relayer-v2-staging.polymarket.dev";
#[cfg(test)]
const TRANSACTION_PATH: &str = "/transaction";
const SUBMIT_PATH: &str = "/submit";
const MAX_SUCCESS_BODY_BYTES: usize = 64 * 1024;
#[cfg(test)]
const MAX_TRANSACTION_SUCCESS_BODY_BYTES: usize = 256 * 1024;
const MAX_ERROR_BODY_DRAIN_BYTES: usize = 8 * 1024;
const ERROR_BODY_DRAIN_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_BACKGROUND_ERROR_BODY_DRAINS: usize = 64;
const RESPONSE_BODY_TOO_LARGE_MESSAGE: &str = "relayer response body exceeded maximum size";
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

    pub async fn submit_wallet_create(
        &self,
        owner: Address,
    ) -> Result<DepositWalletTransactionReceipt> {
        self.validate_amoy_submit_preflight()?;
        let deposit_wallet = derive_deposit_wallet_address(owner, self.config)?;
        let request = build_wallet_create_request(owner, self.config);
        let body = serialize_submit_request(&request)?;
        let response = self
            .send(Method::POST, self.base_url.endpoint(SUBMIT_PATH), Some(body))
            .await?;
        response::parse_submit_response(&response, Some(owner), Some(deposit_wallet))
    }

    pub async fn submit_signed_wallet_batch(
        &self,
        signed: SignedDepositWalletBatch,
    ) -> Result<DepositWalletTransactionReceipt> {
        self.validate_amoy_submit_preflight()?;
        let owner = signed.submit_from();
        let deposit_wallet = signed.deposit_wallet();
        let request = build_deposit_wallet_batch_request_from_signed(signed, self.config)?;
        let body = serialize_submit_request(&request)?;
        let response = self
            .send(Method::POST, self.base_url.endpoint(SUBMIT_PATH), Some(body))
            .await?;
        response::parse_submit_response(&response, Some(owner), Some(deposit_wallet))
    }

    fn validate_amoy_submit_preflight(&self) -> Result<()> {
        if !self.base_url.allows_amoy_submit() {
            return Err(mutation_blocked(
                "production submit requires the validated Amoy relayer host for chain 80002",
            ));
        }

        if self.config != deposit_wallet_contract_config(AMOY_CHAIN_ID)? {
            return Err(mutation_blocked(
                "production submit requires chain 80002 Amoy deposit wallet contract config",
            ));
        }

        Ok(())
    }
}

fn serialize_submit_request<T: serde::Serialize>(request: &T) -> Result<String> {
    serde_json::to_string(request).map_err(|error| RelayerError::Abi(error.to_string()))
}

fn mutation_blocked(message: impl Into<String>) -> RelayerError {
    RelayerError::Other(format!("Deposit-wallet mutation blocked: {}", message.into()))
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
