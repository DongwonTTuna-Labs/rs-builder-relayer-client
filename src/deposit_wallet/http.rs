use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ethers::types::{Address, U256};
use ethers::utils::{keccak256, to_checksum};
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use reqwest::{Client, Method, StatusCode};
use serde::Deserialize;
use url::Url;

use crate::deposit_wallet::{
    build_deposit_wallet_batch_request_from_signed, build_wallet_create_request,
    build_wallet_nonce_request, DepositWalletContractConfig, RelayerSubmitResponse,
    RelayerTransactionState, SignedDepositWalletBatch,
};
use crate::error::{RelayerError, Result};

const RELAYER_HOST: &str = "relayer-v2.polymarket.com";
const SUBMIT_PATH: &str = "/submit";
const TRANSACTION_PATH: &str = "/transaction";
const MAX_ERROR_BODY_DRAIN_BYTES: usize = 4096;
const MAX_TRANSACTION_ID_LEN: usize = 128;
const MAX_ERROR_TOKEN_LEN: usize = 96;
const MAX_POLL_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Clone, PartialEq, Eq)]
pub struct DepositWalletRelayerUrl {
    base: Url,
}

impl DepositWalletRelayerUrl {
    pub fn parse(raw: &str) -> Result<Self> {
        let url = Url::parse(raw)
            .map_err(|e| RelayerError::InvalidRelayerUrl(format!("could not parse URL: {e}")))?;
        validate_rel_url(&url)?;
        Ok(Self { base: url })
    }

    fn endpoint(&self, path: &str) -> Url {
        let mut url = self.base.clone();
        url.set_path(path);
        url.set_query(None);
        url
    }

    #[cfg(test)]
    fn loopback(raw: &str) -> Result<Self> {
        let url = Url::parse(raw)
            .map_err(|e| RelayerError::InvalidRelayerUrl(format!("could not parse URL: {e}")))?;
        Ok(Self { base: url })
    }
}

impl fmt::Debug for DepositWalletRelayerUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("DepositWalletRelayerUrl")
            .field(&self.base.as_str())
            .finish()
    }
}

#[derive(Clone)]
pub struct RelayerKeyAuth {
    api_key: String,
    api_key_address: Address,
}

impl RelayerKeyAuth {
    pub fn new(api_key: impl Into<String>, api_key_address: Address) -> Self {
        Self {
            api_key: api_key.into(),
            api_key_address,
        }
    }

    pub fn api_key_address(&self) -> Address {
        self.api_key_address
    }

    fn headers(&self) -> Result<HeaderMap> {
        let mut headers = HeaderMap::new();
        headers.insert(
            "RELAYER_API_KEY",
            HeaderValue::from_str(&self.api_key)
                .map_err(|_| RelayerError::AuthError("invalid relayer API key".to_string()))?,
        );
        headers.insert(
            "RELAYER_API_KEY_ADDRESS",
            HeaderValue::from_str(&to_checksum(&self.api_key_address, None)).map_err(|_| {
                RelayerError::AuthError("invalid relayer API key address".to_string())
            })?,
        );
        Ok(headers)
    }
}

impl fmt::Debug for RelayerKeyAuth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RelayerKeyAuth")
            .field("api_key", &"<redacted>")
            .field(
                "api_key_address",
                &redacted_address(self.api_key_address),
            )
            .finish()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum DepositWalletMutationGate {
    #[default]
    Deny,
    Permit(DepositWalletMutationPermit),
}

#[derive(Clone, PartialEq, Eq)]
pub struct DepositWalletMutationPermit {
    reason: String,
}

impl DepositWalletMutationPermit {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

impl fmt::Debug for DepositWalletMutationPermit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DepositWalletMutationPermit")
            .field("reason", &self.reason)
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DepositWalletPollPolicy {
    pub max_attempts: usize,
    pub interval: Duration,
}

impl DepositWalletPollPolicy {
    pub fn new(max_attempts: usize, interval: Duration) -> Result<Self> {
        if max_attempts == 0 {
            return Err(RelayerError::Other(
                "deposit wallet poll policy max attempts must be greater than zero".to_string(),
            ));
        }

        Ok(Self {
            max_attempts,
            interval,
        })
    }

    fn interval_for_attempt(&self, attempt: usize) -> Duration {
        let multiplier = 1u32 << attempt.min(4);
        self.interval
            .saturating_mul(multiplier)
            .min(MAX_POLL_INTERVAL)
    }
}

impl Default for DepositWalletPollPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 60,
            interval: Duration::from_secs(2),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DepositWalletTransactionReceipt {
    pub transaction_id: String,
    pub state: RelayerTransactionState,
    pub transaction_hash: Option<String>,
}

#[derive(Clone)]
pub struct DepositWalletRelayerClient {
    http: Client,
    base_url: DepositWalletRelayerUrl,
    auth: RelayerKeyAuth,
    config: DepositWalletContractConfig,
    mutation_state: Arc<Mutex<OwnerMutationState>>,
    clock: Arc<dyn DepositWalletClock>,
    sleeper: Arc<dyn DepositWalletSleeper>,
}

impl DepositWalletRelayerClient {
    pub fn new(
        base_url: DepositWalletRelayerUrl,
        auth: RelayerKeyAuth,
        config: DepositWalletContractConfig,
    ) -> Result<Self> {
        let http = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(30))
            .build()?;

        Ok(Self::from_parts(
            http,
            base_url,
            auth,
            config,
            Arc::new(SystemClock),
            Arc::new(TokioSleeper),
        ))
    }

    fn from_parts(
        http: Client,
        base_url: DepositWalletRelayerUrl,
        auth: RelayerKeyAuth,
        config: DepositWalletContractConfig,
        clock: Arc<dyn DepositWalletClock>,
        sleeper: Arc<dyn DepositWalletSleeper>,
    ) -> Self {
        Self {
            http,
            base_url,
            auth,
            config,
            mutation_state: Arc::new(Mutex::new(OwnerMutationState::default())),
            clock,
            sleeper,
        }
    }

    pub async fn get_wallet_nonce(&self, owner: Address) -> Result<U256> {
        self.ensure_owner_unblocked(owner)?;
        let request = build_wallet_nonce_request(owner);
        let mut url = self.base_url.endpoint(request.path());
        url.query_pairs_mut()
            .append_pair("address", &to_checksum(&owner, None))
            .append_pair("type", request.nonce_type());
        let response = self.send(Method::GET, url, None).await?;
        let nonce = serde_json::from_slice::<WalletNonceResponse>(&response)
            .map_err(|e| RelayerError::Other(format!("could not parse WALLET nonce: {e}")))?;
        U256::from_dec_str(&nonce.nonce)
            .map_err(|e| RelayerError::Other(format!("invalid WALLET nonce: {e}")))
    }

    pub async fn submit_wallet_create(
        &self,
        owner: Address,
        gate: DepositWalletMutationGate,
    ) -> Result<DepositWalletTransactionReceipt> {
        ensure_permitted(&gate)?;
        self.ensure_owner_unblocked(owner)?;
        let request = build_wallet_create_request(owner, self.config);
        let body = serde_json::to_string(&request)
            .map_err(|e| RelayerError::Other(format!("could not serialize WALLET-CREATE: {e}")))?;
        self.submit_owner_body(owner, body).await
    }

    pub async fn submit_signed_wallet_batch(
        &self,
        signed: SignedDepositWalletBatch,
        gate: DepositWalletMutationGate,
    ) -> Result<DepositWalletTransactionReceipt> {
        ensure_permitted(&gate)?;
        let owner = signed.owner();
        self.ensure_owner_unblocked(owner)?;
        self.ensure_deadline_fresh(&signed)?;
        let nonce = self.get_wallet_nonce(owner).await?;
        if nonce != signed.nonce() {
            return Err(RelayerError::Signing(
                "signed deposit wallet batch nonce does not match current WALLET nonce"
                    .to_string(),
            ));
        }
        let request = build_deposit_wallet_batch_request_from_signed(signed, self.config)?;
        let body = serde_json::to_string(&request)
            .map_err(|e| RelayerError::Other(format!("could not serialize WALLET batch: {e}")))?;
        self.submit_owner_body(owner, body).await
    }

    pub async fn get_transaction(
        &self,
        transaction_id: &str,
    ) -> Result<DepositWalletTransactionReceipt> {
        if transaction_id.trim().is_empty() {
            return Err(RelayerError::Other(
                "transaction id must not be empty".to_string(),
            ));
        }

        let transaction_id = validate_transaction_id(transaction_id)?;
        let mut url = self.base_url.endpoint(TRANSACTION_PATH);
        url.query_pairs_mut().append_pair("id", &transaction_id);
        let response = self.send(Method::GET, url, None).await?;
        parse_transaction_response(&transaction_id, &response)
    }

    pub async fn poll_transaction(
        &self,
        transaction_id: &str,
        policy: DepositWalletPollPolicy,
    ) -> Result<DepositWalletTransactionReceipt> {
        let transaction_id = validate_transaction_id(transaction_id)?;
        let transaction_id_for_error = sanitized_external_token(&transaction_id);

        for attempt in 0..policy.max_attempts {
            let receipt = self.get_transaction(&transaction_id).await?;
            match &receipt.state {
                RelayerTransactionState::Confirmed => {
                    self.clear_transaction_block(&transaction_id)?;
                    return Ok(receipt);
                }
                RelayerTransactionState::Invalid => {
                    self.clear_transaction_block(&transaction_id)?;
                    return Err(RelayerError::TransactionInvalid(format!(
                        "deposit wallet transaction {} invalid",
                        transaction_id_for_error
                    )));
                }
                RelayerTransactionState::Failed => {
                    self.clear_transaction_block(&transaction_id)?;
                    return Err(RelayerError::TransactionFailed(format!(
                        "deposit wallet transaction {} failed",
                        transaction_id_for_error
                    )));
                }
                RelayerTransactionState::Unknown(raw) => {
                    self.mark_transaction_reconciliation_required(&transaction_id)?;
                    return Err(RelayerError::ReconciliationRequired(format!(
                        "deposit wallet transaction {} reached unknown state {}",
                        transaction_id_for_error,
                        sanitized_external_token(raw)
                    )));
                }
                RelayerTransactionState::New
                | RelayerTransactionState::Executed
                | RelayerTransactionState::Mined => {}
            }

            if attempt + 1 < policy.max_attempts {
                self.sleeper.sleep(policy.interval_for_attempt(attempt)).await;
            }
        }

        self.mark_transaction_reconciliation_required(&transaction_id)?;
        Err(RelayerError::Timeout)
    }

    pub fn clear_ambiguous_submit_after_manual_reconciliation(
        &self,
        owner: Address,
        permit: DepositWalletMutationPermit,
    ) -> Result<()> {
        if permit.reason.trim().is_empty() {
            return Err(RelayerError::MutationBlocked(
                "manual reconciliation reason required before clearing ambiguous submit"
                    .to_string(),
            ));
        }

        let mut state = self.mutation_state()?;
        state.owner_blocks.remove(&owner);
        state
            .transaction_owners
            .retain(|_, record| record.owner != owner);
        Ok(())
    }

    pub fn ambiguous_submit_block(&self, owner: Address) -> Option<String> {
        let state = self.mutation_state.lock().ok()?;
        match state.owner_blocks.get(&owner) {
            Some(OwnerMutationBlock::Ambiguous { payload_hash }) => Some(payload_hash.clone()),
            _ => None,
        }
    }

    async fn submit_owner_body(
        &self,
        owner: Address,
        body: String,
    ) -> Result<DepositWalletTransactionReceipt> {
        let payload_hash = payload_hash_summary(body.as_bytes());
        self.reserve_owner_submit(owner, payload_hash.clone())?;
        let url = self.base_url.endpoint(SUBMIT_PATH);
        match self.send(Method::POST, url, Some(body.clone())).await {
            Ok(response) => match parse_submit_response(&response) {
                Ok(receipt) => self.handle_submit_receipt(owner, payload_hash, receipt),
                Err(error) => {
                    self.record_ambiguous(owner, payload_hash.clone())?;
                    Err(RelayerError::AmbiguousSubmit(format!(
                        "submit response did not include a usable transactionID for owner {} payload {}: {}",
                        redacted_address(owner),
                        payload_hash,
                        error
                    )))
                }
            },
            Err(RelayerError::Http(error)) => {
                self.record_ambiguous(owner, payload_hash.clone())?;
                Err(RelayerError::AmbiguousSubmit(format!(
                    "submit transport failed for owner {} payload {}: {}",
                    redacted_address(owner),
                    payload_hash,
                    sanitized_external_token(&error.to_string())
                )))
            }
            Err(RelayerError::Api { status, message: _ })
                if is_ambiguous_submit_status(status) =>
            {
                self.record_ambiguous(owner, payload_hash.clone())?;
                Err(RelayerError::AmbiguousSubmit(format!(
                    "submit returned timeout status {} for owner {} payload {}",
                    status,
                    redacted_address(owner),
                    payload_hash
                )))
            }
            Err(error) => {
                self.clear_owner_block_if_payload(owner, &payload_hash)?;
                Err(error)
            }
        }
    }

    async fn send(&self, method: Method, url: Url, body: Option<String>) -> Result<Vec<u8>> {
        let mut headers = self.auth.headers()?;
        if body.is_some() {
            headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        }

        let mut request = self
            .http
            .request(method.clone(), url)
            .headers(headers);
        if let Some(body) = body {
            request = request.body(body);
        }

        let response = request.send().await?;
        if !response.status().is_success() {
            let status = response.status();
            drain_limited_error_body(response).await?;
            if status == StatusCode::TOO_MANY_REQUESTS {
                return Err(RelayerError::QuotaExhausted);
            }
            return Err(RelayerError::Api {
                status: status.as_u16(),
                message: format!("deposit-wallet relayer request failed with HTTP {status}"),
            });
        }

        Ok(response.bytes().await?.to_vec())
    }

    fn ensure_deadline_fresh(&self, signed: &SignedDepositWalletBatch) -> Result<()> {
        let now = U256::from(self.clock.now_unix_seconds());
        if signed.deadline() <= now {
            return Err(RelayerError::Signing(
                "signed deposit wallet batch deadline is expired".to_string(),
            ));
        }
        Ok(())
    }

    fn ensure_owner_unblocked(&self, owner: Address) -> Result<()> {
        let state = self.mutation_state()?;
        if let Some(block) = state.owner_blocks.get(&owner) {
            return Err(owner_block_error(owner, block));
        }
        Ok(())
    }

    fn reserve_owner_submit(&self, owner: Address, payload_hash: String) -> Result<()> {
        let mut state = self.mutation_state()?;
        if let Some(block) = state.owner_blocks.get(&owner) {
            return Err(owner_block_error(owner, block));
        }

        state.owner_blocks.insert(
            owner,
            OwnerMutationBlock::InFlight {
                payload_hash,
                transaction_id: None,
            },
        );
        Ok(())
    }

    fn handle_submit_receipt(
        &self,
        owner: Address,
        payload_hash: String,
        receipt: DepositWalletTransactionReceipt,
    ) -> Result<DepositWalletTransactionReceipt> {
        match &receipt.state {
            RelayerTransactionState::Unknown(raw) => {
                self.record_ambiguous(owner, payload_hash.clone())?;
                self.record_transaction_owner(&receipt.transaction_id, owner, payload_hash)?;
                Err(RelayerError::ReconciliationRequired(format!(
                    "submit response for owner {} reached unknown state {}; manual reconciliation required",
                    redacted_address(owner),
                    sanitized_external_token(raw)
                )))
            }
            RelayerTransactionState::Invalid => {
                self.clear_owner_block_if_payload(owner, &payload_hash)?;
                Err(RelayerError::TransactionInvalid(format!(
                    "deposit wallet submit transaction {} invalid",
                    sanitized_external_token(&receipt.transaction_id)
                )))
            }
            RelayerTransactionState::Failed => {
                self.clear_owner_block_if_payload(owner, &payload_hash)?;
                Err(RelayerError::TransactionFailed(format!(
                    "deposit wallet submit transaction {} failed",
                    sanitized_external_token(&receipt.transaction_id)
                )))
            }
            RelayerTransactionState::Confirmed => {
                self.clear_owner_block_if_payload(owner, &payload_hash)?;
                Ok(receipt)
            }
            RelayerTransactionState::New
            | RelayerTransactionState::Executed
            | RelayerTransactionState::Mined => {
                self.record_inflight_transaction(
                    owner,
                    payload_hash,
                    receipt.transaction_id.clone(),
                )?;
                Ok(receipt)
            }
        }
    }

    fn record_ambiguous(&self, owner: Address, payload_hash: String) -> Result<()> {
        let mut state = self.mutation_state()?;
        state
            .owner_blocks
            .insert(owner, OwnerMutationBlock::Ambiguous { payload_hash });
        Ok(())
    }

    fn record_inflight_transaction(
        &self,
        owner: Address,
        payload_hash: String,
        transaction_id: String,
    ) -> Result<()> {
        let mut state = self.mutation_state()?;
        state.owner_blocks.insert(
            owner,
            OwnerMutationBlock::InFlight {
                payload_hash: payload_hash.clone(),
                transaction_id: Some(transaction_id.clone()),
            },
        );
        state.transaction_owners.insert(
            transaction_id,
            OwnerTransactionRecord {
                owner,
                payload_hash,
            },
        );
        Ok(())
    }

    fn record_transaction_owner(
        &self,
        transaction_id: &str,
        owner: Address,
        payload_hash: String,
    ) -> Result<()> {
        let mut state = self.mutation_state()?;
        state.transaction_owners.insert(
            transaction_id.to_string(),
            OwnerTransactionRecord {
                owner,
                payload_hash,
            },
        );
        Ok(())
    }

    fn clear_transaction_block(&self, transaction_id: &str) -> Result<()> {
        let mut state = self.mutation_state()?;
        if let Some(record) = state.transaction_owners.remove(transaction_id) {
            clear_owner_block_if_payload(&mut state, record.owner, &record.payload_hash);
        }
        Ok(())
    }

    fn mark_transaction_reconciliation_required(&self, transaction_id: &str) -> Result<()> {
        let mut state = self.mutation_state()?;
        if let Some(record) = state.transaction_owners.get(transaction_id).cloned() {
            state.owner_blocks.insert(
                record.owner,
                OwnerMutationBlock::Ambiguous {
                    payload_hash: record.payload_hash,
                },
            );
        }
        Ok(())
    }

    fn clear_owner_block_if_payload(&self, owner: Address, payload_hash: &str) -> Result<()> {
        let mut state = self.mutation_state()?;
        clear_owner_block_if_payload(&mut state, owner, payload_hash);
        Ok(())
    }

    fn mutation_state(&self) -> Result<MutexGuard<'_, OwnerMutationState>> {
        self.mutation_state.lock().map_err(|_| {
            RelayerError::ReconciliationRequired(
                "deposit wallet owner mutation state lock is poisoned; manual reconciliation required"
                    .to_string(),
            )
        })
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

#[derive(Clone, Debug, Default)]
struct OwnerMutationState {
    owner_blocks: HashMap<Address, OwnerMutationBlock>,
    transaction_owners: HashMap<String, OwnerTransactionRecord>,
}

#[derive(Clone, Debug)]
struct OwnerTransactionRecord {
    owner: Address,
    payload_hash: String,
}

#[derive(Clone, Debug)]
enum OwnerMutationBlock {
    InFlight {
        payload_hash: String,
        transaction_id: Option<String>,
    },
    Ambiguous {
        payload_hash: String,
    },
}

impl OwnerMutationBlock {
    fn payload_hash(&self) -> &str {
        match self {
            Self::InFlight { payload_hash, .. } | Self::Ambiguous { payload_hash } => {
                payload_hash
            }
        }
    }
}

#[derive(Deserialize)]
struct WalletNonceResponse {
    nonce: String,
}

trait DepositWalletClock: Send + Sync {
    fn now_unix_seconds(&self) -> u64;
}

struct SystemClock;

impl DepositWalletClock for SystemClock {
    fn now_unix_seconds(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }
}

trait DepositWalletSleeper: Send + Sync {
    fn sleep<'a>(&'a self, duration: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>;
}

struct TokioSleeper;

impl DepositWalletSleeper for TokioSleeper {
    fn sleep<'a>(&'a self, duration: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(tokio::time::sleep(duration))
    }
}

fn validate_rel_url(url: &Url) -> Result<()> {
    if url.scheme() != "https" {
        return Err(RelayerError::InvalidRelayerUrl(
            "relayer URL must use https".to_string(),
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(RelayerError::InvalidRelayerUrl(
            "relayer URL must not include userinfo".to_string(),
        ));
    }
    if url.host_str() != Some(RELAYER_HOST) {
        return Err(RelayerError::InvalidRelayerUrl(
            "relayer URL host is not allowlisted".to_string(),
        ));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(RelayerError::InvalidRelayerUrl(
            "relayer URL must not include query or fragment".to_string(),
        ));
    }
    Ok(())
}

fn ensure_permitted(gate: &DepositWalletMutationGate) -> Result<()> {
    match gate {
        DepositWalletMutationGate::Permit(permit) if !permit.reason.trim().is_empty() => Ok(()),
        DepositWalletMutationGate::Permit(_) => Err(RelayerError::MutationBlocked(
            "explicit deposit-wallet mutation permit reason required".to_string(),
        )),
        DepositWalletMutationGate::Deny => Err(RelayerError::MutationBlocked(
            "explicit deposit-wallet mutation permit required".to_string(),
        )),
    }
}

async fn drain_limited_error_body(mut response: reqwest::Response) -> Result<()> {
    let mut drained = 0usize;
    while drained < MAX_ERROR_BODY_DRAIN_BYTES {
        let Some(chunk) = response.chunk().await? else {
            return Ok(());
        };
        drained = drained.saturating_add(chunk.len());
    }
    Ok(())
}

fn is_ambiguous_submit_status(status: u16) -> bool {
    status == StatusCode::REQUEST_TIMEOUT.as_u16()
        || status == StatusCode::GATEWAY_TIMEOUT.as_u16()
}

fn parse_submit_response(bytes: &[u8]) -> Result<DepositWalletTransactionReceipt> {
    let response = serde_json::from_slice::<RelayerSubmitResponse>(bytes)
        .map_err(|e| RelayerError::Other(format!("could not parse submit response: {e}")))?;
    receipt_from_submit_response(response)
}

fn parse_transaction_response(
    expected_transaction_id: &str,
    bytes: &[u8],
) -> Result<DepositWalletTransactionReceipt> {
    if let Ok(response) = serde_json::from_slice::<RelayerSubmitResponse>(bytes) {
        let receipt = receipt_from_submit_response(response)?;
        return require_transaction_id_match(expected_transaction_id, receipt);
    }

    let responses = serde_json::from_slice::<Vec<RelayerSubmitResponse>>(bytes)
        .map_err(|e| RelayerError::Other(format!("could not parse transaction response: {e}")))?;
    let response = responses
        .into_iter()
        .find(|response| response.transaction_id == expected_transaction_id)
        .ok_or_else(|| {
            RelayerError::ReconciliationRequired(format!(
                "transaction response did not include requested transaction id {}",
                sanitized_external_token(expected_transaction_id)
            ))
        })?;
    let receipt = receipt_from_submit_response(response)?;
    require_transaction_id_match(expected_transaction_id, receipt)
}

fn receipt_from_submit_response(
    response: RelayerSubmitResponse,
) -> Result<DepositWalletTransactionReceipt> {
    if response.transaction_id.trim().is_empty() {
        return Err(RelayerError::Other(
            "relayer response transactionID must not be empty".to_string(),
        ));
    }
    let transaction_id = validate_transaction_id(&response.transaction_id).map_err(|_| {
        RelayerError::Other("relayer response transactionID was invalid".to_string())
    })?;

    Ok(DepositWalletTransactionReceipt {
        transaction_id,
        state: response.state,
        transaction_hash: response.transaction_hash,
    })
}

fn require_transaction_id_match(
    expected_transaction_id: &str,
    receipt: DepositWalletTransactionReceipt,
) -> Result<DepositWalletTransactionReceipt> {
    if receipt.transaction_id != expected_transaction_id {
        return Err(RelayerError::ReconciliationRequired(format!(
            "transaction response id {} did not match requested id {}",
            sanitized_external_token(&receipt.transaction_id),
            sanitized_external_token(expected_transaction_id)
        )));
    }
    Ok(receipt)
}

fn validate_transaction_id(transaction_id: &str) -> Result<String> {
    let trimmed = transaction_id.trim();
    if trimmed.is_empty()
        || trimmed.len() > MAX_TRANSACTION_ID_LEN
        || !trimmed
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(RelayerError::Other(
            "transaction id must be 1-128 ASCII alphanumeric, hyphen, underscore, or period characters"
                .to_string(),
        ));
    }

    Ok(trimmed.to_string())
}

fn sanitized_external_token(value: &str) -> String {
    let mut sanitized = String::new();
    for ch in value.chars().take(MAX_ERROR_TOKEN_LEN) {
        if ch.is_ascii_graphic() || ch == ' ' {
            sanitized.push(ch);
        } else {
            sanitized.push('?');
        }
    }
    if value.chars().count() > MAX_ERROR_TOKEN_LEN {
        sanitized.push_str("...");
    }
    sanitized
}

fn payload_hash_summary(bytes: &[u8]) -> String {
    let hex = hex::encode(keccak256(bytes));
    format!("0x{}...{}", &hex[..8], &hex[56..])
}

fn redacted_address(address: Address) -> String {
    let checksum = to_checksum(&address, None);
    format!("{}...{}", &checksum[..6], &checksum[38..])
}

fn owner_block_error(owner: Address, block: &OwnerMutationBlock) -> RelayerError {
    match block {
        OwnerMutationBlock::InFlight {
            payload_hash,
            transaction_id: Some(transaction_id),
        } => RelayerError::ReconciliationRequired(format!(
            "owner {} has in-flight submit transaction {} payload {}; poll to terminal state or reconcile manually",
            redacted_address(owner),
            sanitized_external_token(transaction_id),
            payload_hash
        )),
        OwnerMutationBlock::InFlight {
            payload_hash,
            transaction_id: None,
        } => RelayerError::ReconciliationRequired(format!(
            "owner {} has in-flight submit payload {}; wait for the submit response or reconcile manually",
            redacted_address(owner),
            payload_hash
        )),
        OwnerMutationBlock::Ambiguous { payload_hash } => RelayerError::ReconciliationRequired(
            format!(
                "owner {} has ambiguous submit payload {}; manual reconciliation required",
                redacted_address(owner),
                payload_hash
            ),
        ),
    }
}

fn clear_owner_block_if_payload(
    state: &mut OwnerMutationState,
    owner: Address,
    payload_hash: &str,
) {
    if state
        .owner_blocks
        .get(&owner)
        .is_some_and(|block| block.payload_hash() == payload_hash)
    {
        state.owner_blocks.remove(&owner);
    }
    state
        .transaction_owners
        .retain(|_, record| record.owner != owner || record.payload_hash != payload_hash);
}

#[cfg(test)]
mod tests {
    use std::io::ErrorKind;
    use std::sync::Arc;

    use ethers::types::Bytes;
    use serde_json::{json, Value};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::oneshot;
    use tokio::task::JoinHandle;

    use crate::auth::{AuthMethod, BuilderConfig};
    use crate::deposit_wallet::{
        deposit_wallet_contract_config, validate_deposit_wallet_batch_signature,
        DepositWalletBatchToSign, DepositWalletCall,
    };

    use super::*;

    const API_KEY: &str = "unit-test-relayer-api-key";
    const WALLET_CREATE_OWNER: &str = "0x6e0c80c90ea6c15917308F820Eac91Ce2724B5b5";
    const API_KEY_ADDRESS: &str = "0xA6Db23622C9EA7584D5c61C3e7497c80E2CE167B";

    #[derive(Clone)]
    struct FixedClock {
        now: u64,
    }

    impl DepositWalletClock for FixedClock {
        fn now_unix_seconds(&self) -> u64 {
            self.now
        }
    }

    #[derive(Default)]
    struct RecordingSleeper {
        sleeps: Mutex<Vec<Duration>>,
    }

    impl RecordingSleeper {
        fn sleeps(&self) -> Vec<Duration> {
            self.sleeps.lock().unwrap().clone()
        }
    }

    impl DepositWalletSleeper for RecordingSleeper {
        fn sleep<'a>(
            &'a self,
            duration: Duration,
        ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
            self.sleeps.lock().unwrap().push(duration);
            Box::pin(async {})
        }
    }

    #[derive(Debug)]
    struct CapturedRequest {
        method: String,
        path: String,
        headers: Vec<(String, String)>,
        body: String,
    }

    impl CapturedRequest {
        fn header(&self, name: &str) -> Option<&str> {
            self.headers
                .iter()
                .find(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
                .map(|(_, value)| value.as_str())
        }
    }

    #[derive(Clone)]
    struct TestResponse {
        status: &'static str,
        headers: Vec<(String, String)>,
        body: String,
    }

    impl TestResponse {
        fn json(status: &'static str, body: impl Into<String>) -> Self {
            Self {
                status,
                headers: vec![("content-type".to_string(), "application/json".to_string())],
                body: body.into(),
            }
        }

        fn redirect(location: String) -> Self {
            Self {
                status: "307 Temporary Redirect",
                headers: vec![("location".to_string(), location)],
                body: String::new(),
            }
        }
    }

    fn address(raw: &str) -> Address {
        raw.parse().expect("test address should parse")
    }

    fn fixture_text(name: &str) -> String {
        let path = format!(
            "{}/tests/fixtures/deposit_wallet/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        std::fs::read_to_string(path).expect("fixture should be readable")
    }

    fn fixture_value(name: &str) -> Value {
        serde_json::from_str(&fixture_text(name)).expect("fixture should be valid JSON")
    }

    fn relayer_auth() -> RelayerKeyAuth {
        RelayerKeyAuth::new(API_KEY, address(API_KEY_ADDRESS))
    }

    fn mutation_permit() -> DepositWalletMutationGate {
        DepositWalletMutationGate::Permit(DepositWalletMutationPermit::new(
            "mocked unit-test relayer call",
        ))
    }

    fn reqwest_client(timeout: Duration) -> Client {
        Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(timeout)
            .build()
            .expect("test HTTP client should build")
    }

    fn test_client(base_url: DepositWalletRelayerUrl) -> DepositWalletRelayerClient {
        test_client_with_auth_clock_timeout(
            base_url,
            relayer_auth(),
            1_700_000_000,
            Duration::from_secs(2),
        )
    }

    fn test_client_with_auth_clock_timeout(
        base_url: DepositWalletRelayerUrl,
        auth: RelayerKeyAuth,
        now: u64,
        timeout: Duration,
    ) -> DepositWalletRelayerClient {
        let clock: Arc<dyn DepositWalletClock> = Arc::new(FixedClock { now });
        let sleeper: Arc<dyn DepositWalletSleeper> = Arc::new(RecordingSleeper::default());
        DepositWalletRelayerClient::from_parts(
            reqwest_client(timeout),
            base_url,
            auth,
            deposit_wallet_contract_config(137).unwrap(),
            clock,
            sleeper,
        )
    }

    fn test_client_with_sleeper(
        base_url: DepositWalletRelayerUrl,
        sleeper: Arc<RecordingSleeper>,
    ) -> DepositWalletRelayerClient {
        let clock: Arc<dyn DepositWalletClock> = Arc::new(FixedClock { now: 1_700_000_000 });
        let sleeper_trait: Arc<dyn DepositWalletSleeper> = sleeper;
        DepositWalletRelayerClient::from_parts(
            reqwest_client(Duration::from_secs(2)),
            base_url,
            relayer_auth(),
            deposit_wallet_contract_config(137).unwrap(),
            clock,
            sleeper_trait,
        )
    }

    fn signed_wallet_batch() -> SignedDepositWalletBatch {
        let fixture = fixture_value("wallet_batch_eip712.json");
        let calls = fixture["calls"]
            .as_array()
            .unwrap()
            .iter()
            .map(|call| DepositWalletCall {
                target: call["target"].as_str().unwrap().parse().unwrap(),
                value: U256::from_dec_str(call["value"].as_str().unwrap()).unwrap(),
                data: Bytes::from(
                    hex::decode(call["data"].as_str().unwrap().trim_start_matches("0x"))
                        .unwrap(),
                ),
            })
            .collect();
        let batch = DepositWalletBatchToSign {
            owner: fixture["owner"].as_str().unwrap().parse().unwrap(),
            nonce_owner: fixture["nonceOwner"].as_str().unwrap().parse().unwrap(),
            submit_from: fixture["submitFrom"].as_str().unwrap().parse().unwrap(),
            deposit_wallet: fixture["depositWallet"].as_str().unwrap().parse().unwrap(),
            chain_id: fixture["chainId"].as_u64().unwrap(),
            nonce: U256::from_dec_str(fixture["nonce"].as_str().unwrap()).unwrap(),
            deadline: U256::from_dec_str(fixture["deadline"].as_str().unwrap()).unwrap(),
            calls,
        };
        validate_deposit_wallet_batch_signature(
            batch,
            fixture["ownerSignature"].as_str().unwrap(),
        )
        .unwrap()
    }

    async fn spawn_server(
        responses: Vec<TestResponse>,
    ) -> (DepositWalletRelayerUrl, JoinHandle<Vec<CapturedRequest>>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test server should bind");
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let mut requests = Vec::with_capacity(responses.len());
            for response in responses {
                let (mut stream, _) = listener.accept().await.expect("server should accept");
                let request = read_request(&mut stream).await;
                write_response(&mut stream, response).await;
                requests.push(request);
            }
            requests
        });

        (
            DepositWalletRelayerUrl::loopback(&format!("http://{addr}")).unwrap(),
            handle,
        )
    }

    async fn spawn_hanging_server() -> (
        DepositWalletRelayerUrl,
        oneshot::Receiver<CapturedRequest>,
        JoinHandle<()>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test server should bind");
        let addr = listener.local_addr().unwrap();
        let (sender, receiver) = oneshot::channel();
        let handle = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("server should accept");
            let request = read_request(&mut stream).await;
            let _ = sender.send(request);
            std::future::pending::<()>().await;
        });

        (
            DepositWalletRelayerUrl::loopback(&format!("http://{addr}")).unwrap(),
            receiver,
            handle,
        )
    }

    async fn spawn_reset_server() -> (DepositWalletRelayerUrl, JoinHandle<Vec<CapturedRequest>>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test server should bind");
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("server should accept");
            let request = read_request(&mut stream).await;
            drop(stream);
            vec![request]
        });

        (
            DepositWalletRelayerUrl::loopback(&format!("http://{addr}")).unwrap(),
            handle,
        )
    }

    async fn read_request(stream: &mut TcpStream) -> CapturedRequest {
        let mut buffer = Vec::new();
        let headers_end = loop {
            let mut chunk = [0u8; 1024];
            let read = stream.read(&mut chunk).await.expect("request should read");
            assert!(read > 0, "request ended before headers completed");
            buffer.extend_from_slice(&chunk[..read]);
            if let Some(index) = find_headers_end(&buffer) {
                break index;
            }
        };

        let body_start = headers_end + 4;
        let header_text = String::from_utf8(buffer[..headers_end].to_vec()).unwrap();
        let mut lines = header_text.split("\r\n");
        let request_line = lines.next().unwrap();
        let mut request_parts = request_line.split_whitespace();
        let method = request_parts.next().unwrap().to_string();
        let path = request_parts.next().unwrap().to_string();
        let headers = lines
            .filter_map(|line| {
                let (name, value) = line.split_once(':')?;
                Some((name.trim().to_string(), value.trim().to_string()))
            })
            .collect::<Vec<_>>();
        let content_length = headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
            .and_then(|(_, value)| value.parse::<usize>().ok())
            .unwrap_or(0);

        while buffer.len() < body_start + content_length {
            let mut chunk = [0u8; 1024];
            let read = stream.read(&mut chunk).await.expect("body should read");
            assert!(read > 0, "request ended before body completed");
            buffer.extend_from_slice(&chunk[..read]);
        }

        let body =
            String::from_utf8(buffer[body_start..body_start + content_length].to_vec()).unwrap();

        CapturedRequest {
            method,
            path,
            headers,
            body,
        }
    }

    fn find_headers_end(buffer: &[u8]) -> Option<usize> {
        buffer
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
    }

    async fn write_response(stream: &mut TcpStream, response: TestResponse) {
        let mut wire = format!(
            "HTTP/1.1 {}\r\ncontent-length: {}\r\nconnection: close\r\n",
            response.status,
            response.body.len()
        );
        for (name, value) in response.headers {
            wire.push_str(&format!("{name}: {value}\r\n"));
        }
        wire.push_str("\r\n");
        wire.push_str(&response.body);
        stream
            .write_all(wire.as_bytes())
            .await
            .expect("response should write");
    }

    fn transaction_response(transaction_id: &str, state: &str) -> String {
        json!({
            "transactionID": transaction_id,
            "state": state,
            "transactionHash": "0x38cbfbeae8fffa4e2b187ee5978d3ee9cafc53af0363ed90a35b7ea9016535d8"
        })
        .to_string()
    }

    async fn poll_sequence(
        states: &[&str],
        max_attempts: usize,
    ) -> (
        Result<DepositWalletTransactionReceipt>,
        Vec<CapturedRequest>,
        Arc<RecordingSleeper>,
    ) {
        let responses = states
            .iter()
            .map(|state| TestResponse::json("200 OK", transaction_response("tx-123", state)))
            .collect::<Vec<_>>();
        let (url, handle) = spawn_server(responses).await;
        let sleeper = Arc::new(RecordingSleeper::default());
        let client = test_client_with_sleeper(url, sleeper.clone());
        let policy = DepositWalletPollPolicy::new(max_attempts, Duration::from_millis(5)).unwrap();
        let result = client.poll_transaction("tx-123", policy).await;
        let requests = handle.await.unwrap();
        (result, requests, sleeper)
    }

    #[test]
    fn production_url_rejects_unsafe_endpoints() {
        let invalid = [
            "http://relayer-v2.polymarket.com",
            "https://user@relayer-v2.polymarket.com",
            "https://relayer-v2.polymarket.com?api_key=leak",
            "https://relayer-v2.polymarket.com#fragment",
            "https://example.com",
            "https://relayer-v2.polymarket.com.evil.example",
        ];

        for raw in invalid {
            assert!(
                matches!(
                    DepositWalletRelayerUrl::parse(raw),
                    Err(RelayerError::InvalidRelayerUrl(_))
                ),
                "{raw} should be rejected"
            );
        }

        DepositWalletRelayerUrl::parse("https://relayer-v2.polymarket.com").unwrap();
    }

    #[test]
    fn auth_debug_redacts_secret_bearing_fields() {
        let auth = relayer_auth();
        let rendered = format!("{auth:?}");
        assert!(!rendered.contains(API_KEY));
        assert!(!rendered.contains(&to_checksum(&address(API_KEY_ADDRESS), None)));

        let builder = BuilderConfig {
            key: "builder-key-secret".to_string(),
            secret: "builder-hmac-secret".to_string(),
            passphrase: "builder-passphrase-secret".to_string(),
        };
        let rendered = format!("{:?}", AuthMethod::Builder(builder));
        assert!(!rendered.contains("builder-key-secret"));
        assert!(!rendered.contains("builder-hmac-secret"));
        assert!(!rendered.contains("builder-passphrase-secret"));

        let rendered = format!(
            "{:?}",
            AuthMethod::relayer_key(API_KEY, &to_checksum(&address(API_KEY_ADDRESS), None))
        );
        assert!(!rendered.contains(API_KEY));
        assert!(!rendered.contains(&to_checksum(&address(API_KEY_ADDRESS), None)));
    }

    #[tokio::test]
    async fn api_error_messages_do_not_echo_secret_material() {
        let signed = signed_wallet_batch();
        let body = format!(
            "bad request echoed {} {} Authorization: bearer value",
            API_KEY,
            fixture_value("wallet_batch_eip712.json")["ownerSignature"]
                .as_str()
                .unwrap()
        );
        let (url, handle) =
            spawn_server(vec![TestResponse::json("400 Bad Request", body)]).await;
        let client = test_client(url);

        let error = client.get_wallet_nonce(signed.owner()).await.unwrap_err();
        let rendered = error.to_string();
        assert!(!rendered.contains(API_KEY));
        assert!(!rendered.contains(
            fixture_value("wallet_batch_eip712.json")["ownerSignature"]
                .as_str()
                .unwrap()
        ));
        assert!(!rendered.contains("bearer value"));

        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
    }

    #[tokio::test]
    async fn default_deny_gate_runs_before_auth_or_http() {
        let url = DepositWalletRelayerUrl::loopback("http://127.0.0.1:1").unwrap();
        let bad_auth = RelayerKeyAuth::new("invalid\nheader", address(API_KEY_ADDRESS));
        let client =
            test_client_with_auth_clock_timeout(url, bad_auth, 1_700_000_000, Duration::from_secs(1));

        let error = client
            .submit_wallet_create(address(WALLET_CREATE_OWNER), DepositWalletMutationGate::Deny)
            .await
            .unwrap_err();

        assert!(matches!(error, RelayerError::MutationBlocked(_)));

        let error = client
            .submit_wallet_create(
                address(WALLET_CREATE_OWNER),
                DepositWalletMutationGate::Permit(DepositWalletMutationPermit::new(" ")),
            )
            .await
            .unwrap_err();

        assert!(matches!(error, RelayerError::MutationBlocked(_)));
    }

    #[tokio::test]
    async fn get_wallet_nonce_sends_exact_path_and_parses_decimal_nonce() {
        let expected = fixture_value("wallet_nonce_request.json");
        let (url, handle) =
            spawn_server(vec![TestResponse::json("200 OK", json!({"nonce": "31"}).to_string())])
                .await;
        let client = test_client(url);
        let owner: Address = expected["address"].as_str().unwrap().parse().unwrap();

        let nonce = client.get_wallet_nonce(owner).await.unwrap();

        assert_eq!(nonce, U256::from(31u64));
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, expected["method"].as_str().unwrap());
        assert_eq!(requests[0].path, expected["pathAndQuery"].as_str().unwrap());
        assert_eq!(requests[0].header("RELAYER_API_KEY"), Some(API_KEY));
        assert_eq!(
            requests[0].header("RELAYER_API_KEY_ADDRESS"),
            Some(to_checksum(&address(API_KEY_ADDRESS), None).as_str())
        );
    }

    #[tokio::test]
    async fn submit_wallet_create_sends_fixture_body_with_explicit_permit() {
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "200 OK",
            transaction_response("tx-create", "STATE_NEW"),
        )])
        .await;
        let client = test_client(url);

        let receipt = client
            .submit_wallet_create(address(WALLET_CREATE_OWNER), mutation_permit())
            .await
            .unwrap();

        assert_eq!(receipt.transaction_id, "tx-create");
        assert_eq!(receipt.state, RelayerTransactionState::New);
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, "POST");
        assert_eq!(requests[0].path, SUBMIT_PATH);
        assert_eq!(requests[0].header("RELAYER_API_KEY"), Some(API_KEY));
        assert_eq!(
            requests[0].header("content-type"),
            Some("application/json")
        );
        assert_eq!(
            serde_json::from_str::<Value>(&requests[0].body).unwrap(),
            fixture_value("wallet_create_submit_body.json")
        );
    }

    #[tokio::test]
    async fn submit_signed_wallet_batch_sends_fixture_body_with_explicit_permit() {
        let signed = signed_wallet_batch();
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "200 OK",
            json!({"nonce": signed.nonce().to_string()}).to_string(),
        ), TestResponse::json(
            "200 OK",
            transaction_response("tx-wallet", "STATE_NEW"),
        )])
        .await;
        let client = test_client(url);

        let receipt = client
            .submit_signed_wallet_batch(signed, mutation_permit())
            .await
            .unwrap();

        assert_eq!(receipt.transaction_id, "tx-wallet");
        assert_eq!(receipt.state, RelayerTransactionState::New);
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].method, "GET");
        assert!(requests[0].path.contains("/nonce?address="));
        assert_eq!(requests[1].method, "POST");
        assert_eq!(requests[1].path, SUBMIT_PATH);
        assert_eq!(
            requests[1].header("content-type"),
            Some("application/json")
        );
        assert_eq!(
            serde_json::from_str::<Value>(&requests[1].body).unwrap(),
            fixture_value("wallet_signed_submit_body.json")
        );
    }

    #[tokio::test]
    async fn submit_signed_wallet_batch_rejects_stale_nonce_before_post() {
        let signed = signed_wallet_batch();
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "200 OK",
            json!({"nonce": (signed.nonce() + U256::one()).to_string()}).to_string(),
        )])
        .await;
        let client = test_client(url);

        let error = client
            .submit_signed_wallet_batch(signed, mutation_permit())
            .await
            .unwrap_err();

        assert!(matches!(error, RelayerError::Signing(message) if message.contains("nonce")));
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, "GET");
    }

    #[tokio::test]
    async fn expired_signed_wallet_batch_fails_before_auth_or_http() {
        let url = DepositWalletRelayerUrl::loopback("http://127.0.0.1:1").unwrap();
        let bad_auth = RelayerKeyAuth::new("invalid\nheader", address(API_KEY_ADDRESS));
        let client =
            test_client_with_auth_clock_timeout(url, bad_auth, 2_000_000_000, Duration::from_secs(1));

        let error = client
            .submit_signed_wallet_batch(signed_wallet_batch(), mutation_permit())
            .await
            .unwrap_err();

        assert!(matches!(error, RelayerError::Signing(message) if message.contains("expired")));
    }

    #[tokio::test]
    async fn partial_submit_response_records_owner_scoped_ambiguous_block_until_cleared() {
        let owner = address(WALLET_CREATE_OWNER);
        let (url, handle) = spawn_server(vec![
            TestResponse::json(
                "200 OK",
                json!({"transactionID": "", "state": "STATE_NEW"}).to_string(),
            ),
            TestResponse::json("200 OK", json!({"nonce": "32"}).to_string()),
        ])
        .await;
        let client = test_client(url);

        let error = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap_err();
        assert!(matches!(error, RelayerError::AmbiguousSubmit(_)));
        assert!(client.ambiguous_submit_block(owner).is_some());

        let blocked = client.get_wallet_nonce(owner).await.unwrap_err();
        assert!(matches!(blocked, RelayerError::ReconciliationRequired(_)));

        let error = client
            .clear_ambiguous_submit_after_manual_reconciliation(
                owner,
                DepositWalletMutationPermit::new(" "),
            )
            .unwrap_err();
        assert!(matches!(error, RelayerError::MutationBlocked(_)));
        assert!(client.ambiguous_submit_block(owner).is_some());
        let blocked = client.get_wallet_nonce(owner).await.unwrap_err();
        assert!(matches!(blocked, RelayerError::ReconciliationRequired(_)));

        client
            .clear_ambiguous_submit_after_manual_reconciliation(
                owner,
                DepositWalletMutationPermit::new("checked mocked relayer state"),
            )
            .unwrap();
        let nonce = client.get_wallet_nonce(owner).await.unwrap();
        assert_eq!(nonce, U256::from(32u64));

        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].path, SUBMIT_PATH);
        assert!(requests[1].path.contains("/nonce?address="));
    }

    #[tokio::test]
    async fn submit_transport_failure_records_ambiguous_block_and_blocks_duplicate_submit() {
        let owner = address(WALLET_CREATE_OWNER);
        let (url, handle) = spawn_reset_server().await;
        let client = test_client(url);

        let error = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap_err();

        assert!(matches!(error, RelayerError::AmbiguousSubmit(_)));
        assert!(client.ambiguous_submit_block(owner).is_some());

        let duplicate = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap_err();
        assert!(matches!(duplicate, RelayerError::ReconciliationRequired(_)));

        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].path, SUBMIT_PATH);
    }

    #[tokio::test]
    async fn same_owner_submit_is_reserved_before_http_completes() {
        let owner = address(WALLET_CREATE_OWNER);
        let (url, request_rx, server) = spawn_hanging_server().await;
        let client = test_client(url);
        let first = {
            let client = client.clone();
            tokio::spawn(async move {
                client
                    .submit_wallet_create(owner, mutation_permit())
                    .await
            })
        };

        let request = request_rx.await.unwrap();
        assert_eq!(request.path, SUBMIT_PATH);

        let second = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap_err();
        assert!(matches!(second, RelayerError::ReconciliationRequired(_)));

        server.abort();
        first.abort();
    }

    #[tokio::test]
    async fn api_failures_and_quota_do_not_record_ambiguous_submit() {
        let owner = address(WALLET_CREATE_OWNER);
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "500 Internal Server Error",
            format!("server echoed {API_KEY}"),
        )])
        .await;
        let client = test_client(url);

        let error = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap_err();

        assert!(matches!(error, RelayerError::Api { status: 500, .. }));
        assert!(client.ambiguous_submit_block(owner).is_none());
        let _ = handle.await.unwrap();

        let (url, handle) =
            spawn_server(vec![TestResponse::json("429 Too Many Requests", "{}")]).await;
        let client = test_client(url);
        let error = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap_err();

        assert!(matches!(error, RelayerError::QuotaExhausted));
        assert!(client.ambiguous_submit_block(owner).is_none());
        let _ = handle.await.unwrap();
    }

    #[tokio::test]
    async fn server_timeout_status_records_ambiguous_submit() {
        let owner = address(WALLET_CREATE_OWNER);
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "504 Gateway Timeout",
            "{}",
        )])
        .await;
        let client = test_client(url);

        let error = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap_err();

        assert!(matches!(error, RelayerError::AmbiguousSubmit(_)));
        assert!(client.ambiguous_submit_block(owner).is_some());
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
    }

    #[tokio::test]
    async fn submit_unknown_state_records_reconciliation_block() {
        let owner = address(WALLET_CREATE_OWNER);
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "200 OK",
            transaction_response("tx-weird", "STATE_WEIRD\nforged"),
        )])
        .await;
        let client = test_client(url);

        let error = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap_err();
        let rendered = error.to_string();

        assert!(matches!(error, RelayerError::ReconciliationRequired(_)));
        assert!(!rendered.contains('\n'));
        assert!(client.ambiguous_submit_block(owner).is_some());
        let _ = handle.await.unwrap();
    }

    #[tokio::test]
    async fn redirects_are_not_followed_and_target_gets_no_auth() {
        let target_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        target_listener.set_nonblocking(true).unwrap();
        let target_addr = target_listener.local_addr().unwrap();

        let (url, handle) = spawn_server(vec![TestResponse::redirect(format!(
            "http://{target_addr}/redirect-target"
        ))])
        .await;
        let client = test_client(url);

        let error = client
            .get_wallet_nonce(address(WALLET_CREATE_OWNER))
            .await
            .unwrap_err();

        assert!(matches!(error, RelayerError::Api { status: 307, .. }));
        let source_requests = handle.await.unwrap();
        assert_eq!(source_requests.len(), 1);
        assert_eq!(source_requests[0].header("RELAYER_API_KEY"), Some(API_KEY));

        match target_listener.accept() {
            Err(error) if error.kind() == ErrorKind::WouldBlock => {}
            Ok(_) => panic!("redirect target unexpectedly received a request"),
            Err(error) => panic!("unexpected redirect target accept error: {error}"),
        }
    }

    #[tokio::test]
    async fn get_transaction_accepts_array_response() {
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "200 OK",
            json!([
                {
                    "transactionID": "tx-array",
                    "state": "STATE_CONFIRMED",
                    "transactionHash": "0x38cbfbeae8fffa4e2b187ee5978d3ee9cafc53af0363ed90a35b7ea9016535d8"
                }
            ])
            .to_string(),
        )])
        .await;
        let client = test_client(url);

        let receipt = client.get_transaction("tx-array").await.unwrap();

        assert_eq!(receipt.transaction_id, "tx-array");
        assert_eq!(receipt.state, RelayerTransactionState::Confirmed);
        assert_eq!(
            receipt.transaction_hash.as_deref(),
            Some("0x38cbfbeae8fffa4e2b187ee5978d3ee9cafc53af0363ed90a35b7ea9016535d8")
        );
        let requests = handle.await.unwrap();
        assert_eq!(requests[0].path, "/transaction?id=tx-array");
    }

    #[tokio::test]
    async fn get_transaction_rejects_mismatched_response_ids_and_invalid_request_ids() {
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "200 OK",
            transaction_response("other-tx", "STATE_CONFIRMED"),
        )])
        .await;
        let client = test_client(url);

        let error = client.get_transaction("tx-array").await.unwrap_err();
        assert!(matches!(error, RelayerError::ReconciliationRequired(_)));
        let _ = handle.await.unwrap();

        let url = DepositWalletRelayerUrl::loopback("http://127.0.0.1:1").unwrap();
        let client = test_client(url);
        let error = client.get_transaction("bad\nid").await.unwrap_err();
        assert!(!error.to_string().contains('\n'));
    }

    #[tokio::test]
    async fn polling_treats_only_confirmed_as_success_and_uses_injected_sleeper() {
        let (result, requests, sleeper) = poll_sequence(
            &[
                "STATE_NEW",
                "STATE_EXECUTED",
                "STATE_MINED",
                "STATE_CONFIRMED",
            ],
            4,
        )
        .await;

        let receipt = result.unwrap();
        assert_eq!(receipt.state, RelayerTransactionState::Confirmed);
        assert_eq!(requests.len(), 4);
        assert!(requests
            .iter()
            .all(|request| request.path == "/transaction?id=tx-123"));
        assert_eq!(
            sleeper.sleeps(),
            vec![
                Duration::from_millis(5),
                Duration::from_millis(10),
                Duration::from_millis(20)
            ]
        );

        let (result, _, _) = poll_sequence(&["STATE_INVALID"], 1).await;
        assert!(matches!(result.unwrap_err(), RelayerError::TransactionInvalid(_)));

        let (result, _, _) = poll_sequence(&["STATE_FAILED"], 1).await;
        assert!(matches!(result.unwrap_err(), RelayerError::TransactionFailed(_)));

        let (result, _, _) = poll_sequence(&["STATE_STRANGE"], 1).await;
        assert!(matches!(
            result.unwrap_err(),
            RelayerError::ReconciliationRequired(_)
        ));

        let (result, requests, sleeper) =
            poll_sequence(&["STATE_NEW", "STATE_NEW"], 2).await;
        assert!(matches!(result.unwrap_err(), RelayerError::Timeout));
        assert_eq!(requests.len(), 2);
        assert_eq!(sleeper.sleeps(), vec![Duration::from_millis(5)]);
    }

    #[tokio::test]
    async fn poll_timeout_keeps_owner_block_until_manual_reconciliation() {
        let owner = address(WALLET_CREATE_OWNER);
        let (url, handle) = spawn_server(vec![
            TestResponse::json("200 OK", transaction_response("tx-pending", "STATE_NEW")),
            TestResponse::json("200 OK", transaction_response("tx-pending", "STATE_NEW")),
            TestResponse::json("200 OK", transaction_response("tx-pending", "STATE_NEW")),
        ])
        .await;
        let client = test_client(url);

        let receipt = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap();
        assert_eq!(receipt.transaction_id, "tx-pending");

        let policy = DepositWalletPollPolicy::new(2, Duration::from_millis(5)).unwrap();
        let error = client
            .poll_transaction("tx-pending", policy)
            .await
            .unwrap_err();
        assert!(matches!(error, RelayerError::Timeout));

        let blocked = client.get_wallet_nonce(owner).await.unwrap_err();
        assert!(matches!(blocked, RelayerError::ReconciliationRequired(_)));

        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 3);
    }

    #[tokio::test]
    async fn confirmed_poll_clears_owner_inflight_block() {
        let owner = address(WALLET_CREATE_OWNER);
        let (url, handle) = spawn_server(vec![
            TestResponse::json("200 OK", transaction_response("tx-confirmed", "STATE_NEW")),
            TestResponse::json(
                "200 OK",
                transaction_response("tx-confirmed", "STATE_CONFIRMED"),
            ),
            TestResponse::json("200 OK", json!({"nonce": "33"}).to_string()),
        ])
        .await;
        let client = test_client(url);

        let receipt = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap();
        assert_eq!(receipt.transaction_id, "tx-confirmed");
        let blocked = client.get_wallet_nonce(owner).await.unwrap_err();
        assert!(matches!(blocked, RelayerError::ReconciliationRequired(_)));

        let receipt = client
            .poll_transaction(
                "tx-confirmed",
                DepositWalletPollPolicy::new(1, Duration::from_millis(5)).unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(receipt.state, RelayerTransactionState::Confirmed);

        let nonce = client.get_wallet_nonce(owner).await.unwrap();
        assert_eq!(nonce, U256::from(33u64));

        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 3);
    }
}
