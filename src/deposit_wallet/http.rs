use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ethers::types::{Address, H256, U256};
use ethers::utils::{keccak256, to_checksum};
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE, RETRY_AFTER};
use reqwest::{Client, Method, StatusCode};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Deserializer};
use url::Url;

use crate::deposit_wallet::{
    build_deposit_wallet_batch_request_from_signed, build_wallet_create_request,
    build_wallet_nonce_request, deposit_wallet_contract_config, DepositWalletContractConfig,
    RelayerSubmitResponse, RelayerTransactionState, SignedDepositWalletBatch, POLYGON_CHAIN_ID,
};
use crate::error::{RelayerError, Result};

const RELAYER_HOST: &str = "relayer-v2.polymarket.com";
const SUBMIT_PATH: &str = "/submit";
const TRANSACTION_PATH: &str = "/transaction";
const MAX_SUCCESS_BODY_BYTES: usize = 64 * 1024;
const MAX_ERROR_BODY_DRAIN_BYTES: usize = 8 * 1024;
const ERROR_BODY_DRAIN_TIMEOUT: Duration = Duration::from_millis(50);
const MAX_BACKGROUND_ERROR_BODY_DRAINS: usize = 64;
const RESPONSE_BODY_TOO_LARGE_MESSAGE: &str = "relayer response body exceeded maximum size";
const MAX_TRANSACTION_ID_LEN: usize = 128;
const MAX_TRANSACTION_RESPONSE_ITEMS: usize = 32;
const MAX_ERROR_TOKEN_LEN: usize = 96;
const MIN_POLL_INTERVAL: Duration = Duration::from_millis(100);
const MAX_POLL_INTERVAL: Duration = Duration::from_secs(30);
const MAX_POLL_ATTEMPTS: usize = 120;
const MAX_OWNER_MUTATION_RECORDS: usize = 1024;

#[derive(Clone, PartialEq, Eq)]
pub struct DepositWalletRelayerUrl {
    base: Url,
}

impl DepositWalletRelayerUrl {
    pub fn parse(raw: &str) -> Result<Self> {
        let url = Url::parse(raw)
            .map_err(|e| RelayerError::invalid_relayer_url(format!("could not parse URL: {e}")))?;
        validate_rel_url(&url)?;
        Ok(Self { base: url })
    }

    fn endpoint(&self, path: &str) -> Url {
        let mut url = self.base.clone();
        url.set_path(path);
        url.set_query(None);
        url
    }

    fn is_production_host(&self) -> bool {
        self.base.host_str() == Some(RELAYER_HOST)
    }

    #[cfg(test)]
    fn loopback(raw: &str) -> Result<Self> {
        let url = Url::parse(raw)
            .map_err(|e| RelayerError::invalid_relayer_url(format!("could not parse URL: {e}")))?;
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
    api_key: Arc<SecretString>,
    api_key_address: Address,
}

impl RelayerKeyAuth {
    pub fn new(api_key: impl Into<String>, api_key_address: Address) -> Self {
        Self {
            api_key: Arc::new(SecretString::from(api_key.into())),
            api_key_address,
        }
    }

    pub fn api_key_address(&self) -> Address {
        self.api_key_address
    }

    fn headers(&self) -> Result<HeaderMap> {
        let mut headers = HeaderMap::new();
        let mut api_key = HeaderValue::from_str(self.api_key.expose_secret())
            .map_err(|_| RelayerError::AuthError("invalid relayer API key".to_string()))?;
        api_key.set_sensitive(true);
        headers.insert("RELAYER_API_KEY", api_key);

        let mut api_key_address =
            HeaderValue::from_str(&to_checksum(&self.api_key_address, None)).map_err(|_| {
                RelayerError::AuthError("invalid relayer API key address".to_string())
            })?;
        api_key_address.set_sensitive(true);
        headers.insert("RELAYER_API_KEY_ADDRESS", api_key_address);
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

#[derive(Clone, Default, PartialEq, Eq)]
pub enum DepositWalletMutationGate {
    #[default]
    Deny,
    Permit(DepositWalletMutationPermit),
}

impl fmt::Debug for DepositWalletMutationGate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Deny => f.write_str("Deny"),
            Self::Permit(permit) => f.debug_tuple("Permit").field(permit).finish(),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct DepositWalletMutationPermit {
    owner: Address,
    reason: String,
    owner_serialization_evidence: String,
}

impl DepositWalletMutationPermit {
    /// Creates an explicit live-mutation permit.
    ///
    /// `owner_serialization_evidence` must identify the caller-side guard that
    /// prevents concurrent or restarted-process WALLET submits for the same
    /// owner. The client's in-memory block is only a local backstop.
    #[cfg(test)]
    pub(crate) fn new(
        owner: Address,
        reason: impl Into<String>,
        owner_serialization_evidence: impl Into<String>,
    ) -> Self {
        Self {
            owner,
            reason: reason.into(),
            owner_serialization_evidence: owner_serialization_evidence.into(),
        }
    }
}

impl fmt::Debug for DepositWalletMutationPermit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DepositWalletMutationPermit")
            .field("owner", &redacted_address(self.owner))
            .field("reason", &"<redacted>")
            .field("owner_serialization_evidence", &"<redacted>")
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
        let policy = Self {
            max_attempts,
            interval,
        };
        policy.validate()?;
        Ok(policy)
    }

    fn validate(&self) -> Result<()> {
        if self.max_attempts == 0 {
            return Err(RelayerError::Other(
                "deposit wallet poll policy max attempts must be greater than zero".to_string(),
            ));
        }
        if self.max_attempts > MAX_POLL_ATTEMPTS {
            return Err(RelayerError::Other(format!(
                "deposit wallet poll policy max attempts must not exceed {MAX_POLL_ATTEMPTS}"
            )));
        }
        if self.interval < MIN_POLL_INTERVAL {
            return Err(RelayerError::Other(format!(
                "deposit wallet poll policy interval must be at least {}ms",
                MIN_POLL_INTERVAL.as_millis()
            )));
        }

        Ok(())
    }

    fn interval_for_attempt(&self, attempt: usize) -> Duration {
        let multiplier = 1u32 << attempt.min(4);
        self.interval
            .saturating_mul(multiplier)
            .min(MAX_POLL_INTERVAL)
    }

    fn interval_for_transaction_attempt(&self, transaction_id: &str, attempt: usize) -> Duration {
        let base = self.interval_for_attempt(attempt);
        base.saturating_add(transaction_poll_jitter(transaction_id, attempt, base))
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

#[derive(Clone, Debug, PartialEq, Eq)]
struct ParsedTransactionReceipt {
    receipt: DepositWalletTransactionReceipt,
    owner: Option<Address>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RelayerTransactionResponseWithOwner {
    #[serde(flatten)]
    response: RelayerSubmitResponse,
    #[serde(default, deserialize_with = "deserialize_optional_address")]
    owner: Option<Address>,
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
        validate_relayer_contract_config(&base_url, config)?;
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
        self.fetch_wallet_nonce(owner).await
    }

    async fn fetch_wallet_nonce(&self, owner: Address) -> Result<U256> {
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
        ensure_permitted(&gate, owner)?;
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
        let owner = signed.owner();
        ensure_permitted(&gate, owner)?;
        self.ensure_owner_unblocked(owner)?;
        self.ensure_deadline_fresh(&signed)?;
        let preflight_hash = signed_digest_payload_hash(signed.digest());
        let mut reservation = self.reserve_owner_submit(owner, preflight_hash)?;

        let nonce = match self.fetch_wallet_nonce(owner).await {
            Ok(nonce) => nonce,
            Err(error) => {
                reservation.clear()?;
                return Err(error);
            }
        };
        if nonce != signed.nonce() {
            reservation.clear()?;
            return Err(RelayerError::Signing(
                "signed deposit wallet batch nonce does not match current WALLET nonce"
                    .to_string(),
            ));
        }
        if let Err(error) = self.ensure_deadline_fresh(&signed) {
            reservation.clear()?;
            return Err(error);
        }
        let request = match build_deposit_wallet_batch_request_from_signed(signed, self.config) {
            Ok(request) => request,
            Err(error) => {
                reservation.clear()?;
                return Err(error);
            }
        };
        let body = match serde_json::to_string(&request) {
            Ok(body) => body,
            Err(error) => {
                reservation.clear()?;
                return Err(RelayerError::Other(format!(
                    "could not serialize WALLET batch: {error}"
                )));
            }
        };
        reservation.update_payload_hash(payload_hash_summary(body.as_bytes()))?;
        self.submit_reserved_owner_body(reservation, body).await
    }

    pub async fn get_transaction(
        &self,
        transaction_id: &str,
    ) -> Result<DepositWalletTransactionReceipt> {
        self.fetch_transaction(transaction_id)
            .await
            .map(|parsed| parsed.receipt)
    }

    async fn fetch_transaction(&self, transaction_id: &str) -> Result<ParsedTransactionReceipt> {
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
        policy.validate()?;
        let transaction_id = validate_transaction_id(transaction_id)?;
        self.poll_validated_transaction(transaction_id, policy, None).await
    }

    pub async fn poll_owner_transaction(
        &self,
        owner: Address,
        transaction_id: &str,
        policy: DepositWalletPollPolicy,
    ) -> Result<DepositWalletTransactionReceipt> {
        policy.validate()?;
        let transaction_id = validate_transaction_id(transaction_id)?;
        self.poll_validated_transaction(transaction_id, policy, Some(owner)).await
    }

    async fn poll_validated_transaction(
        &self,
        transaction_id: String,
        policy: DepositWalletPollPolicy,
        expected_owner: Option<Address>,
    ) -> Result<DepositWalletTransactionReceipt> {
        let transaction_id_for_error = sanitized_external_token(&transaction_id);
        if let Some(owner) = expected_owner {
            let _ = self.has_recovery_owner_evidence(owner, &transaction_id)?;
        }

        for attempt in 0..policy.max_attempts {
            let parsed = match self.fetch_transaction(&transaction_id).await {
                Ok(parsed) => parsed,
                Err(error) => {
                    if let Some(owner) = expected_owner {
                        if self.has_recovery_owner_evidence(owner, &transaction_id)? {
                            self.record_recovered_ambiguous_transaction(owner, &transaction_id)?;
                        }
                    } else {
                        self.mark_transaction_reconciliation_required(&transaction_id)?;
                    }
                    return Err(error);
                }
            };
            let owner_to_verify = match expected_owner {
                Some(owner) => Some(owner),
                None => self.transaction_owner(&transaction_id)?,
            };
            if let Some(owner) = owner_to_verify {
                if let Err(error) = self.require_transaction_owner(&transaction_id, &parsed, owner)
                {
                    self.mark_transaction_reconciliation_required(&transaction_id)?;
                    return Err(error);
                }
            }
            let receipt = parsed.receipt;
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
                    match owner_to_verify {
                        Some(owner) if expected_owner.is_none() => {
                            self.record_recovered_ambiguous_transaction(owner, &transaction_id)?
                        }
                        Some(owner)
                            if self.has_recovery_owner_evidence(owner, &transaction_id)? =>
                        {
                            self.record_recovered_ambiguous_transaction(owner, &transaction_id)?
                        }
                        _ => self.mark_transaction_reconciliation_required(&transaction_id)?,
                    }
                    return Err(RelayerError::reconciliation_required(format!(
                        "deposit wallet transaction {} reached unknown state {}",
                        transaction_id_for_error,
                        sanitized_external_token(raw)
                    )));
                }
                RelayerTransactionState::New
                | RelayerTransactionState::Executed
                | RelayerTransactionState::Mined => {
                    if let Some(owner) = expected_owner {
                        if self.has_recovery_owner_evidence(owner, &transaction_id)? {
                            self.record_recovered_inflight_transaction(owner, &transaction_id)?;
                        }
                    }
                }
            }

            if attempt + 1 < policy.max_attempts {
                self.sleeper
                    .sleep(policy.interval_for_transaction_attempt(&transaction_id, attempt))
                    .await;
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
        validate_permit_owner(&permit, owner)?;
        if permit.reason.trim().is_empty() {
            return Err(RelayerError::mutation_blocked(
                "manual reconciliation reason required before clearing ambiguous submit"
                    .to_string(),
            ));
        }
        if permit.owner_serialization_evidence.trim().is_empty() {
            return Err(RelayerError::mutation_blocked(
                "manual reconciliation evidence required before clearing ambiguous submit"
                    .to_string(),
            ));
        }

        let mut state = self.mutation_state()?;
        match state.owner_blocks.get(&owner).cloned() {
            Some(OwnerMutationBlock::Ambiguous { .. }) => {
                state.owner_blocks.remove(&owner);
                state
                    .transaction_owners
                    .retain(|_, record| record.owner != owner);
            }
            Some(OwnerMutationBlock::InFlight {
                transaction_id: Some(transaction_id),
                ..
            }) => {
                return Err(RelayerError::mutation_blocked(format!(
                    "owner {} has known in-flight submit transaction {}; poll it to a terminal state before clearing",
                    redacted_address(owner),
                    sanitized_external_token(&transaction_id)
                )));
            }
            Some(OwnerMutationBlock::InFlight { .. }) => {
                return Err(RelayerError::mutation_blocked(format!(
                    "owner {} has an active submit request; wait for the response before clearing",
                    redacted_address(owner)
                )));
            }
            None => {}
        }
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
        let reservation = self.reserve_owner_submit(owner, payload_hash)?;
        self.submit_reserved_owner_body(reservation, body).await
    }

    async fn submit_reserved_owner_body(
        &self,
        mut reservation: OwnerSubmitReservation,
        body: String,
    ) -> Result<DepositWalletTransactionReceipt> {
        let owner = reservation.owner();
        let payload_hash = reservation.payload_hash().to_string();
        let url = self.base_url.endpoint(SUBMIT_PATH);
        reservation.arm_ambiguous_on_drop();
        match self.send(Method::POST, url, Some(body)).await {
            Ok(response) => match parse_submit_response(&response) {
                Ok(receipt) => {
                    let result = self.handle_submit_receipt(owner, payload_hash, receipt);
                    if result.is_ok() {
                        reservation.disarm();
                    }
                    result
                }
                Err(error) => {
                    reservation.disarm();
                    self.record_ambiguous(owner, payload_hash.clone())?;
                    Err(RelayerError::ambiguous_submit(format!(
                    "submit response did not include a usable transactionID for owner {} payload {}: {}",
                    redacted_address(owner),
                        display_payload_hash(&payload_hash),
                        error
                    )))
                }
            },
            Err(RelayerError::Http(error)) => {
                reservation.disarm();
                self.record_ambiguous(owner, payload_hash.clone())?;
                Err(RelayerError::ambiguous_submit(format!(
                    "submit transport failed for owner {} payload {}; retry status is ambiguous: {}",
                    redacted_address(owner),
                    display_payload_hash(&payload_hash),
                    sanitized_external_token(&error.to_string())
                )))
            }
            Err(RelayerError::Api {
                status,
                message: _,
            }) if is_ambiguous_submit_status(status) => {
                reservation.disarm();
                self.record_ambiguous(owner, payload_hash.clone())?;
                Err(RelayerError::ambiguous_submit(format!(
                    "submit returned ambiguous HTTP status {} for owner {} payload {}",
                    status,
                    redacted_address(owner),
                    display_payload_hash(&payload_hash)
                )))
            }
            Err(RelayerError::Other(message)) if message == RESPONSE_BODY_TOO_LARGE_MESSAGE => {
                reservation.disarm();
                self.record_ambiguous(owner, payload_hash.clone())?;
                Err(RelayerError::ambiguous_submit(format!(
                    "submit success response exceeded maximum size for owner {} payload {}; manual reconciliation required",
                    redacted_address(owner),
                    display_payload_hash(&payload_hash)
                )))
            }
            Err(error) => {
                reservation.clear()?;
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

        let response = request
            .send()
            .await
            .map_err(|error| RelayerError::Http(error.without_url()))?;
        if !response.status().is_success() {
            let status = response.status();
            let retry_after = retry_after_summary(response.headers());
            try_spawn_error_response_body_drain(response);
            if status == StatusCode::TOO_MANY_REQUESTS {
                return Err(RelayerError::QuotaExhausted);
            }
            return Err(RelayerError::Api {
                status: status.as_u16(),
                message: format!(
                    "deposit-wallet relayer request failed with HTTP {status}{retry_after}"
                ),
            });
        }

        read_limited_response_body(response, MAX_SUCCESS_BODY_BYTES).await
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

    fn reserve_owner_submit(
        &self,
        owner: Address,
        payload_hash: String,
    ) -> Result<OwnerSubmitReservation> {
        let mut state = self.mutation_state()?;
        if let Some(block) = state.owner_blocks.get(&owner) {
            return Err(owner_block_error(owner, block));
        }
        ensure_owner_mutation_capacity(&state, owner, None)?;

        state.owner_blocks.insert(
            owner,
            OwnerMutationBlock::InFlight {
                payload_hash: payload_hash.clone(),
                transaction_id: None,
            },
        );
        Ok(OwnerSubmitReservation::new(
            self.mutation_state.clone(),
            owner,
            payload_hash,
        ))
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
                Err(RelayerError::reconciliation_required(format!(
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
        ensure_owner_mutation_capacity(&state, owner, Some(&transaction_id))?;
        ensure_transaction_owner_mapping_available(&state, &transaction_id, owner, &payload_hash)?;
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
        ensure_owner_mutation_capacity(&state, owner, Some(transaction_id))?;
        ensure_transaction_owner_mapping_available(&state, transaction_id, owner, &payload_hash)?;
        state.transaction_owners.insert(
            transaction_id.to_string(),
            OwnerTransactionRecord {
                owner,
                payload_hash,
            },
        );
        Ok(())
    }

    fn record_recovered_inflight_transaction(
        &self,
        owner: Address,
        transaction_id: &str,
    ) -> Result<()> {
        let mut state = self.mutation_state()?;
        let Some(recovered_payload_hash) =
            current_recovery_payload_hash(&state, transaction_id, owner)?
        else {
            return Ok(());
        };
        ensure_transaction_owner_mapping_available(
            &state,
            transaction_id,
            owner,
            &recovered_payload_hash,
        )?;
        let payload_hash = if let Some(block) = state.owner_blocks.get(&owner) {
            match block {
                OwnerMutationBlock::InFlight {
                    transaction_id: Some(existing_transaction_id),
                    ..
                } if existing_transaction_id == transaction_id => {
                    return Ok(());
                }
                OwnerMutationBlock::Ambiguous { payload_hash }
                    if state.transaction_owners.get(transaction_id).is_some_and(|record| {
                        record.owner == owner && record.payload_hash == *payload_hash
                    }) =>
                {
                    payload_hash.clone()
                }
                _ => return Err(owner_block_error(owner, block)),
            }
        } else {
            recovered_payload_hash
        };
        if !state.owner_blocks.contains_key(&owner) {
            ensure_owner_mutation_capacity(&state, owner, Some(transaction_id))?;
        }
        state.owner_blocks.insert(
            owner,
            OwnerMutationBlock::InFlight {
                payload_hash: payload_hash.clone(),
                transaction_id: Some(transaction_id.to_string()),
            },
        );
        state.transaction_owners.insert(
            transaction_id.to_string(),
            OwnerTransactionRecord {
                owner,
                payload_hash,
            },
        );
        Ok(())
    }

    fn record_recovered_ambiguous_transaction(
        &self,
        owner: Address,
        transaction_id: &str,
    ) -> Result<()> {
        let mut state = self.mutation_state()?;
        let Some(current_payload_hash) = current_recovery_payload_hash(&state, transaction_id, owner)?
        else {
            return Ok(());
        };
        let payload_hash = match state.owner_blocks.get(&owner) {
            Some(OwnerMutationBlock::InFlight {
                payload_hash,
                transaction_id: Some(existing_transaction_id),
            }) if existing_transaction_id == transaction_id => payload_hash.clone(),
            Some(OwnerMutationBlock::Ambiguous { payload_hash })
                if state
                    .transaction_owners
                    .get(transaction_id)
                    .is_some_and(|record| record.owner == owner && record.payload_hash == *payload_hash) =>
            {
                return Ok(());
            }
            Some(block) => return Err(owner_block_error(owner, block)),
            None => current_payload_hash,
        };
        ensure_owner_mutation_capacity(&state, owner, Some(transaction_id))?;
        ensure_transaction_owner_mapping_available(&state, transaction_id, owner, &payload_hash)?;
        state.owner_blocks.insert(
            owner,
            OwnerMutationBlock::Ambiguous {
                payload_hash: payload_hash.clone(),
            },
        );
        state.transaction_owners.insert(
            transaction_id.to_string(),
            OwnerTransactionRecord {
                owner,
                payload_hash,
            },
        );
        Ok(())
    }

    fn has_recovery_owner_evidence(&self, owner: Address, transaction_id: &str) -> Result<bool> {
        let state = self.mutation_state()?;
        if let Some(record) = state.transaction_owners.get(transaction_id) {
            if record.owner != owner {
                return Err(RelayerError::reconciliation_required(format!(
                    "transaction {} is already associated with a different owner; manual reconciliation required",
                    sanitized_external_token(transaction_id)
                )));
            }
            return Ok(true);
        }

        match state.owner_blocks.get(&owner) {
            Some(OwnerMutationBlock::InFlight {
                transaction_id: Some(existing_transaction_id),
                ..
            }) if existing_transaction_id == transaction_id => Ok(true),
            Some(block) => Err(owner_block_error(owner, block)),
            None => Ok(false),
        }
    }

    fn transaction_owner(&self, transaction_id: &str) -> Result<Option<Address>> {
        let state = self.mutation_state()?;
        Ok(state
            .transaction_owners
            .get(transaction_id)
            .map(|record| record.owner))
    }

    fn require_transaction_owner(
        &self,
        transaction_id: &str,
        parsed: &ParsedTransactionReceipt,
        owner: Address,
    ) -> Result<()> {
        match parsed.owner {
            Some(response_owner) if response_owner == owner => Ok(()),
            Some(response_owner) => Err(RelayerError::reconciliation_required(format!(
                "transaction {} owner {} did not match requested owner {}",
                sanitized_external_token(transaction_id),
                redacted_address(response_owner),
                redacted_address(owner)
            ))),
            None => Err(RelayerError::reconciliation_required(format!(
                "transaction {} response did not include owner evidence; owner-scoped recovery poll cannot be used",
                sanitized_external_token(transaction_id)
            ))),
        }
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
            RelayerError::reconciliation_required(
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

struct OwnerSubmitReservation {
    state: Arc<Mutex<OwnerMutationState>>,
    owner: Address,
    payload_hash: String,
    drop_action: OwnerSubmitReservationDropAction,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OwnerSubmitReservationDropAction {
    Clear,
    Ambiguous,
    Disarmed,
}

impl OwnerSubmitReservation {
    fn new(
        state: Arc<Mutex<OwnerMutationState>>,
        owner: Address,
        payload_hash: String,
    ) -> Self {
        Self {
            state,
            owner,
            payload_hash,
            drop_action: OwnerSubmitReservationDropAction::Clear,
        }
    }

    fn owner(&self) -> Address {
        self.owner
    }

    fn payload_hash(&self) -> &str {
        &self.payload_hash
    }

    fn update_payload_hash(&mut self, payload_hash: String) -> Result<()> {
        let mut state = self.state.lock().map_err(|_| {
            RelayerError::reconciliation_required(
                "deposit wallet owner mutation state lock is poisoned; manual reconciliation required"
                    .to_string(),
            )
        })?;
        if state
            .owner_blocks
            .get(&self.owner)
            .is_some_and(|block| block.payload_hash() == self.payload_hash)
        {
            state.owner_blocks.insert(
                self.owner,
                OwnerMutationBlock::InFlight {
                    payload_hash: payload_hash.clone(),
                    transaction_id: None,
                },
            );
            self.payload_hash = payload_hash;
            Ok(())
        } else {
            Err(RelayerError::reconciliation_required(format!(
                "owner {} submit reservation changed before payload hash update",
                redacted_address(self.owner)
            )))
        }
    }

    fn clear(&mut self) -> Result<()> {
        let mut state = self.state.lock().map_err(|_| {
            RelayerError::reconciliation_required(
                "deposit wallet owner mutation state lock is poisoned; manual reconciliation required"
                    .to_string(),
            )
        })?;
        clear_owner_block_if_payload(&mut state, self.owner, &self.payload_hash);
        self.drop_action = OwnerSubmitReservationDropAction::Disarmed;
        Ok(())
    }

    fn arm_ambiguous_on_drop(&mut self) {
        self.drop_action = OwnerSubmitReservationDropAction::Ambiguous;
    }

    fn disarm(&mut self) {
        self.drop_action = OwnerSubmitReservationDropAction::Disarmed;
    }
}

impl Drop for OwnerSubmitReservation {
    fn drop(&mut self) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if state
            .owner_blocks
            .get(&self.owner)
            .is_none_or(|block| block.payload_hash() != self.payload_hash)
        {
            return;
        }

        match self.drop_action {
            OwnerSubmitReservationDropAction::Clear => {
                clear_owner_block_if_payload(&mut state, self.owner, &self.payload_hash);
            }
            OwnerSubmitReservationDropAction::Ambiguous => {
                state.owner_blocks.insert(
                    self.owner,
                    OwnerMutationBlock::Ambiguous {
                        payload_hash: self.payload_hash.clone(),
                    },
                );
            }
            OwnerSubmitReservationDropAction::Disarmed => {}
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
        return Err(RelayerError::invalid_relayer_url(
            "relayer URL must use https".to_string(),
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(RelayerError::invalid_relayer_url(
            "relayer URL must not include userinfo".to_string(),
        ));
    }
    if url.host_str() != Some(RELAYER_HOST) {
        return Err(RelayerError::invalid_relayer_url(
            "relayer URL host is not allowlisted".to_string(),
        ));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(RelayerError::invalid_relayer_url(
            "relayer URL must not include query or fragment".to_string(),
        ));
    }
    Ok(())
}

fn validate_relayer_contract_config(
    base_url: &DepositWalletRelayerUrl,
    config: DepositWalletContractConfig,
) -> Result<()> {
    if !base_url.is_production_host() {
        return Ok(());
    }
    if config != deposit_wallet_contract_config(POLYGON_CHAIN_ID)? {
        return Err(RelayerError::invalid_relayer_url(
            "production deposit wallet relayer URL requires Polygon deposit wallet contract config"
                .to_string(),
        ));
    }
    Ok(())
}

fn ensure_permitted(gate: &DepositWalletMutationGate, owner: Address) -> Result<()> {
    match gate {
        DepositWalletMutationGate::Permit(permit)
            if !permit.reason.trim().is_empty()
                && !permit.owner_serialization_evidence.trim().is_empty() =>
        {
            validate_permit_owner(permit, owner)
        }
        DepositWalletMutationGate::Permit(permit) if permit.reason.trim().is_empty() => {
            Err(RelayerError::mutation_blocked(
                "explicit deposit-wallet mutation permit reason required".to_string(),
            ))
        }
        DepositWalletMutationGate::Permit(_) => Err(RelayerError::mutation_blocked(
            "owner-scoped mutation serialization evidence required before live deposit-wallet mutation"
                .to_string(),
        )),
        DepositWalletMutationGate::Deny => Err(RelayerError::mutation_blocked(
            "explicit deposit-wallet mutation permit required".to_string(),
        )),
    }
}

fn validate_permit_owner(permit: &DepositWalletMutationPermit, owner: Address) -> Result<()> {
    if permit.owner != owner {
        return Err(RelayerError::mutation_blocked(format!(
            "deposit-wallet mutation permit owner {} does not match request owner {}",
            redacted_address(permit.owner),
            redacted_address(owner)
        )));
    }
    Ok(())
}

async fn read_limited_response_body(
    mut response: reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(RelayerError::Other(RESPONSE_BODY_TOO_LARGE_MESSAGE.to_string()));
    }

    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| RelayerError::Http(error.without_url()))?
    {
        if body.len().saturating_add(chunk.len()) > limit {
            return Err(RelayerError::Other(RESPONSE_BODY_TOO_LARGE_MESSAGE.to_string()));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

async fn drain_error_response_body(response: reqwest::Response) {
    let drain = read_limited_response_body(response, MAX_ERROR_BODY_DRAIN_BYTES);
    let _ = tokio::time::timeout(ERROR_BODY_DRAIN_TIMEOUT, drain).await;
}

fn try_spawn_error_response_body_drain(response: reqwest::Response) {
    static DRAIN_SEMAPHORE: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();
    let semaphore = DRAIN_SEMAPHORE
        .get_or_init(|| Arc::new(tokio::sync::Semaphore::new(MAX_BACKGROUND_ERROR_BODY_DRAINS)))
        .clone();
    let Ok(permit) = semaphore.try_acquire_owned() else {
        return;
    };
    tokio::spawn(async move {
        let _permit = permit;
        drain_error_response_body(response).await;
    });
}

fn retry_after_summary(headers: &HeaderMap) -> String {
    headers
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(|seconds| format!("; retry after {seconds}s"))
        .unwrap_or_default()
}

fn is_ambiguous_submit_status(status: u16) -> bool {
    status == StatusCode::REQUEST_TIMEOUT.as_u16()
        || StatusCode::from_u16(status).is_ok_and(|status| status.is_server_error())
}

fn parse_submit_response(bytes: &[u8]) -> Result<DepositWalletTransactionReceipt> {
    let response = serde_json::from_slice::<RelayerSubmitResponse>(bytes)
        .map_err(|e| RelayerError::Other(format!("could not parse submit response: {e}")))?;
    receipt_from_submit_response(response, None).map(|parsed| parsed.receipt)
}

fn parse_transaction_response(
    expected_transaction_id: &str,
    bytes: &[u8],
) -> Result<ParsedTransactionReceipt> {
    if let Ok(response) = serde_json::from_slice::<RelayerTransactionResponseWithOwner>(bytes) {
        let receipt = receipt_from_submit_response(response.response, response.owner)?;
        return require_transaction_id_match(expected_transaction_id, receipt);
    }

    let responses = serde_json::from_slice::<Vec<RelayerTransactionResponseWithOwner>>(bytes)
        .map_err(|e| RelayerError::Other(format!("could not parse transaction response: {e}")))?;
    if responses.len() > MAX_TRANSACTION_RESPONSE_ITEMS {
        return Err(RelayerError::reconciliation_required(format!(
            "transaction response included more than {MAX_TRANSACTION_RESPONSE_ITEMS} items"
        )));
    }
    let response = responses
        .into_iter()
        .find(|response| response.response.transaction_id == expected_transaction_id)
        .ok_or_else(|| {
            RelayerError::reconciliation_required(format!(
                "transaction response did not include requested transaction id {}",
                sanitized_external_token(expected_transaction_id)
            ))
        })?;
    let receipt = receipt_from_submit_response(response.response, response.owner)?;
    require_transaction_id_match(expected_transaction_id, receipt)
}

fn receipt_from_submit_response(
    response: RelayerSubmitResponse,
    owner: Option<Address>,
) -> Result<ParsedTransactionReceipt> {
    if response.transaction_id.trim().is_empty() {
        return Err(RelayerError::Other(
            "relayer response transactionID must not be empty".to_string(),
        ));
    }
    let transaction_id = validate_transaction_id(&response.transaction_id).map_err(|_| {
        RelayerError::Other("relayer response transactionID was invalid".to_string())
    })?;
    let transaction_hash = response
        .transaction_hash
        .map(|hash| validate_transaction_hash(&hash))
        .transpose()?;

    Ok(ParsedTransactionReceipt {
        receipt: DepositWalletTransactionReceipt {
            transaction_id,
            state: response.state,
            transaction_hash,
        },
        owner,
    })
}

fn require_transaction_id_match(
    expected_transaction_id: &str,
    parsed: ParsedTransactionReceipt,
) -> Result<ParsedTransactionReceipt> {
    if parsed.receipt.transaction_id != expected_transaction_id {
        return Err(RelayerError::reconciliation_required(format!(
            "transaction response id {} did not match requested id {}",
            sanitized_external_token(&parsed.receipt.transaction_id),
            sanitized_external_token(expected_transaction_id)
        )));
    }
    Ok(parsed)
}

fn validate_transaction_id(transaction_id: &str) -> Result<String> {
    if transaction_id.is_empty()
        || transaction_id.len() > MAX_TRANSACTION_ID_LEN
        || !transaction_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(RelayerError::Other(
            "transaction id must be 1-128 ASCII alphanumeric, hyphen, underscore, or period characters"
                .to_string(),
        ));
    }

    Ok(transaction_id.to_string())
}

fn validate_transaction_hash(transaction_hash: &str) -> Result<String> {
    if transaction_hash.len() == 66
        && transaction_hash.starts_with("0x")
        && transaction_hash[2..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Ok(transaction_hash.to_string());
    }

    Err(RelayerError::reconciliation_required(
        "relayer response transactionHash was invalid".to_string(),
    ))
}

fn deserialize_optional_address<'de, D>(deserializer: D) -> std::result::Result<Option<Address>, D::Error>
where
    D: Deserializer<'de>,
{
    let Some(raw) = Option::<String>::deserialize(deserializer)? else {
        return Ok(None);
    };
    raw.parse().map(Some).map_err(serde::de::Error::custom)
}

fn transaction_poll_jitter(transaction_id: &str, attempt: usize, base: Duration) -> Duration {
    let max_jitter_ms = (base.as_millis() / 4).min(250) as u64;
    if max_jitter_ms == 0 {
        return Duration::ZERO;
    }

    let mut input = transaction_id.as_bytes().to_vec();
    input.extend_from_slice(&attempt.to_be_bytes());
    let digest = keccak256(input);
    Duration::from_millis((u64::from(digest[0]) % max_jitter_ms) + 1)
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
    format!("0x{hex}")
}

fn signed_digest_payload_hash(digest: H256) -> String {
    let hex = hex::encode(digest.as_bytes());
    format!("signed-digest:0x{hex}")
}

#[cfg(test)]
fn recovered_payload_hash(transaction_id: &str) -> String {
    let hex = hex::encode(keccak256(transaction_id.as_bytes()));
    format!("recovered:0x{hex}")
}

fn display_payload_hash(payload_hash: &str) -> String {
    let Some((prefix, hex)) = payload_hash.rsplit_once("0x") else {
        return sanitized_external_token(payload_hash);
    };
    if hex.len() != 64 || !hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return sanitized_external_token(payload_hash);
    }
    format!("{prefix}0x{}...{}", &hex[..8], &hex[56..])
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
        } => RelayerError::reconciliation_required(format!(
            "owner {} has in-flight submit transaction {} payload {}; poll to terminal state before another owner mutation",
            redacted_address(owner),
            sanitized_external_token(transaction_id),
            display_payload_hash(payload_hash)
        )),
        OwnerMutationBlock::InFlight {
            payload_hash,
            transaction_id: None,
        } => RelayerError::reconciliation_required(format!(
            "owner {} has in-flight submit payload {}; wait for the submit response before another owner mutation",
            redacted_address(owner),
            display_payload_hash(payload_hash)
        )),
        OwnerMutationBlock::Ambiguous { payload_hash } => RelayerError::reconciliation_required(
            format!(
                "owner {} has ambiguous submit payload {}; manual reconciliation required",
                redacted_address(owner),
                display_payload_hash(payload_hash)
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
}

fn ensure_owner_mutation_capacity(
    state: &OwnerMutationState,
    owner: Address,
    transaction_id: Option<&str>,
) -> Result<()> {
    if !state.owner_blocks.contains_key(&owner)
        && state.owner_blocks.len() >= MAX_OWNER_MUTATION_RECORDS
    {
        return Err(RelayerError::mutation_blocked(format!(
            "owner mutation state already tracks {MAX_OWNER_MUTATION_RECORDS} owners; reconcile terminal transactions before accepting another owner"
        )));
    }
    if let Some(transaction_id) = transaction_id {
        if !state.transaction_owners.contains_key(transaction_id)
            && state.transaction_owners.len() >= MAX_OWNER_MUTATION_RECORDS
        {
            return Err(RelayerError::mutation_blocked(format!(
                "owner mutation state already tracks {MAX_OWNER_MUTATION_RECORDS} transactions; reconcile terminal transactions before accepting another transaction"
            )));
        }
    }
    Ok(())
}

fn ensure_transaction_owner_mapping_available(
    state: &OwnerMutationState,
    transaction_id: &str,
    owner: Address,
    payload_hash: &str,
) -> Result<()> {
    if let Some(existing) = state.transaction_owners.get(transaction_id) {
        if existing.owner != owner || existing.payload_hash != payload_hash {
            return Err(RelayerError::reconciliation_required(format!(
                "transaction {} is already associated with a different owner or payload; manual reconciliation required",
                sanitized_external_token(transaction_id)
            )));
        }
    }
    Ok(())
}

fn current_recovery_payload_hash(
    state: &OwnerMutationState,
    transaction_id: &str,
    owner: Address,
) -> Result<Option<String>> {
    if let Some(existing) = state.transaction_owners.get(transaction_id) {
        if existing.owner != owner {
            return Err(RelayerError::reconciliation_required(format!(
                "transaction {} is already associated with a different owner; manual reconciliation required",
                sanitized_external_token(transaction_id)
            )));
        }
        return Ok(Some(existing.payload_hash.clone()));
    }

    match state.owner_blocks.get(&owner) {
        Some(OwnerMutationBlock::InFlight {
            payload_hash,
            transaction_id: Some(existing_transaction_id),
        }) if existing_transaction_id == transaction_id => Ok(Some(payload_hash.clone())),
        Some(block) => Err(owner_block_error(owner, block)),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::io::ErrorKind;
    use std::sync::Arc;

    use ethers::types::Bytes;
    use serde_json::{json, Value};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
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
    const INVALID_RELAYER_URL_PREFIX: &str = "Invalid relayer URL:";
    const MUTATION_BLOCKED_PREFIX: &str = "Deposit-wallet mutation blocked:";
    const AMBIGUOUS_SUBMIT_PREFIX: &str = "Ambiguous deposit-wallet submit:";
    const RECONCILIATION_REQUIRED_PREFIX: &str = "Deposit-wallet reconciliation required:";
    const TEST_SERVER_TIMEOUT: Duration = Duration::from_secs(2);

    #[derive(Clone)]
    struct FixedClock {
        now: u64,
    }

    impl DepositWalletClock for FixedClock {
        fn now_unix_seconds(&self) -> u64 {
            self.now
        }
    }

    struct SequenceClock {
        values: Mutex<VecDeque<u64>>,
    }

    impl SequenceClock {
        fn new(values: impl IntoIterator<Item = u64>) -> Self {
            Self {
                values: Mutex::new(values.into_iter().collect()),
            }
        }
    }

    impl DepositWalletClock for SequenceClock {
        fn now_unix_seconds(&self) -> u64 {
            let mut values = self.values.lock().unwrap();
            if values.len() > 1 {
                values.pop_front().unwrap()
            } else {
                *values.front().unwrap()
            }
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

    #[derive(Default)]
    struct ClearingSleeper {
        sleeps: Mutex<Vec<Duration>>,
        state: Mutex<Option<Arc<Mutex<OwnerMutationState>>>>,
    }

    impl ClearingSleeper {
        fn attach_state(&self, state: Arc<Mutex<OwnerMutationState>>) {
            *self.state.lock().unwrap() = Some(state);
        }
    }

    impl DepositWalletSleeper for ClearingSleeper {
        fn sleep<'a>(
            &'a self,
            duration: Duration,
        ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
            self.sleeps.lock().unwrap().push(duration);
            let state = self.state.lock().unwrap().clone();
            if let Some(state) = state {
                let mut state = state.lock().unwrap();
                state.owner_blocks.clear();
                state.transaction_owners.clear();
            }
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
        include_content_length: bool,
        body: String,
    }

    impl TestResponse {
        fn json(status: &'static str, body: impl Into<String>) -> Self {
            Self {
                status,
                headers: vec![("content-type".to_string(), "application/json".to_string())],
                include_content_length: true,
                body: body.into(),
            }
        }

        fn json_without_content_length(status: &'static str, body: impl Into<String>) -> Self {
            Self {
                status,
                headers: vec![("content-type".to_string(), "application/json".to_string())],
                include_content_length: false,
                body: body.into(),
            }
        }

        fn redirect(location: String) -> Self {
            Self {
                status: "307 Temporary Redirect",
                headers: vec![("location".to_string(), location)],
                include_content_length: true,
                body: String::new(),
            }
        }

        fn with_header(mut self, name: &str, value: &str) -> Self {
            self.headers.push((name.to_string(), value.to_string()));
            self
        }
    }

    fn address(raw: &str) -> Address {
        raw.parse().expect("test address should parse")
    }

    fn error_has_prefix(error: &RelayerError, prefix: &str) -> bool {
        matches!(error, RelayerError::Other(message) if message.starts_with(prefix))
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
        mutation_permit_for(address(WALLET_CREATE_OWNER))
    }

    fn mutation_permit_for(owner: Address) -> DepositWalletMutationGate {
        DepositWalletMutationGate::Permit(DepositWalletMutationPermit::new(
            owner,
            "mocked unit-test relayer call",
            "single-process mocked owner serialization guard",
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
                let (mut stream, _) = tokio::time::timeout(TEST_SERVER_TIMEOUT, listener.accept())
                    .await
                    .expect("server accept should not hang")
                    .expect("server should accept");
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

    async fn spawn_reset_server() -> (DepositWalletRelayerUrl, JoinHandle<Vec<CapturedRequest>>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test server should bind");
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let (mut stream, _) = tokio::time::timeout(TEST_SERVER_TIMEOUT, listener.accept())
                .await
                .expect("server accept should not hang")
                .expect("server should accept");
            let request = read_request(&mut stream).await;
            drop(stream);
            vec![request]
        });

        (
            DepositWalletRelayerUrl::loopback(&format!("http://{addr}")).unwrap(),
            handle,
        )
    }

    async fn spawn_truncated_error_body_server(
        status: &'static str,
    ) -> (DepositWalletRelayerUrl, JoinHandle<Vec<CapturedRequest>>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test server should bind");
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let (mut stream, _) = tokio::time::timeout(TEST_SERVER_TIMEOUT, listener.accept())
                .await
                .expect("server accept should not hang")
                .expect("server should accept");
            let request = read_request(&mut stream).await;
            let wire = format!(
                "HTTP/1.1 {status}\r\nconnection: close\r\ncontent-type: application/json\r\ncontent-length: 1024\r\n\r\npartial"
            );
            stream
                .write_all(wire.as_bytes())
                .await
                .expect("partial response should write");
            vec![request]
        });

        (
            DepositWalletRelayerUrl::loopback(&format!("http://{addr}")).unwrap(),
            handle,
        )
    }

    async fn spawn_truncated_success_body_server() -> (
        DepositWalletRelayerUrl,
        JoinHandle<Vec<CapturedRequest>>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test server should bind");
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let (mut stream, _) = tokio::time::timeout(TEST_SERVER_TIMEOUT, listener.accept())
                .await
                .expect("server accept should not hang")
                .expect("server should accept");
            let request = read_request(&mut stream).await;
            let wire = "HTTP/1.1 200 OK\r\nconnection: close\r\ncontent-type: application/json\r\ncontent-length: 1024\r\n\r\n{\"nonce\":";
            stream
                .write_all(wire.as_bytes())
                .await
                .expect("partial response should write");
            vec![request]
        });

        (
            DepositWalletRelayerUrl::loopback(&format!("http://{addr}")).unwrap(),
            handle,
        )
    }

    async fn spawn_controlled_error_body_server() -> (
        DepositWalletRelayerUrl,
        JoinHandle<Vec<CapturedRequest>>,
        tokio::sync::oneshot::Receiver<()>,
        tokio::sync::oneshot::Sender<()>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test server should bind");
        let addr = listener.local_addr().unwrap();
        let (headers_sent_tx, headers_sent_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(async move {
            let (mut stream, _) = tokio::time::timeout(TEST_SERVER_TIMEOUT, listener.accept())
                .await
                .expect("server accept should not hang")
                .expect("server should accept");
            let request = read_request(&mut stream).await;
            let wire = "HTTP/1.1 500 Internal Server Error\r\nconnection: close\r\ncontent-type: application/json\r\ncontent-length: 1024\r\n\r\npartial";
            stream
                .write_all(wire.as_bytes())
                .await
                .expect("partial response should write");
            let _ = headers_sent_tx.send(());
            let _ = release_rx.await;
            vec![request]
        });

        (
            DepositWalletRelayerUrl::loopback(&format!("http://{addr}")).unwrap(),
            handle,
            headers_sent_rx,
            release_tx,
        )
    }

    async fn read_request(stream: &mut TcpStream) -> CapturedRequest {
        let mut buffer = Vec::new();
        let headers_end = loop {
            let mut chunk = [0u8; 1024];
            let read = tokio::time::timeout(TEST_SERVER_TIMEOUT, stream.read(&mut chunk))
                .await
                .expect("header read should not hang")
                .expect("request should read");
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
            let read = tokio::time::timeout(TEST_SERVER_TIMEOUT, stream.read(&mut chunk))
                .await
                .expect("body read should not hang")
                .expect("body should read");
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
        let mut wire = format!("HTTP/1.1 {}\r\nconnection: close\r\n", response.status);
        if response.include_content_length {
            wire.push_str(&format!("content-length: {}\r\n", response.body.len()));
        }
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
            "transactionHash": "0x38cbfbeae8fffa4e2b187ee5978d3ee9cafc53af0363ed90a35b7ea9016535d8",
            "owner": WALLET_CREATE_OWNER
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
        DepositWalletPollPolicy,
    ) {
        let responses = states
            .iter()
            .map(|state| TestResponse::json("200 OK", transaction_response("tx-123", state)))
            .collect::<Vec<_>>();
        let (url, handle) = spawn_server(responses).await;
        let sleeper = Arc::new(RecordingSleeper::default());
        let client = test_client_with_sleeper(url, sleeper.clone());
        let policy =
            DepositWalletPollPolicy::new(max_attempts, Duration::from_millis(100)).unwrap();
        let result = client.poll_transaction("tx-123", policy.clone()).await;
        let requests = handle.await.unwrap();
        (result, requests, sleeper, policy)
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
            let error = DepositWalletRelayerUrl::parse(raw)
                .expect_err("unsafe relayer URL should be rejected");
            assert!(error_has_prefix(&error, INVALID_RELAYER_URL_PREFIX));
        }

        DepositWalletRelayerUrl::parse("https://relayer-v2.polymarket.com").unwrap();
    }

    #[test]
    fn production_relayer_client_rejects_non_polygon_contract_config() {
        let url = DepositWalletRelayerUrl::parse("https://relayer-v2.polymarket.com").unwrap();
        let amoy_config = deposit_wallet_contract_config(80002).unwrap();
        let error = DepositWalletRelayerClient::new(url, relayer_auth(), amoy_config).unwrap_err();

        assert!(error_has_prefix(&error, INVALID_RELAYER_URL_PREFIX));
        assert!(error.to_string().contains("Polygon"));
    }

    #[test]
    fn auth_debug_redacts_secret_bearing_fields() {
        let auth = relayer_auth();
        let rendered = format!("{auth:?}");
        assert!(!rendered.contains(API_KEY));
        assert!(!rendered.contains(&to_checksum(&address(API_KEY_ADDRESS), None)));
        let headers = auth.headers().unwrap();
        assert!(headers.get("RELAYER_API_KEY").unwrap().is_sensitive());
        assert!(headers
            .get("RELAYER_API_KEY_ADDRESS")
            .unwrap()
            .is_sensitive());
        let rendered_headers = format!("{headers:?}");
        assert!(!rendered_headers.contains(API_KEY));
        assert!(!rendered_headers.contains(&to_checksum(&address(API_KEY_ADDRESS), None)));

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
    async fn transport_errors_do_not_echo_full_nonce_url() {
        let (url, handle) = spawn_reset_server().await;
        let client = test_client_with_auth_clock_timeout(
            url,
            relayer_auth(),
            1_700_000_000,
            Duration::from_millis(500),
        );
        let owner = address(WALLET_CREATE_OWNER);

        let error = client.get_wallet_nonce(owner).await.unwrap_err();
        let rendered = error.to_string();

        assert!(!rendered.contains("/nonce"));
        assert!(!rendered.contains(&to_checksum(&owner, None)));
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].path.contains("/nonce?address="));
    }

    #[tokio::test]
    async fn success_body_read_errors_do_not_echo_full_nonce_url() {
        let (url, handle) = spawn_truncated_success_body_server().await;
        let client = test_client_with_auth_clock_timeout(
            url,
            relayer_auth(),
            1_700_000_000,
            Duration::from_millis(100),
        );
        let owner = address(WALLET_CREATE_OWNER);

        let error = client.get_wallet_nonce(owner).await.unwrap_err();
        let rendered = error.to_string();

        assert!(!rendered.contains("/nonce"));
        assert!(!rendered.contains(&to_checksum(&owner, None)));
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].path.contains("/nonce?address="));
    }

    #[tokio::test]
    async fn non_success_error_body_drain_does_not_delay_caller() {
        let (url, handle, headers_sent, release_server) =
            spawn_controlled_error_body_server().await;
        let client = test_client_with_auth_clock_timeout(
            url,
            relayer_auth(),
            1_700_000_000,
            Duration::from_secs(1),
        );
        let owner = address(WALLET_CREATE_OWNER);

        let client_task = tokio::spawn(async move { client.get_wallet_nonce(owner).await });
        headers_sent
            .await
            .expect("server should send non-success headers");

        let result = tokio::time::timeout(TEST_SERVER_TIMEOUT, client_task)
        .await
        .expect("non-success status should return while error body is still held")
        .expect("client task should not panic");
        assert!(matches!(result.unwrap_err(), RelayerError::Api { status: 500, .. }));

        let _ = release_server.send(());
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

        assert!(error_has_prefix(&error, MUTATION_BLOCKED_PREFIX));

        let error = client
            .submit_wallet_create(
                address(WALLET_CREATE_OWNER),
                DepositWalletMutationGate::Permit(DepositWalletMutationPermit::new(
                    address(WALLET_CREATE_OWNER),
                    " ",
                    "single-process mocked owner serialization guard",
                )),
            )
            .await
            .unwrap_err();

        assert!(error_has_prefix(&error, MUTATION_BLOCKED_PREFIX));

        let error = client
            .submit_wallet_create(
                address(WALLET_CREATE_OWNER),
                DepositWalletMutationGate::Permit(DepositWalletMutationPermit::new(
                    address(WALLET_CREATE_OWNER),
                    "mocked unit-test relayer call",
                    "",
                )),
            )
            .await
            .unwrap_err();

        assert!(error_has_prefix(&error, MUTATION_BLOCKED_PREFIX));

        let error = client
            .submit_signed_wallet_batch(signed_wallet_batch(), DepositWalletMutationGate::Deny)
            .await
            .unwrap_err();

        assert!(error_has_prefix(&error, MUTATION_BLOCKED_PREFIX));

        let error = client
            .submit_wallet_create(
                address(WALLET_CREATE_OWNER),
                DepositWalletMutationGate::Permit(DepositWalletMutationPermit::new(
                    Address::zero(),
                    "mocked unit-test relayer call",
                    "single-process mocked owner serialization guard",
                )),
            )
            .await
            .unwrap_err();

        assert!(error_has_prefix(&error, MUTATION_BLOCKED_PREFIX));

        let signed = signed_wallet_batch();
        let owner = signed.owner();
        let error = client
            .submit_signed_wallet_batch(
                signed,
                DepositWalletMutationGate::Permit(DepositWalletMutationPermit::new(
                    owner,
                    "",
                    "single-process mocked owner serialization guard",
                )),
            )
            .await
            .unwrap_err();

        assert!(error_has_prefix(&error, MUTATION_BLOCKED_PREFIX));
    }

    #[test]
    fn mutation_permit_debug_redacts_approval_evidence() {
        let owner = address(WALLET_CREATE_OWNER);
        let permit = DepositWalletMutationPermit::new(
            owner,
            "ticket-123 caller lock",
            "owner-lock-key-456",
        );
        let rendered_permit = format!("{permit:?}");
        let rendered_gate = format!("{:?}", DepositWalletMutationGate::Permit(permit));

        assert!(rendered_permit.contains("DepositWalletMutationPermit"));
        assert!(rendered_permit.contains("<redacted>"));
        assert!(!rendered_permit.contains("ticket-123"));
        assert!(!rendered_permit.contains("owner-lock-key-456"));
        assert!(!rendered_gate.contains("ticket-123"));
        assert!(!rendered_gate.contains("owner-lock-key-456"));
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
        let owner = signed.owner();
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
            .submit_signed_wallet_batch(signed, mutation_permit_for(owner))
            .await
            .unwrap();

        assert_eq!(receipt.transaction_id, "tx-wallet");
        assert_eq!(receipt.state, RelayerTransactionState::New);
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].method, "GET");
        assert_eq!(
            requests[0].path,
            format!("/nonce?address={}&type=WALLET", to_checksum(&owner, None))
        );
        assert_eq!(requests[0].header("RELAYER_API_KEY"), Some(API_KEY));
        assert_eq!(
            requests[0].header("RELAYER_API_KEY_ADDRESS"),
            Some(to_checksum(&address(API_KEY_ADDRESS), None).as_str())
        );
        assert_ne!(
            requests[0].header("RELAYER_API_KEY_ADDRESS"),
            Some(to_checksum(&owner, None).as_str())
        );
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
        let owner = signed.owner();

        let error = client
            .submit_signed_wallet_batch(signed, mutation_permit_for(owner))
            .await
            .unwrap_err();

        assert!(matches!(error, RelayerError::Signing(message) if message.contains("nonce")));
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, "GET");
    }

    #[tokio::test]
    async fn signed_wallet_local_request_build_failure_clears_owner_reservation() {
        let signed = signed_wallet_batch();
        let owner = signed.owner();
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "200 OK",
            json!({"nonce": signed.nonce().to_string()}).to_string(),
        )])
        .await;
        let clock: Arc<dyn DepositWalletClock> = Arc::new(FixedClock { now: 1_700_000_000 });
        let sleeper: Arc<dyn DepositWalletSleeper> = Arc::new(RecordingSleeper::default());
        let client = DepositWalletRelayerClient::from_parts(
            reqwest_client(Duration::from_secs(2)),
            url,
            relayer_auth(),
            deposit_wallet_contract_config(80002).unwrap(),
            clock,
            sleeper,
        );

        let error = client
            .submit_signed_wallet_batch(signed, mutation_permit_for(owner))
            .await
            .unwrap_err();

        assert!(matches!(error, RelayerError::Signing(_)));
        assert!(client.ambiguous_submit_block(owner).is_none());
        client.ensure_owner_unblocked(owner).unwrap();
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, "GET");
    }

    #[tokio::test]
    async fn signed_wallet_batch_rechecks_deadline_after_nonce_lookup_before_post() {
        let signed = signed_wallet_batch();
        let deadline = signed.deadline().as_u64();
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "200 OK",
            json!({"nonce": signed.nonce().to_string()}).to_string(),
        )])
        .await;
        let clock: Arc<dyn DepositWalletClock> = Arc::new(SequenceClock::new([
            1_700_000_000,
            deadline,
        ]));
        let sleeper: Arc<dyn DepositWalletSleeper> = Arc::new(RecordingSleeper::default());
        let client = DepositWalletRelayerClient::from_parts(
            reqwest_client(Duration::from_secs(2)),
            url,
            relayer_auth(),
            deposit_wallet_contract_config(137).unwrap(),
            clock,
            sleeper,
        );
        let owner = signed.owner();

        let error = client
            .submit_signed_wallet_batch(signed, mutation_permit_for(owner))
            .await
            .unwrap_err();

        assert!(matches!(error, RelayerError::Signing(message) if message.contains("expired")));
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

        let signed = signed_wallet_batch();
        let owner = signed.owner();
        let error = client
            .submit_signed_wallet_batch(signed, mutation_permit_for(owner))
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
        assert!(error_has_prefix(&error, AMBIGUOUS_SUBMIT_PREFIX));
        assert!(client.ambiguous_submit_block(owner).is_some());

        let blocked = client.get_wallet_nonce(owner).await.unwrap_err();
        assert!(error_has_prefix(&blocked, RECONCILIATION_REQUIRED_PREFIX));

        let error = client
            .clear_ambiguous_submit_after_manual_reconciliation(
                owner,
                DepositWalletMutationPermit::new(
                    owner,
                    " ",
                    "single-process mocked owner serialization guard",
                ),
            )
            .unwrap_err();
        assert!(error_has_prefix(&error, MUTATION_BLOCKED_PREFIX));
        assert!(client.ambiguous_submit_block(owner).is_some());
        let blocked = client.get_wallet_nonce(owner).await.unwrap_err();
        assert!(error_has_prefix(&blocked, RECONCILIATION_REQUIRED_PREFIX));

        let error = client
            .clear_ambiguous_submit_after_manual_reconciliation(
                owner,
                DepositWalletMutationPermit::new(owner, "checked mocked relayer state", ""),
            )
            .unwrap_err();
        assert!(error_has_prefix(&error, MUTATION_BLOCKED_PREFIX));
        assert!(client.ambiguous_submit_block(owner).is_some());

        client
            .clear_ambiguous_submit_after_manual_reconciliation(
                owner,
                DepositWalletMutationPermit::new(
                    owner,
                    "checked mocked relayer state",
                    "single-process mocked owner serialization guard",
                ),
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

        assert!(error_has_prefix(&error, AMBIGUOUS_SUBMIT_PREFIX));
        assert!(client.ambiguous_submit_block(owner).is_some());

        let duplicate = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap_err();
        assert!(error_has_prefix(&duplicate, RECONCILIATION_REQUIRED_PREFIX));

        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].path, SUBMIT_PATH);
    }

    #[tokio::test]
    async fn signed_submit_nonce_failure_clears_owner_reservation() {
        let signed = signed_wallet_batch();
        let owner = signed.owner();
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "400 Bad Request",
            json!({"error": "nonce unavailable"}).to_string(),
        )])
        .await;
        let client = test_client(url);

        let error = client
            .submit_signed_wallet_batch(signed, mutation_permit_for(owner))
            .await
            .unwrap_err();

        assert!(matches!(error, RelayerError::Api { status: 400, .. }));
        assert!(client.ambiguous_submit_block(owner).is_none());
        client.ensure_owner_unblocked(owner).unwrap();
        let mut retry_reservation = client
            .reserve_owner_submit(owner, "payload:retry-after-nonce-error".to_string())
            .unwrap();
        retry_reservation.clear().unwrap();
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].path.contains("/nonce?address="));
    }

    #[test]
    fn owner_submit_reservation_blocks_duplicate_and_clears_cleanly() {
        let owner = address(WALLET_CREATE_OWNER);
        let client =
            test_client(DepositWalletRelayerUrl::loopback("http://127.0.0.1:1").unwrap());
        let mut reservation = client
            .reserve_owner_submit(owner, "payload:unit-test".to_string())
            .unwrap();

        let second = match client.reserve_owner_submit(owner, "payload:duplicate".to_string()) {
            Ok(_) => panic!("duplicate owner reservation should fail"),
            Err(error) => error,
        };
        assert!(error_has_prefix(&second, RECONCILIATION_REQUIRED_PREFIX));
        assert!(client.ensure_owner_unblocked(owner).is_err());

        reservation.clear().unwrap();
        client.ensure_owner_unblocked(owner).unwrap();
    }

    #[test]
    fn dropped_pre_submit_owner_reservation_clears_owner_block() {
        let signed = signed_wallet_batch();
        let owner = signed.owner();
        let client =
            test_client(DepositWalletRelayerUrl::loopback("http://127.0.0.1:1").unwrap());
        let payload_hash = signed_digest_payload_hash(signed.digest());
        let reservation = client
            .reserve_owner_submit(owner, payload_hash.clone())
            .unwrap();

        let duplicate = match client.reserve_owner_submit(owner, "payload:duplicate".to_string()) {
            Ok(_) => panic!("duplicate owner reservation should fail"),
            Err(error) => error,
        };
        assert!(error_has_prefix(&duplicate, RECONCILIATION_REQUIRED_PREFIX));

        drop(reservation);
        assert!(client.ambiguous_submit_block(owner).is_none());
        client.ensure_owner_unblocked(owner).unwrap();
    }

    #[test]
    fn dropped_post_submit_owner_reservation_records_ambiguous_block() {
        let signed = signed_wallet_batch();
        let owner = signed.owner();
        let client =
            test_client(DepositWalletRelayerUrl::loopback("http://127.0.0.1:1").unwrap());
        let payload_hash = signed_digest_payload_hash(signed.digest());
        let mut reservation = client
            .reserve_owner_submit(owner, payload_hash.clone())
            .unwrap();
        reservation.arm_ambiguous_on_drop();

        drop(reservation);
        assert_eq!(client.ambiguous_submit_block(owner), Some(payload_hash));
    }

    #[test]
    fn owner_mutation_state_rejects_new_entries_after_capacity() {
        let client =
            test_client(DepositWalletRelayerUrl::loopback("http://127.0.0.1:1").unwrap());
        {
            let mut state = client.mutation_state().unwrap();
            for index in 0..MAX_OWNER_MUTATION_RECORDS {
                let owner = Address::from_low_u64_be(index as u64 + 1);
                let payload_hash = format!("payload-{index}");
                state.owner_blocks.insert(
                    owner,
                    OwnerMutationBlock::Ambiguous {
                        payload_hash: payload_hash.clone(),
                    },
                );
                state.transaction_owners.insert(
                    format!("tx-{index}"),
                    OwnerTransactionRecord {
                        owner,
                        payload_hash,
                    },
                );
            }
        }

        let error = client
            .reserve_owner_submit(Address::from_low_u64_be(10_000), "payload-overflow".to_string())
            .err()
            .expect("new owner reservation should respect capacity");

        assert!(error_has_prefix(&error, MUTATION_BLOCKED_PREFIX));
    }

    #[test]
    fn owner_mutation_state_rejects_new_transactions_after_transaction_capacity() {
        let client =
            test_client(DepositWalletRelayerUrl::loopback("http://127.0.0.1:1").unwrap());
        let owner = address(WALLET_CREATE_OWNER);
        {
            let mut state = client.mutation_state().unwrap();
            for index in 0..MAX_OWNER_MUTATION_RECORDS {
                state.transaction_owners.insert(
                    format!("tx-{index}"),
                    OwnerTransactionRecord {
                        owner: Address::from_low_u64_be(index as u64 + 1),
                        payload_hash: format!("payload-{index}"),
                    },
                );
            }
        }

        let error = client
            .record_transaction_owner("tx-overflow", owner, "payload-overflow".to_string())
            .expect_err("new transaction owner should respect transaction capacity");

        assert!(error_has_prefix(&error, MUTATION_BLOCKED_PREFIX));
        client.ensure_owner_unblocked(owner).unwrap();
    }

    #[test]
    fn owner_mutation_state_rejects_transaction_owner_mapping_conflicts() {
        let client =
            test_client(DepositWalletRelayerUrl::loopback("http://127.0.0.1:1").unwrap());
        let owner = address(WALLET_CREATE_OWNER);
        let other_owner = address("0x0000000000000000000000000000000000000001");

        client
            .record_inflight_transaction(
                owner,
                "payload-original".to_string(),
                "tx-reused".to_string(),
            )
            .unwrap();

        let error = client
            .record_inflight_transaction(
                other_owner,
                "payload-other".to_string(),
                "tx-reused".to_string(),
            )
            .expect_err("transaction id reuse across owners should require reconciliation");

        assert!(error_has_prefix(&error, RECONCILIATION_REQUIRED_PREFIX));
        client.ensure_owner_unblocked(other_owner).unwrap();
        let state = client.mutation_state().unwrap();
        let record = state.transaction_owners.get("tx-reused").unwrap();
        assert_eq!(record.owner, owner);
        assert_eq!(record.payload_hash, "payload-original");
    }

    #[tokio::test]
    async fn api_failures_and_quota_do_not_record_ambiguous_submit() {
        let owner = address(WALLET_CREATE_OWNER);
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "400 Bad Request",
            format!("request echoed {API_KEY}"),
        )])
        .await;
        let client = test_client(url);

        let error = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap_err();

        assert!(matches!(error, RelayerError::Api { status: 400, .. }));
        assert!(client.ambiguous_submit_block(owner).is_none());
        let _ = handle.await.unwrap();

        let (url, handle) = spawn_server(vec![TestResponse::json("429 Too Many Requests", "{}")
            .with_header("retry-after", "7")])
        .await;
        let client = test_client(url);
        let error = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            RelayerError::QuotaExhausted
        ));
        assert!(client.ambiguous_submit_block(owner).is_none());
        let _ = handle.await.unwrap();
    }

    #[tokio::test]
    async fn non_success_status_survives_truncated_error_body() {
        let (url, handle) = spawn_truncated_error_body_server("400 Bad Request").await;
        let client = test_client(url);

        let error = tokio::time::timeout(
            Duration::from_secs(1),
            client.get_wallet_nonce(address(WALLET_CREATE_OWNER)),
        )
        .await
        .expect("truncated 400 drain should not wait for the client timeout")
            .unwrap_err();

        assert!(matches!(error, RelayerError::Api { status: 400, .. }));
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].path.contains("/nonce?address="));

        let (url, handle) = spawn_truncated_error_body_server("429 Too Many Requests").await;
        let client = test_client(url);
        let error = tokio::time::timeout(
            Duration::from_secs(1),
            client.get_wallet_nonce(address(WALLET_CREATE_OWNER)),
        )
        .await
        .expect("truncated 429 drain should not wait for the client timeout")
            .unwrap_err();

        assert!(matches!(error, RelayerError::QuotaExhausted));
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].path.contains("/nonce?address="));
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

        assert!(error_has_prefix(&error, AMBIGUOUS_SUBMIT_PREFIX));
        assert!(client.ambiguous_submit_block(owner).is_some());
        let duplicate = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap_err();
        assert!(error_has_prefix(&duplicate, RECONCILIATION_REQUIRED_PREFIX));
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
    }

    #[tokio::test]
    async fn request_timeout_status_records_ambiguous_submit_and_blocks_duplicate() {
        let owner = address(WALLET_CREATE_OWNER);
        let (url, handle) = spawn_server(vec![TestResponse::json("408 Request Timeout", "{}")])
            .await;
        let client = test_client(url);

        let error = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap_err();

        assert!(error_has_prefix(&error, AMBIGUOUS_SUBMIT_PREFIX));
        assert!(client.ambiguous_submit_block(owner).is_some());
        let duplicate = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap_err();
        assert!(error_has_prefix(&duplicate, RECONCILIATION_REQUIRED_PREFIX));
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
    }

    #[tokio::test]
    async fn server_error_status_records_ambiguous_submit_and_blocks_duplicate() {
        let owner = address(WALLET_CREATE_OWNER);
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "500 Internal Server Error",
            "{}",
        )])
        .await;
        let client = test_client(url);

        let error = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap_err();

        assert!(error_has_prefix(&error, AMBIGUOUS_SUBMIT_PREFIX));
        assert!(client.ambiguous_submit_block(owner).is_some());
        let duplicate = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap_err();
        assert!(error_has_prefix(&duplicate, RECONCILIATION_REQUIRED_PREFIX));
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
    }

    #[tokio::test]
    async fn oversized_submit_success_response_records_ambiguous_submit() {
        let owner = address(WALLET_CREATE_OWNER);
        let (url, handle) = spawn_server(vec![TestResponse::json_without_content_length(
            "200 OK",
            "x".repeat(MAX_SUCCESS_BODY_BYTES + 1),
        )])
        .await;
        let client = test_client(url);

        let error = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap_err();

        assert!(error_has_prefix(&error, AMBIGUOUS_SUBMIT_PREFIX));
        assert!(client.ambiguous_submit_block(owner).is_some());
        let duplicate = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap_err();
        assert!(error_has_prefix(&duplicate, RECONCILIATION_REQUIRED_PREFIX));
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

        assert!(error_has_prefix(&error, RECONCILIATION_REQUIRED_PREFIX));
        assert!(!rendered.contains('\n'));
        assert!(client.ambiguous_submit_block(owner).is_some());
        let _ = handle.await.unwrap();
    }

    #[tokio::test]
    async fn submit_receipt_recording_failure_leaves_clearable_ambiguous_block() {
        let owner = address(WALLET_CREATE_OWNER);
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "200 OK",
            transaction_response("tx-overflow", "STATE_NEW"),
        )])
        .await;
        let client = test_client(url);
        {
            let mut state = client.mutation_state().unwrap();
            for index in 0..MAX_OWNER_MUTATION_RECORDS {
                state.transaction_owners.insert(
                    format!("tx-existing-{index}"),
                    OwnerTransactionRecord {
                        owner: Address::from_low_u64_be(index as u64 + 1),
                        payload_hash: format!("payload-{index}"),
                    },
                );
            }
        }

        let error = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap_err();

        assert!(error_has_prefix(&error, MUTATION_BLOCKED_PREFIX));
        assert!(client.ambiguous_submit_block(owner).is_some());
        client
            .clear_ambiguous_submit_after_manual_reconciliation(
                owner,
                DepositWalletMutationPermit::new(
                    owner,
                    "checked mocked submit recording failure",
                    "single-process mocked owner serialization guard",
                ),
            )
            .unwrap();
        client.ensure_owner_unblocked(owner).unwrap();
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
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
        let client =
            DepositWalletRelayerClient::new(url, relayer_auth(), deposit_wallet_contract_config(137).unwrap())
                .unwrap();

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
                    "transactionID": "other-tx",
                    "state": "STATE_FAILED",
                    "transactionHash": "0x38cbfbeae8fffa4e2b187ee5978d3ee9cafc53af0363ed90a35b7ea9016535d8"
                },
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
        assert!(error_has_prefix(&error, RECONCILIATION_REQUIRED_PREFIX));
        let _ = handle.await.unwrap();

        let (url, handle) = spawn_server(vec![TestResponse::json(
            "200 OK",
            json!([
                {
                    "transactionID": "other-tx",
                    "state": "STATE_CONFIRMED",
                    "transactionHash": "0x38cbfbeae8fffa4e2b187ee5978d3ee9cafc53af0363ed90a35b7ea9016535d8"
                }
            ])
            .to_string(),
        )])
        .await;
        let client = test_client(url);

        let error = client.get_transaction("tx-array").await.unwrap_err();
        assert!(error_has_prefix(&error, RECONCILIATION_REQUIRED_PREFIX));
        let _ = handle.await.unwrap();

        let (url, handle) = spawn_server(vec![TestResponse::json(
            "200 OK",
            transaction_response(" tx-array ", "STATE_CONFIRMED"),
        )])
        .await;
        let client = test_client(url);

        let error = client.get_transaction("tx-array").await.unwrap_err();
        assert!(matches!(error, RelayerError::Other(_)));
        let _ = handle.await.unwrap();

        let url = DepositWalletRelayerUrl::loopback("http://127.0.0.1:1").unwrap();
        let client = test_client(url);
        let error = client.get_transaction("bad\nid").await.unwrap_err();
        assert!(!error.to_string().contains('\n'));
    }

    #[tokio::test]
    async fn get_transaction_preserves_unknown_state_wire_value_in_public_receipt() {
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "200 OK",
            transaction_response("tx-unknown", "STATE_WEIRD\nforged"),
        )])
        .await;
        let client = test_client(url);

        let receipt = client.get_transaction("tx-unknown").await.unwrap();

        match receipt.state {
            RelayerTransactionState::Unknown(raw) => {
                assert_eq!(raw, "STATE_WEIRD\nforged");
            }
            state => panic!("expected unknown state, got {state:?}"),
        }
        let _ = handle.await.unwrap();
    }

    #[tokio::test]
    async fn get_transaction_rejects_invalid_hash_and_oversized_success_body() {
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "200 OK",
            json!({
                "transactionID": "tx-bad-hash",
                "state": "STATE_CONFIRMED",
                "transactionHash": "bad\nhash"
            })
            .to_string(),
        )])
        .await;
        let client = test_client(url);

        let error = client.get_transaction("tx-bad-hash").await.unwrap_err();
        assert!(error_has_prefix(&error, RECONCILIATION_REQUIRED_PREFIX));
        assert!(!error.to_string().contains("bad\nhash"));
        let _ = handle.await.unwrap();

        let (url, handle) = spawn_server(vec![TestResponse::json(
            "200 OK",
            "x".repeat(MAX_SUCCESS_BODY_BYTES + 1),
        )])
        .await;
        let client = test_client(url);

        let error = client.get_wallet_nonce(address(WALLET_CREATE_OWNER)).await.unwrap_err();
        assert!(matches!(error, RelayerError::Other(message) if message.contains("maximum size")));
        let _ = handle.await.unwrap();

        let (url, handle) = spawn_server(vec![TestResponse::json_without_content_length(
            "200 OK",
            "x".repeat(MAX_SUCCESS_BODY_BYTES + 1),
        )])
        .await;
        let client = test_client(url);

        let error = client
            .get_wallet_nonce(address(WALLET_CREATE_OWNER))
            .await
            .unwrap_err();
        assert!(matches!(error, RelayerError::Other(message) if message.contains("maximum size")));
        let _ = handle.await.unwrap();
    }

    #[tokio::test]
    async fn get_wallet_nonce_rejects_malformed_success_payloads() {
        for body in ["not-json".to_string(), "{}".to_string(), json!({"nonce": "nan"}).to_string()]
        {
            let (url, handle) =
                spawn_server(vec![TestResponse::json("200 OK", body)]).await;
            let client = test_client(url);

            let error = client
                .get_wallet_nonce(address(WALLET_CREATE_OWNER))
                .await
                .unwrap_err();

            assert!(matches!(error, RelayerError::Other(_)));
            let requests = handle.await.unwrap();
            assert_eq!(requests.len(), 1);
            assert_eq!(
                requests[0].path,
                format!(
                    "/nonce?address={}&type=WALLET",
                    to_checksum(&address(WALLET_CREATE_OWNER), None)
                )
            );
        }
    }

    #[tokio::test]
    async fn get_transaction_rejects_oversized_array_response() {
        let body = (0..=MAX_TRANSACTION_RESPONSE_ITEMS)
            .map(|index| {
                json!({
                    "transactionID": format!("other-tx-{index}"),
                    "state": "STATE_CONFIRMED",
                    "transactionHash": "0x38cbfbeae8fffa4e2b187ee5978d3ee9cafc53af0363ed90a35b7ea9016535d8"
                })
            })
            .collect::<Vec<_>>();
        let (url, handle) =
            spawn_server(vec![TestResponse::json("200 OK", json!(body).to_string())]).await;
        let client = test_client(url);

        let error = client.get_transaction("tx-array").await.unwrap_err();
        assert!(error_has_prefix(&error, RECONCILIATION_REQUIRED_PREFIX));
        let _ = handle.await.unwrap();
    }

    #[tokio::test]
    async fn submit_terminal_states_clear_owner_block_or_return_terminal_error() {
        let owner = address(WALLET_CREATE_OWNER);

        for (transaction_id, state, expected_error) in [
            ("tx-confirmed-now", "STATE_CONFIRMED", None),
            ("tx-invalid-now", "STATE_INVALID", Some("invalid")),
            ("tx-failed-now", "STATE_FAILED", Some("failed")),
        ] {
            let (url, handle) = spawn_server(vec![
                TestResponse::json("200 OK", transaction_response(transaction_id, state)),
                TestResponse::json("200 OK", json!({"nonce": "34"}).to_string()),
            ])
            .await;
            let client = test_client(url);

            let result = client.submit_wallet_create(owner, mutation_permit()).await;
            match expected_error {
                None => {
                    let receipt = result.unwrap();
                    assert_eq!(receipt.transaction_id, transaction_id);
                    assert_eq!(receipt.state, RelayerTransactionState::Confirmed);
                }
                Some("invalid") => {
                    assert!(matches!(result.unwrap_err(), RelayerError::TransactionInvalid(_)));
                }
                Some("failed") => {
                    assert!(matches!(result.unwrap_err(), RelayerError::TransactionFailed(_)));
                }
                _ => unreachable!(),
            }

            let nonce = client.get_wallet_nonce(owner).await.unwrap();
            assert_eq!(nonce, U256::from(34u64));
            let requests = handle.await.unwrap();
            assert_eq!(requests.len(), 2);
            assert_eq!(requests[0].path, SUBMIT_PATH);
            assert!(requests[1].path.contains("/nonce?address="));
        }
    }

    #[tokio::test]
    async fn polling_treats_only_confirmed_as_success_and_uses_injected_sleeper() {
        let (result, requests, sleeper, _policy) = poll_sequence(
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
        let sleeps = sleeper.sleeps();
        assert_eq!(sleeps.len(), 3);
        assert!((Duration::from_millis(100)..=Duration::from_millis(125)).contains(&sleeps[0]));
        assert!((Duration::from_millis(200)..=Duration::from_millis(250)).contains(&sleeps[1]));
        assert!((Duration::from_millis(400)..=Duration::from_millis(500)).contains(&sleeps[2]));
        assert_ne!(sleeps[0], Duration::from_millis(100));

        let (result, _, _, _) = poll_sequence(&["STATE_INVALID"], 1).await;
        assert!(matches!(result.unwrap_err(), RelayerError::TransactionInvalid(_)));

        let (result, _, _, _) = poll_sequence(&["STATE_FAILED"], 1).await;
        assert!(matches!(result.unwrap_err(), RelayerError::TransactionFailed(_)));

        let (result, _, _, _) = poll_sequence(&["STATE_STRANGE"], 1).await;
        let error = result.unwrap_err();
        assert!(error_has_prefix(&error, RECONCILIATION_REQUIRED_PREFIX));

        let (result, requests, sleeper, _policy) =
            poll_sequence(&["STATE_NEW", "STATE_NEW"], 2).await;
        assert!(matches!(result.unwrap_err(), RelayerError::Timeout));
        assert_eq!(requests.len(), 2);
        let sleeps = sleeper.sleeps();
        assert_eq!(sleeps.len(), 1);
        assert!((Duration::from_millis(100)..=Duration::from_millis(125)).contains(&sleeps[0]));
    }

    #[tokio::test]
    async fn poll_policy_rejects_invalid_bounds_and_caps_backoff() {
        assert!(DepositWalletPollPolicy::new(0, Duration::from_secs(1)).is_err());
        assert!(DepositWalletPollPolicy::new(1, Duration::from_millis(99)).is_err());
        assert!(
            DepositWalletPollPolicy::new(MAX_POLL_ATTEMPTS + 1, Duration::from_secs(1)).is_err()
        );

        let policy = DepositWalletPollPolicy::new(1, Duration::from_secs(10)).unwrap();
        assert_eq!(policy.interval_for_attempt(4), MAX_POLL_INTERVAL);

        let invalid_literal = DepositWalletPollPolicy {
            max_attempts: 0,
            interval: Duration::from_secs(1),
        };
        let client =
            test_client(DepositWalletRelayerUrl::loopback("http://127.0.0.1:1").unwrap());
        let error = client
            .poll_transaction("tx-123", invalid_literal)
            .await
            .unwrap_err();
        assert!(matches!(error, RelayerError::Other(message) if message.contains("max attempts")));
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

        let policy = DepositWalletPollPolicy::new(2, Duration::from_millis(100)).unwrap();
        let error = client
            .poll_transaction("tx-pending", policy)
            .await
            .unwrap_err();
        assert!(matches!(error, RelayerError::Timeout));

        let blocked = client.get_wallet_nonce(owner).await.unwrap_err();
        assert!(error_has_prefix(&blocked, RECONCILIATION_REQUIRED_PREFIX));

        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 3);
    }

    #[tokio::test]
    async fn owner_aware_repoll_can_continue_known_ambiguous_transaction() {
        let owner = address(WALLET_CREATE_OWNER);
        let (url, handle) = spawn_server(vec![
            TestResponse::json("200 OK", transaction_response("tx-repoll", "STATE_NEW")),
            TestResponse::json("200 OK", transaction_response("tx-repoll", "STATE_NEW")),
            TestResponse::json("200 OK", transaction_response("tx-repoll", "STATE_NEW")),
            TestResponse::json("200 OK", transaction_response("tx-repoll", "STATE_CONFIRMED")),
            TestResponse::json("200 OK", json!({"nonce": "38"}).to_string()),
        ])
        .await;
        let client = test_client(url);

        let receipt = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap();
        assert_eq!(receipt.transaction_id, "tx-repoll");

        let timeout = client
            .poll_owner_transaction(
                owner,
                "tx-repoll",
                DepositWalletPollPolicy::new(1, Duration::from_millis(100)).unwrap(),
            )
            .await
            .unwrap_err();
        assert!(matches!(timeout, RelayerError::Timeout));
        assert!(client.ambiguous_submit_block(owner).is_some());

        let receipt = client
            .poll_owner_transaction(
                owner,
                "tx-repoll",
                DepositWalletPollPolicy::new(2, Duration::from_millis(100)).unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(receipt.state, RelayerTransactionState::Confirmed);

        let nonce = client.get_wallet_nonce(owner).await.unwrap();
        assert_eq!(nonce, U256::from(38u64));

        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 5);
    }

    #[tokio::test]
    async fn owner_aware_poll_rechecks_owner_evidence_before_each_state_mutation() {
        let owner = address(WALLET_CREATE_OWNER);
        let (url, handle) = spawn_server(vec![
            TestResponse::json("200 OK", transaction_response("tx-cleared", "STATE_NEW")),
            TestResponse::json("200 OK", transaction_response("tx-cleared", "STATE_NEW")),
            TestResponse::json("200 OK", transaction_response("tx-cleared", "STATE_NEW")),
            TestResponse::json("200 OK", json!({"nonce": "39"}).to_string()),
        ])
        .await;
        let sleeper = Arc::new(ClearingSleeper::default());
        let sleeper_trait: Arc<dyn DepositWalletSleeper> = sleeper.clone();
        let client = DepositWalletRelayerClient::from_parts(
            reqwest_client(Duration::from_secs(2)),
            url,
            relayer_auth(),
            deposit_wallet_contract_config(137).unwrap(),
            Arc::new(FixedClock { now: 1_700_000_000 }),
            sleeper_trait,
        );
        sleeper.attach_state(client.mutation_state.clone());

        let receipt = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap();
        assert_eq!(receipt.transaction_id, "tx-cleared");

        let error = client
            .poll_owner_transaction(
                owner,
                "tx-cleared",
                DepositWalletPollPolicy::new(2, Duration::from_millis(100)).unwrap(),
            )
            .await
            .unwrap_err();

        assert!(matches!(error, RelayerError::Timeout));
        assert!(client.ambiguous_submit_block(owner).is_none());
        client.ensure_owner_unblocked(owner).unwrap();
        let nonce = client.get_wallet_nonce(owner).await.unwrap();
        assert_eq!(nonce, U256::from(39u64));

        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 4);
    }

    #[tokio::test]
    async fn owner_aware_poll_without_local_evidence_does_not_block_owner() {
        let owner = address(WALLET_CREATE_OWNER);
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "200 OK",
            transaction_response("tx-recovered", "STATE_NEW"),
        )])
        .await;
        let client = test_client(url);

        let error = client
            .poll_owner_transaction(
                owner,
                "tx-recovered",
                DepositWalletPollPolicy::new(1, Duration::from_millis(100)).unwrap(),
            )
            .await
            .unwrap_err();
        assert!(matches!(error, RelayerError::Timeout));

        assert!(client.ambiguous_submit_block(owner).is_none());
        client.ensure_owner_unblocked(owner).unwrap();

        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].path, "/transaction?id=tx-recovered");
    }

    #[tokio::test]
    async fn owner_aware_poll_allows_existing_inflight_transaction_to_reach_terminal_state() {
        let owner = address(WALLET_CREATE_OWNER);
        let (url, handle) = spawn_server(vec![
            TestResponse::json("200 OK", transaction_response("tx-owner-aware", "STATE_NEW")),
            TestResponse::json("200 OK", transaction_response("tx-owner-aware", "STATE_NEW")),
            TestResponse::json(
                "200 OK",
                transaction_response("tx-owner-aware", "STATE_CONFIRMED"),
            ),
            TestResponse::json("200 OK", json!({"nonce": "35"}).to_string()),
        ])
        .await;
        let client = test_client(url);

        let receipt = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap();
        assert_eq!(receipt.transaction_id, "tx-owner-aware");

        let receipt = client
            .poll_owner_transaction(
                owner,
                "tx-owner-aware",
                DepositWalletPollPolicy::new(2, Duration::from_millis(100)).unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(receipt.state, RelayerTransactionState::Confirmed);

        let nonce = client.get_wallet_nonce(owner).await.unwrap();
        assert_eq!(nonce, U256::from(35u64));
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 4);
    }

    #[tokio::test]
    async fn owner_aware_poll_requires_response_owner_before_recording_block() {
        let owner = address(WALLET_CREATE_OWNER);
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "200 OK",
            json!({
                "transactionID": "tx-no-owner",
                "state": "STATE_NEW",
                "transactionHash": "0x38cbfbeae8fffa4e2b187ee5978d3ee9cafc53af0363ed90a35b7ea9016535d8"
            })
            .to_string(),
        )])
        .await;
        let client = test_client(url);

        let error = client
            .poll_owner_transaction(
                owner,
                "tx-no-owner",
                DepositWalletPollPolicy::new(1, Duration::from_millis(100)).unwrap(),
            )
            .await
            .unwrap_err();
        assert!(error_has_prefix(&error, RECONCILIATION_REQUIRED_PREFIX));
        client.ensure_owner_unblocked(owner).unwrap();
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);

        let other_owner = address("0x0000000000000000000000000000000000000001");
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "200 OK",
            json!({
                "transactionID": "tx-wrong-owner",
                "state": "STATE_NEW",
                "transactionHash": "0x38cbfbeae8fffa4e2b187ee5978d3ee9cafc53af0363ed90a35b7ea9016535d8",
                "owner": format!("{other_owner:?}")
            })
            .to_string(),
        )])
        .await;
        let client = test_client(url);

        let error = client
            .poll_owner_transaction(
                owner,
                "tx-wrong-owner",
                DepositWalletPollPolicy::new(1, Duration::from_millis(100)).unwrap(),
            )
            .await
            .unwrap_err();
        assert!(error_has_prefix(&error, RECONCILIATION_REQUIRED_PREFIX));
        client.ensure_owner_unblocked(owner).unwrap();
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
    }

    #[tokio::test]
    async fn owner_aware_poll_fetch_failure_without_evidence_does_not_block_owner() {
        let owner = address(WALLET_CREATE_OWNER);
        let (url, handle) = spawn_reset_server().await;
        let client = test_client(url);

        let error = client
            .poll_owner_transaction(
                owner,
                "tx-recovery-fetch-failed",
                DepositWalletPollPolicy::new(1, Duration::from_millis(100)).unwrap(),
            )
            .await
            .unwrap_err();

        assert!(matches!(error, RelayerError::Http(_)));
        assert!(client.ambiguous_submit_block(owner).is_none());
        client.ensure_owner_unblocked(owner).unwrap();
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].path, "/transaction?id=tx-recovery-fetch-failed");
    }

    #[tokio::test]
    async fn owner_aware_poll_fetch_failure_blocks_known_owner_transaction() {
        let owner = address(WALLET_CREATE_OWNER);
        let transaction_id = "tx-known-fetch-failed";
        let (url, handle) = spawn_reset_server().await;
        let client = test_client(url);
        {
            let payload_hash = recovered_payload_hash(transaction_id);
            let mut state = client.mutation_state().unwrap();
            state.owner_blocks.insert(
                owner,
                OwnerMutationBlock::InFlight {
                    payload_hash: payload_hash.clone(),
                    transaction_id: Some(transaction_id.to_string()),
                },
            );
            state.transaction_owners.insert(
                transaction_id.to_string(),
                OwnerTransactionRecord {
                    owner,
                    payload_hash,
                },
            );
        }

        let error = client
            .poll_owner_transaction(
                owner,
                transaction_id,
                DepositWalletPollPolicy::new(1, Duration::from_millis(100)).unwrap(),
            )
            .await
            .unwrap_err();

        assert!(matches!(error, RelayerError::Http(_)));
        assert!(client.ambiguous_submit_block(owner).is_some());
        let blocked = client.get_wallet_nonce(owner).await.unwrap_err();
        assert!(error_has_prefix(&blocked, RECONCILIATION_REQUIRED_PREFIX));
        client
            .clear_ambiguous_submit_after_manual_reconciliation(
                owner,
                DepositWalletMutationPermit::new(
                    owner,
                    "checked recovery fetch failure",
                    "single-process mocked owner serialization guard",
                ),
            )
            .unwrap();
        client.ensure_owner_unblocked(owner).unwrap();
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].path, "/transaction?id=tx-known-fetch-failed");
    }

    #[tokio::test]
    async fn owner_aware_poll_unknown_state_does_not_overwrite_existing_owner_block() {
        let owner = address(WALLET_CREATE_OWNER);
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "200 OK",
            transaction_response("tx-original", "STATE_NEW"),
        )])
        .await;
        let client = test_client(url);

        let receipt = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap();
        assert_eq!(receipt.transaction_id, "tx-original");

        let original_hash = {
            let state = client.mutation_state().unwrap();
            match state.owner_blocks.get(&owner) {
                Some(OwnerMutationBlock::InFlight {
                    payload_hash,
                    transaction_id: Some(transaction_id),
                }) => {
                    assert_eq!(transaction_id, "tx-original");
                    payload_hash.clone()
                }
                block => panic!("expected in-flight owner block, got {block:?}"),
            }
        };

        let error = client
            .poll_owner_transaction(
                owner,
                "tx-other",
                DepositWalletPollPolicy::new(1, Duration::from_millis(100)).unwrap(),
            )
            .await
            .unwrap_err();

        assert!(error_has_prefix(&error, RECONCILIATION_REQUIRED_PREFIX));
        {
            let state = client.mutation_state().unwrap();
            match state.owner_blocks.get(&owner) {
                Some(OwnerMutationBlock::InFlight {
                    payload_hash,
                    transaction_id: Some(transaction_id),
                }) => {
                    assert_eq!(transaction_id, "tx-original");
                    assert_eq!(payload_hash, &original_hash);
                }
                block => panic!("expected original in-flight owner block, got {block:?}"),
            }
            assert!(!state.transaction_owners.contains_key("tx-other"));
        }
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
    }

    #[tokio::test]
    async fn owner_aware_poll_unknown_state_without_local_evidence_does_not_block_owner() {
        let owner = address(WALLET_CREATE_OWNER);
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "200 OK",
            transaction_response("tx-no-local-evidence", "STATE_UNKNOWN_NEW"),
        )])
        .await;
        let client = test_client(url);

        let error = client
            .poll_owner_transaction(
                owner,
                "tx-no-local-evidence",
                DepositWalletPollPolicy::new(1, Duration::from_millis(100)).unwrap(),
            )
            .await
            .unwrap_err();

        assert!(error_has_prefix(&error, RECONCILIATION_REQUIRED_PREFIX));
        assert!(client.ambiguous_submit_block(owner).is_none());
        client.ensure_owner_unblocked(owner).unwrap();
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
    }

    #[tokio::test]
    async fn owner_aware_poll_unknown_state_records_reconciliation_block_after_owner_evidence() {
        let owner = address(WALLET_CREATE_OWNER);
        let transaction_id = "tx-unknown-owner";
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "200 OK",
            transaction_response(transaction_id, "STATE_UNKNOWN_NEW"),
        )])
        .await;
        let client = test_client(url);
        {
            let payload_hash = recovered_payload_hash(transaction_id);
            let mut state = client.mutation_state().unwrap();
            state.owner_blocks.insert(
                owner,
                OwnerMutationBlock::InFlight {
                    payload_hash: payload_hash.clone(),
                    transaction_id: Some(transaction_id.to_string()),
                },
            );
            state.transaction_owners.insert(
                transaction_id.to_string(),
                OwnerTransactionRecord {
                    owner,
                    payload_hash,
                },
            );
        }

        let error = client
            .poll_owner_transaction(
                owner,
                transaction_id,
                DepositWalletPollPolicy::new(1, Duration::from_millis(100)).unwrap(),
            )
            .await
            .unwrap_err();

        assert!(error_has_prefix(&error, RECONCILIATION_REQUIRED_PREFIX));
        assert!(client.ambiguous_submit_block(owner).is_some());
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
    }

    #[tokio::test]
    async fn local_transaction_poll_requires_response_owner_before_clearing_block() {
        let owner = address(WALLET_CREATE_OWNER);
        let other_owner = address("0x0000000000000000000000000000000000000001");
        let (url, handle) = spawn_server(vec![
            TestResponse::json("200 OK", transaction_response("tx-owner-mismatch", "STATE_NEW")),
            TestResponse::json(
                "200 OK",
                json!({
                    "transactionID": "tx-owner-mismatch",
                    "state": "STATE_CONFIRMED",
                    "transactionHash": "0x38cbfbeae8fffa4e2b187ee5978d3ee9cafc53af0363ed90a35b7ea9016535d8",
                    "owner": format!("{other_owner:?}")
                })
                .to_string(),
            ),
        ])
        .await;
        let client = test_client(url);

        let receipt = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap();
        assert_eq!(receipt.transaction_id, "tx-owner-mismatch");

        let error = client
            .poll_transaction(
                "tx-owner-mismatch",
                DepositWalletPollPolicy::new(1, Duration::from_millis(100)).unwrap(),
            )
            .await
            .unwrap_err();

        assert!(error_has_prefix(&error, RECONCILIATION_REQUIRED_PREFIX));
        assert!(client.ambiguous_submit_block(owner).is_some());
        let blocked = client.get_wallet_nonce(owner).await.unwrap_err();
        assert!(error_has_prefix(&blocked, RECONCILIATION_REQUIRED_PREFIX));
        client
            .clear_ambiguous_submit_after_manual_reconciliation(
                owner,
                DepositWalletMutationPermit::new(
                    owner,
                    "checked mocked owner mismatch",
                    "single-process mocked owner serialization guard",
                ),
            )
            .unwrap();
        client.ensure_owner_unblocked(owner).unwrap();
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 2);
    }

    #[tokio::test]
    async fn local_transaction_poll_parse_error_marks_inflight_block_reconciliation_required() {
        let owner = address(WALLET_CREATE_OWNER);
        let (url, handle) = spawn_server(vec![
            TestResponse::json("200 OK", transaction_response("tx-malformed", "STATE_NEW")),
            TestResponse::json("200 OK", "{\"transactionID\":\"tx-malformed\""),
        ])
        .await;
        let client = test_client(url);

        let receipt = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap();
        assert_eq!(receipt.transaction_id, "tx-malformed");

        let error = client
            .poll_transaction(
                "tx-malformed",
                DepositWalletPollPolicy::new(1, Duration::from_millis(100)).unwrap(),
            )
            .await
            .unwrap_err();

        assert!(matches!(error, RelayerError::Other(_)));
        assert!(client.ambiguous_submit_block(owner).is_some());
        let blocked = client.get_wallet_nonce(owner).await.unwrap_err();
        assert!(error_has_prefix(&blocked, RECONCILIATION_REQUIRED_PREFIX));
        client
            .clear_ambiguous_submit_after_manual_reconciliation(
                owner,
                DepositWalletMutationPermit::new(
                    owner,
                    "checked malformed transaction response",
                    "single-process mocked owner serialization guard",
                ),
            )
            .unwrap();
        client.ensure_owner_unblocked(owner).unwrap();
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 2);
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
        assert!(error_has_prefix(&blocked, RECONCILIATION_REQUIRED_PREFIX));

        let receipt = client
            .poll_transaction(
                "tx-confirmed",
                DepositWalletPollPolicy::new(1, Duration::from_millis(100)).unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(receipt.state, RelayerTransactionState::Confirmed);

        let nonce = client.get_wallet_nonce(owner).await.unwrap();
        assert_eq!(nonce, U256::from(33u64));

        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 3);
    }

    #[tokio::test]
    async fn terminal_error_poll_clears_owner_inflight_block() {
        let owner = address(WALLET_CREATE_OWNER);

        for (transaction_id, terminal_state, expected_error) in [
            (
                "tx-terminal-invalid",
                "STATE_INVALID",
                "transaction invalid",
            ),
            ("tx-terminal-failed", "STATE_FAILED", "transaction failed"),
        ] {
            let (url, handle) = spawn_server(vec![
                TestResponse::json("200 OK", transaction_response(transaction_id, "STATE_NEW")),
                TestResponse::json("200 OK", transaction_response(transaction_id, terminal_state)),
                TestResponse::json("200 OK", json!({"nonce": "37"}).to_string()),
            ])
            .await;
            let client = test_client(url);

            let receipt = client
                .submit_wallet_create(owner, mutation_permit())
                .await
                .unwrap();
            assert_eq!(receipt.transaction_id, transaction_id);
            assert!(client.get_wallet_nonce(owner).await.is_err());

            let error = client
                .poll_transaction(
                    transaction_id,
                    DepositWalletPollPolicy::new(1, Duration::from_millis(100)).unwrap(),
                )
                .await
                .unwrap_err();
            assert!(
                matches!(
                    (&expected_error, &error),
                    (&"transaction invalid", RelayerError::TransactionInvalid(_))
                        | (&"transaction failed", RelayerError::TransactionFailed(_))
                ),
                "expected {expected_error}, got {error:?}"
            );

            client.ensure_owner_unblocked(owner).unwrap();
            {
                let state = client.mutation_state().unwrap();
                assert!(!state.transaction_owners.contains_key(transaction_id));
            }
            let nonce = client.get_wallet_nonce(owner).await.unwrap();
            assert_eq!(nonce, U256::from(37u64));

            let requests = handle.await.unwrap();
            assert_eq!(requests.len(), 3);
        }
    }

    #[test]
    fn manual_clear_removes_only_matching_owner_transaction_records() {
        let owner = address(WALLET_CREATE_OWNER);
        let other_owner = address("0x0000000000000000000000000000000000000001");
        let client =
            test_client(DepositWalletRelayerUrl::loopback("http://127.0.0.1:1").unwrap());
        {
            let mut state = client.mutation_state().unwrap();
            state.owner_blocks.insert(
                owner,
                OwnerMutationBlock::Ambiguous {
                    payload_hash: "payload-owner".to_string(),
                },
            );
            state.transaction_owners.insert(
                "tx-owner-stale".to_string(),
                OwnerTransactionRecord {
                    owner,
                    payload_hash: "payload-owner".to_string(),
                },
            );
            state.transaction_owners.insert(
                "tx-other-live".to_string(),
                OwnerTransactionRecord {
                    owner: other_owner,
                    payload_hash: "payload-other".to_string(),
                },
            );
        }

        client
            .clear_ambiguous_submit_after_manual_reconciliation(
                owner,
                DepositWalletMutationPermit::new(
                    owner,
                    "checked mocked owner transaction map cleanup",
                    "single-process mocked owner serialization guard",
                ),
            )
            .unwrap();

        client.ensure_owner_unblocked(owner).unwrap();
        let state = client.mutation_state().unwrap();
        assert!(!state.transaction_owners.contains_key("tx-owner-stale"));
        assert_eq!(
            state
                .transaction_owners
                .get("tx-other-live")
                .map(|record| record.owner),
            Some(other_owner)
        );
    }

    #[tokio::test]
    async fn manual_clear_rejects_known_inflight_submit_until_terminal_poll() {
        let owner = address(WALLET_CREATE_OWNER);
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "200 OK",
            transaction_response("tx-known", "STATE_NEW"),
        )])
        .await;
        let client = test_client(url);

        let receipt = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap();
        assert_eq!(receipt.transaction_id, "tx-known");

        let error = client
            .clear_ambiguous_submit_after_manual_reconciliation(
                owner,
                DepositWalletMutationPermit::new(
                    owner,
                    "checked mocked relayer state",
                    "single-process mocked owner serialization guard",
                ),
            )
            .unwrap_err();
        assert!(error_has_prefix(&error, MUTATION_BLOCKED_PREFIX));

        let duplicate = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap_err();
        assert!(error_has_prefix(&duplicate, RECONCILIATION_REQUIRED_PREFIX));

        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
    }
}
