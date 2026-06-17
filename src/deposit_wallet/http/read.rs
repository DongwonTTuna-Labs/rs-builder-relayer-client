#[cfg(test)]
use super::redaction::{redacted_address, sanitized_external_token, unknown_state_error_summary};
#[cfg(test)]
use super::response::{parse_transaction_response, validate_transaction_id, ParsedTransactionReceipt};
use super::*;
use serde::Deserialize;
use serde_json::value::RawValue;

const MAX_WALLET_NONCE_DECIMAL_DIGITS: usize = 78;

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

    pub(super) async fn fetch_wallet_nonce(&self, owner: Address) -> Result<U256> {
        let request = build_wallet_nonce_request(owner);
        let mut url = self.base_url.endpoint(request.path());
        url.query_pairs_mut()
            .append_pair("address", &to_checksum(&owner, None))
            .append_pair("type", request.nonce_type());
        let response = self.send(Method::GET, url, None).await?;
        parse_wallet_nonce_response(&response)
    }

    #[cfg(test)]
    pub(crate) async fn get_transaction_for_owner(
        &self,
        owner: Address,
        transaction_id: &str,
    ) -> Result<DepositWalletTransactionReceipt> {
        if self.base_url.is_production_host() {
            return Err(RelayerError::read_blocked(
                "production WALLET transaction reads are disabled in this PR until an official or recorded WALLET polling response fixture is reviewed"
                    .to_string(),
            ));
        }
        self.fetch_transaction(transaction_id)
            .await
            .and_then(|parsed| validate_owner_transaction_receipt(owner, parsed.receipt))
    }

    #[cfg(test)]
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

#[cfg(test)]
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
