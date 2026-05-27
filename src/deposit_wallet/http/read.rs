use super::*;
use super::response::{
    parse_transaction_response, validate_transaction_id, ParsedTransactionReceipt, PollFetchError,
};
use super::redaction::{sanitized_external_token, unknown_state_error_summary};
use super::state::OwnerNonceReadReservation;

#[derive(Deserialize)]
pub(super) struct WalletNonceResponse {
    nonce: serde_json::Value,
}

pub struct DepositWalletNonceLease {
    owner: Address,
    nonce: U256,
    reservation: Option<OwnerNonceReadReservation>,
}

impl DepositWalletNonceLease {
    pub fn owner(&self) -> Address {
        self.owner
    }

    pub fn nonce(&self) -> U256 {
        self.nonce
    }

    pub(super) fn into_reservation(mut self) -> OwnerNonceReadReservation {
        self.reservation
            .take()
            .expect("nonce lease reservation should be present until consumed")
    }
}

impl fmt::Debug for DepositWalletNonceLease {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DepositWalletNonceLease")
            .field("owner", &super::redaction::redacted_address(self.owner))
            .field("nonce", &self.nonce)
            .finish_non_exhaustive()
    }
}

impl DepositWalletRelayerClient {
    pub async fn get_wallet_nonce(
        &self,
        owner: Address,
        gate: DepositWalletMutationGate,
    ) -> Result<U256> {
        self.ensure_permitted_for_action(&gate, owner, DepositWalletMutationAction::WalletNonceRead)?;
        let _reservation = self.reserve_owner_nonce_read(owner)?;
        self.fetch_wallet_nonce(owner).await
    }

    pub async fn get_wallet_nonce_with_lease(
        &self,
        owner: Address,
        gate: DepositWalletMutationGate,
    ) -> Result<DepositWalletNonceLease> {
        self.ensure_permitted_for_action(&gate, owner, DepositWalletMutationAction::WalletNonceRead)?;
        let reservation = self.reserve_owner_nonce_read(owner)?;
        let nonce = match self.fetch_wallet_nonce(owner).await {
            Ok(nonce) => nonce,
            Err(error) => {
                drop(reservation);
                return Err(error);
            }
        };
        Ok(DepositWalletNonceLease {
            owner,
            nonce,
            reservation: Some(reservation),
        })
    }

    pub(super) async fn fetch_wallet_nonce(&self, owner: Address) -> Result<U256> {
        let request = build_wallet_nonce_request(owner);
        let mut url = self.base_url.endpoint(request.path());
        url.query_pairs_mut()
            .append_pair("address", &to_checksum(&owner, None))
            .append_pair("type", request.nonce_type());
        let response = self.send(Method::GET, url, None).await?;
        let nonce = serde_json::from_slice::<WalletNonceResponse>(&response)
            .map_err(|e| RelayerError::Other(format!("could not parse WALLET nonce: {e}")))?;
        parse_wallet_nonce_value(nonce.nonce)
    }

    pub async fn get_transaction(
        &self,
        transaction_id: &str,
    ) -> Result<DepositWalletTransactionReceipt> {
        self.fetch_transaction(transaction_id)
            .await
            .and_then(|parsed| validate_public_transaction_receipt(parsed.receipt))
    }

    pub(super) async fn fetch_transaction(&self, transaction_id: &str) -> Result<ParsedTransactionReceipt> {
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

    pub(super) async fn fetch_transaction_for_poll(
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
}

fn validate_public_transaction_receipt(
    receipt: DepositWalletTransactionReceipt,
) -> Result<DepositWalletTransactionReceipt> {
    let transaction_id = sanitized_external_token(&receipt.transaction_id);
    if receipt.owner.is_none() {
        return Err(RelayerError::reconciliation_required(format!(
            "deposit wallet transaction {transaction_id} did not include owner evidence; manual reconciliation required"
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

pub(super) fn parse_wallet_nonce_value(value: serde_json::Value) -> Result<U256> {
    match value {
        serde_json::Value::String(raw) => parse_wallet_nonce_decimal(&raw),
        serde_json::Value::Number(number) => parse_wallet_nonce_decimal(&number.to_string()),
        _ => Err(RelayerError::Other(
            "invalid WALLET nonce: expected decimal string or JSON number".to_string(),
        )),
    }
}

fn parse_wallet_nonce_decimal(raw: &str) -> Result<U256> {
    U256::from_dec_str(raw)
        .map_err(|e| RelayerError::Other(format!("invalid WALLET nonce: {e}")))
}
