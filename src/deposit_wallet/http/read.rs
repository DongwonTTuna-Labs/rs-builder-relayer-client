use super::*;
use super::response::{
    parse_transaction_response, validate_transaction_id, ParsedTransactionReceipt, PollFetchError,
};

#[derive(Deserialize)]
pub(super) struct WalletNonceResponse {
    nonce: String,
}


impl DepositWalletRelayerClient {
    pub async fn get_wallet_nonce(&self, owner: Address) -> Result<U256> {
        self.ensure_owner_unblocked(owner)?;
        self.fetch_wallet_nonce(owner).await
    }

    pub async fn get_wallet_nonce_with_evidence(
        &self,
        owner: Address,
    ) -> Result<DepositWalletWalletNonceEvidence> {
        self.ensure_owner_unblocked(owner)?;
        let nonce = self.fetch_wallet_nonce(owner).await?;
        Ok(DepositWalletWalletNonceEvidence::new(
            owner,
            self.mutation_scope(DepositWalletMutationAction::WalletBatch),
            nonce,
            self.clock.now_unix_seconds(),
        ))
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
        U256::from_dec_str(&nonce.nonce)
            .map_err(|e| RelayerError::Other(format!("invalid WALLET nonce: {e}")))
    }

    pub async fn get_transaction(
        &self,
        transaction_id: &str,
    ) -> Result<DepositWalletTransactionReceipt> {
        // Raw transaction lookup is intentionally read-only: it never records,
        // clears, or bypasses owner-scoped mutation blocks. Callers that need
        // owner recovery semantics must use poll_owner_transaction.
        self.fetch_transaction(transaction_id)
            .await
            .map(|parsed| parsed.receipt)
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
