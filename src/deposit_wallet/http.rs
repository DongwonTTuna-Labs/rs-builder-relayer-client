use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};
#[cfg(test)]
use tokio::sync::OwnedSemaphorePermit;

use ethers::types::{Address, H256, U256};
use ethers::utils::{keccak256, to_checksum};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, CONTENT_TYPE, RETRY_AFTER};
use reqwest::{Client, Method, StatusCode};
use secrecy::{ExposeSecret, SecretString};
use serde::de::{self, SeqAccess, Visitor};
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
const MIN_POLL_INTERVAL: Duration = Duration::from_millis(100);
const MAX_POLL_INTERVAL: Duration = Duration::from_secs(30);
const MAX_POLL_ATTEMPTS: usize = 120;
const MAX_OWNER_MUTATION_RECORDS: usize = 1024;

#[derive(Clone, PartialEq, Eq)]
pub struct DepositWalletRelayerUrl {
    base: Url,
    kind: DepositWalletRelayerUrlKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DepositWalletRelayerUrlKind {
    Production,
    #[cfg(test)]
    MockLoopback,
}

impl DepositWalletRelayerUrl {
    /// Builds a production relayer URL.
    ///
    /// This PR keeps live submit mutations disabled for production URLs. The
    /// loopback submit transport is intentionally limited to crate-local tests
    /// until a later live-gate PR adds approved external integration hooks.
    pub fn parse(raw: &str) -> Result<Self> {
        let url = Url::parse(raw)
            .map_err(|e| RelayerError::invalid_relayer_url(format!("could not parse URL: {e}")))?;
        validate_rel_url(&url)?;
        Ok(Self {
            base: url,
            kind: DepositWalletRelayerUrlKind::Production,
        })
    }

    fn endpoint(&self, path: &str) -> Url {
        let mut url = self.base.clone();
        url.set_path(path);
        url.set_query(None);
        url
    }

    fn is_production_host(&self) -> bool {
        self.kind == DepositWalletRelayerUrlKind::Production
    }

    #[cfg(test)]
    fn loopback(raw: &str) -> Result<Self> {
        let url = Url::parse(raw)
            .map_err(|e| RelayerError::invalid_relayer_url(format!("could not parse URL: {e}")))?;
        validate_mock_loopback_url(&url)?;
        Ok(Self {
            base: url,
            kind: DepositWalletRelayerUrlKind::MockLoopback,
        })
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
        headers.insert(relayer_api_key_header()?, api_key);

        let mut api_key_address =
            HeaderValue::from_str(&to_checksum(&self.api_key_address, None)).map_err(|_| {
                RelayerError::AuthError("invalid relayer API key address".to_string())
            })?;
        api_key_address.set_sensitive(true);
        headers.insert(relayer_api_key_address_header()?, api_key_address);
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

fn relayer_api_key_header() -> Result<HeaderName> {
    HeaderName::from_bytes(b"RELAYER_API_KEY")
        .map_err(|_| RelayerError::AuthError("invalid relayer API key header name".to_string()))
}

fn relayer_api_key_address_header() -> Result<HeaderName> {
    HeaderName::from_bytes(b"RELAYER_API_KEY_ADDRESS").map_err(|_| {
        RelayerError::AuthError("invalid relayer API key address header name".to_string())
    })
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
    owner_serialization_evidence: DepositWalletOwnerSerializationEvidence,
}

impl DepositWalletMutationPermit {
    /// Creates an explicit owner-scoped live mutation permit.
    ///
    /// The evidence must come from a caller-side owner lock, nonce lease, or
    /// actor queue that prevents concurrent WALLET-CREATE/WALLET submits for
    /// the same owner. The crate validates the evidence shape and expiry before
    /// request construction; the caller remains responsible for enforcing the
    /// referenced guard in its runtime.
    pub fn from_owner_serialization_evidence(
        reason: impl Into<String>,
        owner_serialization_evidence: DepositWalletOwnerSerializationEvidence,
    ) -> Result<Self> {
        let reason = reason.into();
        if reason.trim().is_empty() {
            return Err(RelayerError::mutation_blocked(
                "explicit deposit-wallet mutation permit reason required".to_string(),
            ));
        }
        Ok(Self {
            owner: owner_serialization_evidence.owner,
            reason,
            owner_serialization_evidence,
        })
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

#[derive(Clone, PartialEq, Eq)]
pub struct DepositWalletOwnerSerializationEvidence {
    owner: Address,
    issuer: String,
    lease_id_hash: String,
    acquired_at_unix_seconds: u64,
    expires_at_unix_seconds: u64,
}

impl DepositWalletOwnerSerializationEvidence {
    /// Records caller-side proof that same-owner submit work is serialized.
    ///
    /// `lease_id` is hashed before storage so debug output and errors never
    /// expose raw lock keys, queue ids, or database lease identifiers.
    pub fn new(
        owner: Address,
        issuer: impl Into<String>,
        lease_id: impl AsRef<[u8]>,
        acquired_at_unix_seconds: u64,
        expires_at_unix_seconds: u64,
    ) -> Result<Self> {
        let issuer = issuer.into();
        if issuer.trim().is_empty() {
            return Err(RelayerError::mutation_blocked(
                "owner serialization evidence issuer required".to_string(),
            ));
        }
        let lease_id = lease_id.as_ref();
        if lease_id.is_empty() {
            return Err(RelayerError::mutation_blocked(
                "owner serialization evidence lease id required".to_string(),
            ));
        }
        if expires_at_unix_seconds <= acquired_at_unix_seconds {
            return Err(RelayerError::mutation_blocked(
                "owner serialization evidence must expire after acquisition".to_string(),
            ));
        }
        Ok(Self {
            owner,
            issuer,
            lease_id_hash: payload_hash_summary(lease_id),
            acquired_at_unix_seconds,
            expires_at_unix_seconds,
        })
    }

    pub fn owner(&self) -> Address {
        self.owner
    }

    pub fn expires_at_unix_seconds(&self) -> u64 {
        self.expires_at_unix_seconds
    }
}

impl fmt::Debug for DepositWalletOwnerSerializationEvidence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DepositWalletOwnerSerializationEvidence")
            .field("owner", &redacted_address(self.owner))
            .field("issuer", &"<redacted>")
            .field("lease_id_hash", &display_payload_hash(&self.lease_id_hash))
            .field("acquired_at_unix_seconds", &self.acquired_at_unix_seconds)
            .field("expires_at_unix_seconds", &self.expires_at_unix_seconds)
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
    pub owner: Option<Address>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ParsedTransactionReceipt {
    receipt: DepositWalletTransactionReceipt,
    owner: Option<Address>,
}

#[derive(Debug)]
struct ResponseError {
    error: RelayerError,
    retry_after: Option<Duration>,
}

impl ResponseError {
    fn new(error: RelayerError, retry_after: Option<Duration>) -> Self {
        Self { error, retry_after }
    }
}

#[derive(Debug)]
struct PollFetchError {
    error: RelayerError,
    retry_after: Option<Duration>,
    owner: Option<Address>,
    trusted_recovery_owner_block: bool,
}

impl PollFetchError {
    fn from_response_error(error: ResponseError) -> Self {
        let trusted_recovery_owner_block = is_trusted_recovery_fetch_failure(&error.error);
        Self {
            error: error.error,
            retry_after: error.retry_after,
            owner: None,
            trusted_recovery_owner_block,
        }
    }

    fn from_transaction_parse_error(error: TransactionParseError) -> Self {
        Self {
            error: error.error,
            retry_after: None,
            owner: error.owner,
            trusted_recovery_owner_block: false,
        }
    }
}

#[derive(Debug)]
struct TransactionParseError {
    error: RelayerError,
    owner: Option<Address>,
}

impl TransactionParseError {
    fn new(error: RelayerError, owner: Option<Address>) -> Self {
        Self { error, owner }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RelayerTransactionResponseWithOwner {
    #[serde(flatten)]
    response: RelayerSubmitResponse,
    // Official GET /transaction responses include owner as the owner address.
    // Keep it internal because the public receipt intentionally exposes only
    // non-sensitive polling identifiers.
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
    error_body_drain_limiter: ErrorBodyDrainLimiter,
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
            error_body_drain_limiter: ErrorBodyDrainLimiter::new(MAX_BACKGROUND_ERROR_BODY_DRAINS),
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
        self.ensure_permitted(&gate, owner)?;
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
        self.ensure_permitted(&gate, owner)?;
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
        let response = self
            .send_with_success_limit(Method::GET, url, None, MAX_TRANSACTION_SUCCESS_BODY_BYTES)
            .await?;
        parse_transaction_response(&transaction_id, &response)
            .map_err(|parse_error| parse_error.error)
    }

    async fn fetch_transaction_for_poll(
        &self,
        transaction_id: &str,
    ) -> std::result::Result<ParsedTransactionReceipt, PollFetchError> {
        let mut url = self.base_url.endpoint(TRANSACTION_PATH);
        url.query_pairs_mut().append_pair("id", transaction_id);
        let response = self
            .send_with_success_limit_and_retry_after(
                Method::GET,
                url,
                None,
                MAX_TRANSACTION_SUCCESS_BODY_BYTES,
            )
            .await
            .map_err(PollFetchError::from_response_error)?;
        parse_transaction_response(transaction_id, &response)
            .map_err(PollFetchError::from_transaction_parse_error)
    }

    pub async fn poll_transaction(
        &self,
        transaction_id: &str,
        policy: DepositWalletPollPolicy,
    ) -> Result<DepositWalletTransactionReceipt> {
        policy.validate()?;
        let transaction_id = validate_transaction_id(transaction_id)?;
        self.poll_validated_transaction(transaction_id, policy, None, false)
            .await
    }

    pub async fn poll_owner_transaction(
        &self,
        owner: Address,
        transaction_id: &str,
        policy: DepositWalletPollPolicy,
    ) -> Result<DepositWalletTransactionReceipt> {
        policy.validate()?;
        let transaction_id = validate_transaction_id(transaction_id)?;
        if !self.has_recovery_owner_evidence(owner, &transaction_id)? {
            return Err(RelayerError::mutation_blocked(
                "owner-scoped recovery polling requires local transaction evidence or explicit mutation permit"
                    .to_string(),
            ));
        }
        self.poll_validated_transaction(transaction_id, policy, Some(owner), false)
            .await
    }

    pub async fn poll_owner_transaction_with_reconciliation_permit(
        &self,
        owner: Address,
        transaction_id: &str,
        policy: DepositWalletPollPolicy,
        gate: DepositWalletMutationGate,
    ) -> Result<DepositWalletTransactionReceipt> {
        policy.validate()?;
        self.ensure_permitted(&gate, owner)?;
        let transaction_id = validate_transaction_id(transaction_id)?;
        self.poll_validated_transaction(transaction_id, policy, Some(owner), true)
            .await
    }

    async fn poll_validated_transaction(
        &self,
        transaction_id: String,
        policy: DepositWalletPollPolicy,
        expected_owner: Option<Address>,
        trusted_owner_recovery: bool,
    ) -> Result<DepositWalletTransactionReceipt> {
        let transaction_id_for_error = sanitized_external_token(&transaction_id);
        if let Some(owner) = expected_owner {
            if trusted_owner_recovery {
                self.record_recovered_inflight_transaction(owner, &transaction_id)?;
            } else {
                let _ = self.has_recovery_owner_evidence(owner, &transaction_id)?;
            }
        }

        for attempt in 0..policy.max_attempts {
            let parsed = match self.fetch_transaction_for_poll(&transaction_id).await {
                Ok(parsed) => parsed,
                Err(poll_error) => {
                    if is_transient_poll_error(&poll_error.error) && attempt + 1 < policy.max_attempts {
                        let policy_interval =
                            policy.interval_for_transaction_attempt(&transaction_id, attempt);
                        let sleep_for = poll_error
                            .retry_after
                            .map(|retry_after| {
                                retry_after.min(MAX_POLL_INTERVAL).max(policy_interval)
                            })
                            .unwrap_or(policy_interval);
                        self.sleeper
                            .sleep(sleep_for)
                            .await;
                        continue;
                    }
                    let response_owner = poll_error.owner;
                    let trusted_recovery_owner_block =
                        trusted_owner_recovery && poll_error.trusted_recovery_owner_block;
                    let error = poll_error.error;
                    if let Some(owner) = expected_owner {
                        if response_owner == Some(owner)
                            || trusted_recovery_owner_block
                            || self.has_recovery_owner_evidence(owner, &transaction_id)?
                        {
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
            let terminal_evidence = owner_to_verify
                .map(|owner| {
                    self.current_recovery_payload_record(&transaction_id, owner)
                        .map(|record| record.map(|record| (owner, record)))
                })
                .transpose()?
                .flatten();
            match &receipt.state {
                RelayerTransactionState::Confirmed => {
                    if receipt.transaction_hash.is_none() {
                        if let Some(owner) = owner_to_verify {
                            self.record_recovered_ambiguous_transaction(owner, &transaction_id)?;
                        } else {
                            self.mark_transaction_reconciliation_required(&transaction_id)?;
                        }
                        return Err(RelayerError::reconciliation_required(format!(
                            "confirmed deposit wallet transaction {} did not include transactionHash; manual reconciliation required",
                            transaction_id_for_error
                        )));
                    }
                    if let Some((owner, record)) = terminal_evidence {
                        if record.source == OwnerTransactionSource::OwnerRecovery {
                            self.record_recovered_ambiguous_transaction(owner, &transaction_id)?;
                            return Err(RelayerError::reconciliation_required(format!(
                                "confirmed owner-scoped recovery transaction {} did not prove the ambiguous submit payload; manual reconciliation required",
                                transaction_id_for_error
                            )));
                        }
                        self.clear_transaction_block_if_current(
                            &transaction_id,
                            owner,
                            &record.payload_hash,
                        )?;
                    }
                    return Ok(receipt);
                }
                RelayerTransactionState::Invalid => {
                    if let Some((owner, record)) = terminal_evidence {
                        if record.source == OwnerTransactionSource::OwnerRecovery {
                            self.record_recovered_ambiguous_transaction(owner, &transaction_id)?;
                        } else {
                            self.clear_transaction_block_if_current(
                                &transaction_id,
                                owner,
                                &record.payload_hash,
                            )?;
                        }
                    }
                    return Err(RelayerError::TransactionInvalid(format!(
                        "deposit wallet transaction {} invalid",
                        transaction_id_for_error
                    )));
                }
                RelayerTransactionState::Failed => {
                    if let Some((owner, record)) = terminal_evidence {
                        if record.source == OwnerTransactionSource::OwnerRecovery {
                            self.record_recovered_ambiguous_transaction(owner, &transaction_id)?;
                        } else {
                            self.clear_transaction_block_if_current(
                                &transaction_id,
                                owner,
                                &record.payload_hash,
                            )?;
                        }
                    }
                    return Err(RelayerError::TransactionFailed(format!(
                        "deposit wallet transaction {} failed",
                        transaction_id_for_error
                    )));
                }
                RelayerTransactionState::Unknown(raw) => {
                    match owner_to_verify {
                        Some(owner) => {
                            self.record_recovered_ambiguous_transaction(owner, &transaction_id)?
                        }
                        _ => self.mark_transaction_reconciliation_required(&transaction_id)?,
                    }
                    return Err(RelayerError::reconciliation_required(format!(
                        "deposit wallet transaction {} reached unknown state {}",
                        transaction_id_for_error,
                        unknown_state_error_summary(raw)
                    )));
                }
                RelayerTransactionState::New
                | RelayerTransactionState::Executed
                | RelayerTransactionState::Mined => {
                    if let Some(owner) = expected_owner {
                        self.record_recovered_inflight_transaction(owner, &transaction_id)?;
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
        self.ensure_permitted(&DepositWalletMutationGate::Permit(permit), owner)?;

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
                    let terminal_failure =
                        matches!(&receipt.state, RelayerTransactionState::Invalid | RelayerTransactionState::Failed);
                    let result = self.handle_submit_receipt(owner, payload_hash, receipt);
                    if result.is_ok()
                        || (terminal_failure
                            && matches!(
                                &result,
                                Err(RelayerError::TransactionInvalid(_))
                                    | Err(RelayerError::TransactionFailed(_))
                            ))
                    {
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
            }) => {
                reservation.disarm();
                self.record_ambiguous(owner, payload_hash.clone())?;
                Err(RelayerError::ambiguous_submit(format!(
                    "submit returned HTTP status {} after POST for owner {} payload {}; manual reconciliation required",
                    status,
                    redacted_address(owner),
                    display_payload_hash(&payload_hash)
                )))
            }
            Err(RelayerError::QuotaExhausted) => {
                reservation.disarm();
                self.record_ambiguous(owner, payload_hash.clone())?;
                Err(RelayerError::ambiguous_submit(format!(
                    "submit returned HTTP status 429 after POST for owner {} payload {}; manual reconciliation required",
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
        self.send_with_success_limit(method, url, body, MAX_SUCCESS_BODY_BYTES)
            .await
    }

    async fn send_with_success_limit(
        &self,
        method: Method,
        url: Url,
        body: Option<String>,
        success_body_limit: usize,
    ) -> Result<Vec<u8>> {
        self.send_with_success_limit_and_retry_after(method, url, body, success_body_limit)
            .await
            .map_err(|error| error.error)
    }

    async fn send_with_success_limit_and_retry_after(
        &self,
        method: Method,
        url: Url,
        body: Option<String>,
        success_body_limit: usize,
    ) -> std::result::Result<Vec<u8>, ResponseError> {
        let mut headers = self
            .auth
            .headers()
            .map_err(|error| ResponseError::new(error, None))?;
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
            .map_err(|error| ResponseError::new(RelayerError::Http(error.without_url()), None))?;
        if !response.status().is_success() {
            let status = response.status();
            let retry_after = retry_after_duration(response.headers());
            let retry_after_message = retry_after_summary_from_duration(retry_after);
            self.error_body_drain_limiter
                .try_spawn_error_response_body_drain(response);
            if status == StatusCode::TOO_MANY_REQUESTS {
                return Err(ResponseError::new(RelayerError::QuotaExhausted, retry_after));
            }
            return Err(ResponseError::new(
                RelayerError::Api {
                    status: status.as_u16(),
                    message: format!(
                        "deposit-wallet relayer request failed with HTTP {status}{retry_after_message}"
                    ),
                },
                retry_after,
            ));
        }

        read_limited_response_body(response, success_body_limit).await
            .map_err(|error| ResponseError::new(error, None))
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
        if state.transaction_owners.len() >= MAX_OWNER_MUTATION_RECORDS {
            return Err(RelayerError::mutation_blocked(format!(
                "owner mutation state already tracks {MAX_OWNER_MUTATION_RECORDS} transactions; reconcile terminal transactions before accepting another submit"
            )));
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
                    unknown_state_error_summary(raw)
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
                self.record_ambiguous(owner, payload_hash.clone())?;
                self.record_transaction_owner(&receipt.transaction_id, owner, payload_hash)?;
                Err(RelayerError::reconciliation_required(format!(
                    "deposit wallet submit transaction {} returned terminal state before transaction polling; manual reconciliation required",
                    sanitized_external_token(&receipt.transaction_id)
                )))
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
                source: OwnerTransactionSource::LocalSubmit,
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
                source: OwnerTransactionSource::LocalSubmit,
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
        let mut source = OwnerTransactionSource::OwnerRecovery;
        let payload_hash = if let Some(block) = state.owner_blocks.get(&owner) {
            match block {
                OwnerMutationBlock::InFlight {
                    transaction_id: Some(existing_transaction_id),
                    ..
                } if existing_transaction_id == transaction_id => {
                    return Ok(());
                }
                OwnerMutationBlock::Ambiguous { payload_hash } => {
                    if let Some(record) = state.transaction_owners.get(transaction_id) {
                        source = record.source;
                    }
                    payload_hash.clone()
                }
                _ => return Err(owner_block_error(owner, block)),
            }
        } else {
            recovered_payload_hash(transaction_id)
        };
        if !state.owner_blocks.contains_key(&owner) {
            ensure_owner_mutation_capacity(&state, owner, Some(transaction_id))?;
        }
        ensure_transaction_owner_mapping_available(&state, transaction_id, owner, &payload_hash)?;
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
                source,
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
        let current_record = current_recovery_payload_record(&state, transaction_id, owner)?;
        let mut source = current_record
            .as_ref()
            .map(|record| record.source)
            .unwrap_or(OwnerTransactionSource::OwnerRecovery);
        let current_payload_hash = current_record
            .map(|record| record.payload_hash)
            .unwrap_or_else(|| recovered_payload_hash(transaction_id));
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
            Some(OwnerMutationBlock::Ambiguous { payload_hash }) => {
                source = OwnerTransactionSource::OwnerRecovery;
                payload_hash.clone()
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
                source,
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

    fn current_recovery_payload_record(
        &self,
        transaction_id: &str,
        owner: Address,
    ) -> Result<Option<OwnerTransactionRecord>> {
        let state = self.mutation_state()?;
        current_recovery_payload_record(&state, transaction_id, owner)
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

    fn clear_transaction_block_if_current(
        &self,
        transaction_id: &str,
        owner: Address,
        payload_hash: &str,
    ) -> Result<()> {
        let mut state = self.mutation_state()?;
        if state.transaction_owners.get(transaction_id).is_some_and(|record| {
            record.owner == owner && record.payload_hash == payload_hash
        }) {
            state.transaction_owners.remove(transaction_id);
            clear_owner_block_if_payload(&mut state, owner, payload_hash);
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

    fn ensure_permitted(&self, gate: &DepositWalletMutationGate, owner: Address) -> Result<()> {
        match gate {
            DepositWalletMutationGate::Permit(permit) => {
                validate_permit_owner(permit, owner)?;
                validate_permit_fresh(permit, self.clock.now_unix_seconds())
            }
            DepositWalletMutationGate::Deny => Err(RelayerError::mutation_blocked(
                "explicit deposit-wallet mutation permit required".to_string(),
            )),
        }
    }

    #[cfg(test)]
    fn hold_error_body_drain_permits_for_test(&self) -> Vec<OwnedSemaphorePermit> {
        self.error_body_drain_limiter.hold_all_permits_for_test()
    }

    #[cfg(test)]
    fn dropped_error_body_drains_for_test(&self) -> usize {
        self.error_body_drain_limiter.dropped_for_test()
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
    source: OwnerTransactionSource,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OwnerTransactionSource {
    LocalSubmit,
    OwnerRecovery,
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

#[derive(Clone)]
struct ErrorBodyDrainLimiter {
    semaphore: Arc<tokio::sync::Semaphore>,
    #[cfg(test)]
    dropped: Arc<AtomicUsize>,
}

impl ErrorBodyDrainLimiter {
    fn new(limit: usize) -> Self {
        Self {
            semaphore: Arc::new(tokio::sync::Semaphore::new(limit)),
            #[cfg(test)]
            dropped: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn try_spawn_error_response_body_drain(&self, response: reqwest::Response) {
        let Ok(permit) = self.semaphore.clone().try_acquire_owned() else {
            #[cfg(test)]
            self.dropped.fetch_add(1, Ordering::SeqCst);
            return;
        };
        tokio::spawn(async move {
            let _permit = permit;
            drain_error_response_body(response).await;
        });
    }

    #[cfg(test)]
    fn hold_all_permits_for_test(&self) -> Vec<OwnedSemaphorePermit> {
        (0..MAX_BACKGROUND_ERROR_BODY_DRAINS)
            .map(|_| {
                self.semaphore
                    .clone()
                    .try_acquire_owned()
                    .expect("test should be able to hold all drain permits")
            })
            .collect()
    }

    #[cfg(test)]
    fn dropped_for_test(&self) -> usize {
        self.dropped.load(Ordering::SeqCst)
    }
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
    if !matches!(url.port(), None | Some(443)) {
        return Err(RelayerError::invalid_relayer_url(
            "relayer URL must use the default HTTPS port".to_string(),
        ));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(RelayerError::invalid_relayer_url(
            "relayer URL must not include query or fragment".to_string(),
        ));
    }
    if url.path() != "/" {
        return Err(RelayerError::invalid_relayer_url(
            "relayer URL must not include a path".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
fn validate_mock_loopback_url(url: &Url) -> Result<()> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(RelayerError::invalid_relayer_url(
            "mock relayer URL must use http or https".to_string(),
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(RelayerError::invalid_relayer_url(
            "mock relayer URL must not include userinfo".to_string(),
        ));
    }
    if !matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "::1" | "[::1]")) {
        return Err(RelayerError::invalid_relayer_url(
            "mock relayer URL must be loopback-only".to_string(),
        ));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(RelayerError::invalid_relayer_url(
            "mock relayer URL must not include query or fragment".to_string(),
        ));
    }
    if url.path() != "/" {
        return Err(RelayerError::invalid_relayer_url(
            "mock relayer URL must not include a path".to_string(),
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

fn validate_permit_owner(permit: &DepositWalletMutationPermit, owner: Address) -> Result<()> {
    if permit.owner != permit.owner_serialization_evidence.owner {
        return Err(RelayerError::mutation_blocked(
            "deposit-wallet mutation permit owner does not match owner serialization evidence"
                .to_string(),
        ));
    }
    if permit.owner != owner {
        return Err(RelayerError::mutation_blocked(format!(
            "deposit-wallet mutation permit owner {} does not match request owner {}",
            redacted_address(permit.owner),
            redacted_address(owner)
        )));
    }
    Ok(())
}

fn validate_permit_fresh(permit: &DepositWalletMutationPermit, now_unix_seconds: u64) -> Result<()> {
    if permit.reason.trim().is_empty() {
        return Err(RelayerError::mutation_blocked(
            "explicit deposit-wallet mutation permit reason required".to_string(),
        ));
    }
    if permit.owner_serialization_evidence.expires_at_unix_seconds <= now_unix_seconds {
        return Err(RelayerError::mutation_blocked(
            "deposit-wallet mutation permit owner serialization evidence is expired".to_string(),
        ));
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

fn retry_after_duration(headers: &HeaderMap) -> Option<Duration> {
    retry_after_duration_at(headers, SystemTime::now())
}

fn retry_after_duration_at(headers: &HeaderMap, now: SystemTime) -> Option<Duration> {
    let value = headers
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)?;
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    let retry_at = httpdate::parse_http_date(value).ok()?;
    Some(retry_at.duration_since(now).unwrap_or(Duration::ZERO))
}

fn retry_after_summary_from_duration(retry_after: Option<Duration>) -> String {
    retry_after
        .map(|duration| format!("; retry after {}s", duration.as_secs()))
        .unwrap_or_default()
}

fn is_transient_poll_error(error: &RelayerError) -> bool {
    match error {
        RelayerError::QuotaExhausted | RelayerError::Timeout | RelayerError::Http(_) => true,
        RelayerError::Api { status, .. } => {
            matches!(*status, 408 | 425 | 429) || (500..=599).contains(status)
        }
        RelayerError::Other(message) if message == RESPONSE_BODY_TOO_LARGE_MESSAGE => false,
        _ => false,
    }
}

fn is_trusted_recovery_fetch_failure(error: &RelayerError) -> bool {
    match error {
        RelayerError::Http(_) | RelayerError::QuotaExhausted | RelayerError::Api { .. } => true,
        RelayerError::Other(message) => message == RESPONSE_BODY_TOO_LARGE_MESSAGE,
        _ => false,
    }
}

fn parse_submit_response(bytes: &[u8]) -> Result<DepositWalletTransactionReceipt> {
    let response = serde_json::from_slice::<RelayerSubmitResponse>(bytes)
        .map_err(|e| RelayerError::Other(format!("could not parse submit response: {e}")))?;
    receipt_from_submit_response(response, None).map(|parsed| parsed.receipt)
}

fn parse_transaction_response(
    expected_transaction_id: &str,
    bytes: &[u8],
) -> std::result::Result<ParsedTransactionReceipt, TransactionParseError> {
    match bytes.iter().copied().find(|byte| !byte.is_ascii_whitespace()) {
        Some(b'{') => {
            let response = serde_json::from_slice::<RelayerTransactionResponseWithOwner>(bytes)
                .map_err(|e| {
                    TransactionParseError::new(
                        RelayerError::Other(format!("could not parse transaction response: {e}")),
                        None,
                    )
                })?;
            let owner = response.owner;
            let receipt = receipt_from_submit_response(response.response, owner)
                .map_err(|error| TransactionParseError::new(error, owner))?;
            return require_transaction_id_match(expected_transaction_id, receipt)
                .map_err(|error| TransactionParseError::new(error, owner));
        }
        Some(b'[') => {}
        _ => {
            return Err(TransactionParseError::new(
                RelayerError::Other(
                    "could not parse transaction response: expected JSON object or array"
                        .to_string(),
                ),
                None,
            ))
        }
    }

    let response = select_transaction_response_from_array(expected_transaction_id, bytes)?;
    let owner = response.owner;
    let receipt = receipt_from_submit_response(response.response, owner)
        .map_err(|error| TransactionParseError::new(error, owner))?;
    require_transaction_id_match(expected_transaction_id, receipt)
        .map_err(|error| TransactionParseError::new(error, owner))
}

fn select_transaction_response_from_array(
    expected_transaction_id: &str,
    bytes: &[u8],
) -> std::result::Result<RelayerTransactionResponseWithOwner, TransactionParseError> {
    struct SelectTransactionVisitor<'a> {
        expected_transaction_id: &'a str,
    }

    impl<'de> Visitor<'de> for SelectTransactionVisitor<'_> {
        type Value = RelayerTransactionResponseWithOwner;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a transaction response array")
        }

        fn visit_seq<A>(self, mut seq: A) -> std::result::Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let mut count = 0usize;
            let mut matching_response = None;
            while let Some(response) = seq.next_element::<RelayerTransactionResponseWithOwner>()? {
                count += 1;
                if count > MAX_TRANSACTION_RESPONSE_ITEMS {
                    return Err(de::Error::custom(TRANSACTION_RESPONSE_ITEM_LIMIT_ERROR));
                }
                if response.response.transaction_id == self.expected_transaction_id {
                    if matching_response.is_some() {
                        return Err(de::Error::custom(TRANSACTION_RESPONSE_DUPLICATE_ID_ERROR));
                    }
                    matching_response = Some(response);
                }
            }
            matching_response
                .ok_or_else(|| de::Error::custom(TRANSACTION_RESPONSE_MISSING_ID_ERROR))
        }
    }

    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let response = deserializer
        .deserialize_seq(SelectTransactionVisitor {
            expected_transaction_id,
        })
        .map_err(|error| {
            let message = error.to_string();
            if message.contains(TRANSACTION_RESPONSE_ITEM_LIMIT_ERROR) {
                TransactionParseError::new(
                    RelayerError::reconciliation_required(format!(
                        "transaction response included more than {MAX_TRANSACTION_RESPONSE_ITEMS} items"
                    )),
                    None,
                )
            } else if message.contains(TRANSACTION_RESPONSE_MISSING_ID_ERROR) {
                TransactionParseError::new(
                    RelayerError::reconciliation_required(format!(
                        "transaction response did not include requested transaction id {}",
                        sanitized_external_token(expected_transaction_id)
                    )),
                    None,
                )
            } else if message.contains(TRANSACTION_RESPONSE_DUPLICATE_ID_ERROR) {
                TransactionParseError::new(
                    RelayerError::reconciliation_required(format!(
                        "transaction response included duplicate requested transaction id {}; manual reconciliation required",
                        sanitized_external_token(expected_transaction_id)
                    )),
                    None,
                )
            } else {
                TransactionParseError::new(
                    RelayerError::Other(format!("could not parse transaction response: {error}")),
                    None,
                )
            }
        })?;
    deserializer.end().map_err(|error| {
        TransactionParseError::new(
            RelayerError::Other(format!("could not parse transaction response: {error}")),
            None,
        )
    })?;
    Ok(response)
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
        .as_deref()
        .map(str::trim)
        .filter(|hash| !hash.is_empty())
        .map(validate_transaction_hash)
        .transpose()?;

    Ok(ParsedTransactionReceipt {
        receipt: DepositWalletTransactionReceipt {
            transaction_id,
            state: response.state,
            transaction_hash,
            owner,
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

fn unknown_state_error_summary(_value: &str) -> &'static str {
    "<unrecognized relayer state>"
}

fn payload_hash_summary(bytes: &[u8]) -> String {
    let hex = hex::encode(keccak256(bytes));
    format!("0x{hex}")
}

fn signed_digest_payload_hash(digest: H256) -> String {
    let hex = hex::encode(digest.as_bytes());
    format!("signed-digest:0x{hex}")
}

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

fn current_recovery_payload_record(
    state: &OwnerMutationState,
    transaction_id: &str,
    owner: Address,
) -> Result<Option<OwnerTransactionRecord>> {
    if let Some(existing) = state.transaction_owners.get(transaction_id) {
        if existing.owner != owner {
            return Err(RelayerError::reconciliation_required(format!(
                "transaction {} is already associated with a different owner; manual reconciliation required",
                sanitized_external_token(transaction_id)
            )));
        }
        return Ok(Some(existing.clone()));
    }

    match state.owner_blocks.get(&owner) {
        Some(OwnerMutationBlock::InFlight {
            payload_hash,
            transaction_id: Some(existing_transaction_id),
        }) if existing_transaction_id == transaction_id => Ok(Some(OwnerTransactionRecord {
            owner,
            payload_hash: payload_hash.clone(),
            source: OwnerTransactionSource::OwnerRecovery,
        })),
        Some(block) => Err(owner_block_error(owner, block)),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests;
