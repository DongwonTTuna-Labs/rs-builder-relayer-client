use super::redaction::{redacted_address, sanitized_external_token, unknown_state_error_summary};
use super::response::{
    parse_deployed_response, parse_transaction_response, validate_transaction_id,
    ParsedTransactionReceipt,
};
use super::*;
use serde::Deserialize;
use serde_json::value::RawValue;
use tokio::time::{sleep_until, Instant};

const MAX_WALLET_NONCE_DECIMAL_DIGITS: usize = 78;
const DEFAULT_TRANSACTION_POLL_INITIAL_BACKOFF: Duration = Duration::from_millis(500);
const DEFAULT_TRANSACTION_POLL_MAX_BACKOFF: Duration = Duration::from_secs(5);
const DEFAULT_TRANSACTION_POLL_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DepositWalletPollingConfig {
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
    pub timeout: Duration,
}

impl Default for DepositWalletPollingConfig {
    fn default() -> Self {
        Self {
            initial_backoff: DEFAULT_TRANSACTION_POLL_INITIAL_BACKOFF,
            max_backoff: DEFAULT_TRANSACTION_POLL_MAX_BACKOFF,
            timeout: DEFAULT_TRANSACTION_POLL_TIMEOUT,
        }
    }
}

impl DepositWalletPollingConfig {
    fn validate(self) -> Result<Self> {
        if self.initial_backoff.is_zero() {
            return Err(RelayerError::Other(
                "transaction poll initial backoff must be greater than zero".to_string(),
            ));
        }
        if self.max_backoff.is_zero() {
            return Err(RelayerError::Other(
                "transaction poll max backoff must be greater than zero".to_string(),
            ));
        }
        if self.timeout.is_zero() {
            return Err(RelayerError::Other(
                "transaction poll timeout must be greater than zero".to_string(),
            ));
        }
        if self.initial_backoff > self.max_backoff {
            return Err(RelayerError::Other(
                "transaction poll initial backoff must not exceed max backoff".to_string(),
            ));
        }
        Ok(self)
    }
}

#[derive(Deserialize)]
pub(super) struct WalletNonceResponse<'a> {
    #[serde(borrow)]
    nonce: &'a RawValue,
}

impl DepositWalletRelayerClient {
    /// Fetches a fresh WALLET nonce for the supplied deposit-wallet owner.
    pub async fn get_wallet_nonce(&self, owner: Address) -> Result<U256> {
        self.fetch_wallet_nonce(owner).await
    }

    pub async fn discover_deposit_wallet(
        &self,
        owner: Address,
    ) -> Result<DepositWalletDeployment> {
        self.fetch_deployed_wallet(owner).await
    }

    pub async fn get_transaction_for_owner(
        &self,
        owner: Address,
        transaction_id: &str,
    ) -> Result<DepositWalletTransactionReceipt> {
        self.fetch_transaction(transaction_id)
            .await
            .and_then(|parsed| validate_owner_transaction_receipt(owner, parsed.receipt))
    }

    pub async fn poll_transaction_for_owner(
        &self,
        owner: Address,
        transaction_id: &str,
    ) -> Result<DepositWalletTransactionReceipt> {
        self.poll_transaction_for_owner_with_config(
            owner,
            transaction_id,
            DepositWalletPollingConfig::default(),
        )
        .await
    }

    pub async fn poll_transaction_for_owner_with_config(
        &self,
        owner: Address,
        transaction_id: &str,
        config: DepositWalletPollingConfig,
    ) -> Result<DepositWalletTransactionReceipt> {
        let config = config.validate()?;
        let transaction_id = validate_transaction_id(transaction_id)?;
        let deadline = Instant::now() + config.timeout;
        let mut backoff = config.initial_backoff;

        loop {
            let receipt = self.get_transaction_for_owner(owner, &transaction_id).await?;
            if matches!(receipt.state, RelayerTransactionState::Confirmed) {
                return Ok(receipt);
            }

            let now = Instant::now();
            if now >= deadline {
                return Err(RelayerError::Timeout);
            }
            let remaining = deadline - now;
            let sleep_for = backoff.min(config.max_backoff).min(remaining);
            let reaches_deadline = sleep_for == remaining;
            sleep_until(now + sleep_for).await;
            if reaches_deadline {
                return Err(RelayerError::Timeout);
            }
            backoff = next_poll_backoff(backoff, config.max_backoff);
        }
    }

    pub(super) async fn fetch_wallet_nonce(&self, owner: Address) -> Result<U256> {
        let request = build_wallet_nonce_request(owner);
        let mut url = self.base_url.endpoint(request.path());
        url.query_pairs_mut()
            .append_pair("address", &to_checksum(&owner, None))
            .append_pair("type", request.nonce_type());
        let response = self.send(Method::GET, url, None).await?;
        parse_wallet_nonce_response(&response)
    }

    pub(super) async fn fetch_deployed_wallet(
        &self,
        owner: Address,
    ) -> Result<DepositWalletDeployment> {
        let expected_deposit_wallet = derive_deposit_wallet_address(owner, self.config)?;
        let mut url = self.base_url.endpoint(DEPLOYED_PATH);
        url.query_pairs_mut()
            .append_pair("address", &to_checksum(&owner, None))
            .append_pair("type", WALLET_TRANSACTION_TYPE);
        let response = self
            .send_with_success_limit(Method::GET, url, None, MAX_DEPLOYED_SUCCESS_BODY_BYTES)
            .await?;
        parse_deployed_response(owner, expected_deposit_wallet, &response)
    }

    pub(super) async fn fetch_transaction(
        &self,
        transaction_id: &str,
    ) -> Result<ParsedTransactionReceipt> {
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
        parse_transaction_response(&transaction_id, self.config, &response)
            .map_err(|parse_error| parse_error.error)
    }
}

fn next_poll_backoff(current: Duration, max: Duration) -> Duration {
    current.checked_mul(2).unwrap_or(max).min(max)
}

fn validate_owner_transaction_receipt(
    expected_owner: Address,
    receipt: DepositWalletTransactionReceipt,
) -> Result<DepositWalletTransactionReceipt> {
    let transaction_id = sanitized_external_token(&receipt.transaction_id);
    let Some(owner) = receipt.owner else {
        return Err(RelayerError::reconciliation_required(format!(
            "deposit wallet transaction {transaction_id} did not include owner evidence; manual reconciliation required"
        )));
    };
    if owner != expected_owner {
        return Err(RelayerError::reconciliation_required(format!(
            "deposit wallet transaction {transaction_id} owner {} did not match requested owner {}; manual reconciliation required",
            redacted_address(owner),
            redacted_address(expected_owner)
        )));
    }

    match &receipt.state {
        RelayerTransactionState::Confirmed => {
            if receipt.transaction_hash.is_none() {
                return Err(RelayerError::reconciliation_required(format!(
                    "confirmed deposit wallet transaction {transaction_id} did not include transactionHash; manual reconciliation required"
                )));
            }
        }
        RelayerTransactionState::Invalid => {
            return Err(RelayerError::TransactionInvalid(format!(
                "deposit wallet transaction {transaction_id} invalid"
            )));
        }
        RelayerTransactionState::Failed => {
            return Err(RelayerError::TransactionFailed(format!(
                "deposit wallet transaction {transaction_id} failed"
            )));
        }
        RelayerTransactionState::Unknown(raw) => {
            return Err(RelayerError::reconciliation_required(format!(
                "deposit wallet transaction {transaction_id} reached unknown state {}",
                unknown_state_error_summary(raw)
            )));
        }
        RelayerTransactionState::New
        | RelayerTransactionState::Executed
        | RelayerTransactionState::Mined => {}
    }
    Ok(receipt)
}

pub(super) fn parse_wallet_nonce_response(response: &[u8]) -> Result<U256> {
    let nonce = serde_json::from_slice::<WalletNonceResponse>(response)
        .map_err(|_| RelayerError::Other("could not parse WALLET nonce".to_string()))?;
    parse_wallet_nonce_raw(nonce.nonce.get())
}

fn parse_wallet_nonce_raw(raw: &str) -> Result<U256> {
    if raw.starts_with('"') {
        let decoded = serde_json::from_str::<String>(raw)
            .map_err(|_| RelayerError::Other("could not parse WALLET nonce".to_string()))?;
        return parse_wallet_nonce_decimal(&decoded);
    }
    if raw.as_bytes().first().is_some_and(u8::is_ascii_digit) {
        return parse_wallet_nonce_decimal(raw);
    }
    Err(RelayerError::Other(
        "invalid WALLET nonce: expected decimal string or JSON number".to_string(),
    ))
}

fn parse_wallet_nonce_decimal(raw: &str) -> Result<U256> {
    if raw.is_empty()
        || raw.len() > MAX_WALLET_NONCE_DECIMAL_DIGITS
        || !raw.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(RelayerError::Other(
            "invalid WALLET nonce: expected 1-78 ASCII decimal digits".to_string(),
        ));
    }
    U256::from_dec_str(raw)
        .map_err(|_| RelayerError::Other("invalid WALLET nonce: outside U256 range".to_string()))
}
