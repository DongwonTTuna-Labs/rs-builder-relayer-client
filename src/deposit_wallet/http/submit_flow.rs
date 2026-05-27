use super::*;
use super::redaction::{
    display_payload_hash, payload_hash_summary, redacted_address, sanitized_external_token,
    signed_digest_payload_hash,
};
use super::response::parse_submit_response;
use super::state::OwnerSubmitReservation;

impl DepositWalletRelayerClient {
    /// Submits a `WALLET-CREATE` request when the mutation gate permits it.
    ///
    /// Production clients created with [`DepositWalletRelayerUrl::parse`] cannot
    /// construct a public permit in this PR; live production submit requires a
    /// later durable owner-state capability. Test-loopback clients exercise the
    /// request body and post-boundary reconciliation behavior.
    pub async fn submit_wallet_create(
        &self,
        owner: Address,
        gate: DepositWalletMutationGate,
    ) -> Result<DepositWalletTransactionReceipt> {
        self.ensure_permitted_for_action(&gate, owner, DepositWalletMutationAction::WalletCreate)?;
        self.ensure_owner_unblocked(owner)?;
        let request = build_wallet_create_request(owner, self.config);
        let body = serde_json::to_string(&request)
            .map_err(|e| RelayerError::Other(format!("could not serialize WALLET-CREATE: {e}")))?;
        self.submit_owner_body(owner, body).await
    }

    /// Submits a signed `WALLET` batch when the mutation gate permits it.
    ///
    /// As with [`Self::submit_wallet_create`], the public production API remains
    /// default-deny in this PR. Production submit enablement is intentionally
    /// reserved for a later live-submit change with durable owner state.
    pub async fn submit_signed_wallet_batch(
        &self,
        signed: SignedDepositWalletBatch,
        gate: DepositWalletMutationGate,
    ) -> Result<DepositWalletTransactionReceipt> {
        let owner = signed.owner();
        self.ensure_permitted_for_action(&gate, owner, DepositWalletMutationAction::WalletBatch)?;
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
        if let Err(error) =
            self.ensure_permitted_for_action(&gate, owner, DepositWalletMutationAction::WalletBatch)
        {
            reservation.clear()?;
            return Err(error);
        }
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

    pub(super) async fn submit_owner_body(
        &self,
        owner: Address,
        body: String,
    ) -> Result<DepositWalletTransactionReceipt> {
        let payload_hash = payload_hash_summary(body.as_bytes());
        let reservation = self.reserve_owner_submit(owner, payload_hash)?;
        self.submit_reserved_owner_body(reservation, body).await
    }

    pub(super) async fn submit_reserved_owner_body(
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
                    self.record_ambiguous_post_boundary(&mut reservation, owner, payload_hash.clone())?;
                    Err(RelayerError::ambiguous_submit(format!(
                    "submit response did not include a usable transactionID for owner {} payload {}: {}",
                    redacted_address(owner),
                        display_payload_hash(&payload_hash),
                        error
                    )))
                }
            },
            Err(RelayerError::Http(error)) => {
                self.record_ambiguous_post_boundary(&mut reservation, owner, payload_hash.clone())?;
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
                self.record_ambiguous_post_boundary(&mut reservation, owner, payload_hash.clone())?;
                Err(RelayerError::ambiguous_submit(format!(
                    "submit returned HTTP status {} after POST for owner {} payload {}; manual reconciliation required",
                    status,
                    redacted_address(owner),
                    display_payload_hash(&payload_hash)
                )))
            }
            Err(RelayerError::QuotaExhausted) => {
                self.record_ambiguous_post_boundary(&mut reservation, owner, payload_hash.clone())?;
                Err(RelayerError::ambiguous_submit(format!(
                    "submit returned HTTP status 429 after POST for owner {} payload {}; manual reconciliation required",
                    redacted_address(owner),
                    display_payload_hash(&payload_hash)
                )))
            }
            Err(RelayerError::Timeout) => {
                self.record_ambiguous_post_boundary(&mut reservation, owner, payload_hash.clone())?;
                Err(RelayerError::ambiguous_submit(format!(
                    "submit timed out after POST for owner {} payload {}; manual reconciliation required",
                    redacted_address(owner),
                    display_payload_hash(&payload_hash)
                )))
            }
            Err(RelayerError::Other(message)) if message == RESPONSE_BODY_TOO_LARGE_MESSAGE => {
                self.record_ambiguous_post_boundary(&mut reservation, owner, payload_hash.clone())?;
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

    fn record_ambiguous_post_boundary(
        &self,
        reservation: &mut OwnerSubmitReservation,
        owner: Address,
        payload_hash: String,
    ) -> Result<()> {
        self.record_ambiguous(owner, payload_hash)?;
        reservation.disarm();
        Ok(())
    }

    pub(super) fn ensure_deadline_fresh(&self, signed: &SignedDepositWalletBatch) -> Result<()> {
        let now = U256::from(self.clock.now_unix_seconds());
        if signed.deadline() <= now {
            return Err(RelayerError::Signing(
                "signed deposit wallet batch deadline is expired".to_string(),
            ));
        }
        Ok(())
    }

}
