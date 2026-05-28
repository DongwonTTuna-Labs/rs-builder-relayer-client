use super::*;
use super::redaction::{
    display_payload_hash, external_token_hash, payload_hash_summary, redacted_address,
    signed_digest_payload_hash,
};
use super::read::DepositWalletNonceLease;
use super::response::{extract_submit_transaction_id, parse_submit_response};
use super::state::{OwnerNonceReadReservation, OwnerSubmitReservation};

impl DepositWalletRelayerClient {
    /// Submits a `WALLET-CREATE` request when the mutation gate permits it.
    ///
    /// Production clients created with [`DepositWalletRelayerUrl::parse`] cannot
    /// construct a public permit in this PR; live production submit requires a
    /// later durable owner-state capability. Test-loopback clients exercise the
    /// request body and post-boundary reconciliation behavior. This method is
    /// non-live for production until that capability is added.
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

    /// Submits a signed `WALLET` batch using a nonce lease returned by
    /// [`Self::get_wallet_nonce_with_lease`].
    ///
    /// This is a non-live test-loopback surface in this PR: production permit
    /// construction is intentionally unavailable, and production mutation
    /// submission remains blocked until durable owner state and the later
    /// live-execution capability are added.
    pub async fn submit_signed_wallet_batch_with_nonce_lease(
        &self,
        signed: SignedDepositWalletBatch,
        gate: DepositWalletMutationGate,
        nonce_lease: DepositWalletNonceLease,
    ) -> Result<DepositWalletTransactionReceipt> {
        let owner = signed.owner();
        self.ensure_permitted_for_action(&gate, owner, DepositWalletMutationAction::WalletBatch)?;
        if nonce_lease.owner() != owner {
            return Err(RelayerError::mutation_blocked(format!(
                "signed WALLET batch owner {} did not match WALLET nonce lease owner {}",
                redacted_address(owner),
                redacted_address(nonce_lease.owner())
            )));
        }
        if nonce_lease.nonce() != signed.nonce() {
            return Err(RelayerError::Signing(
                "signed deposit wallet batch nonce does not match WALLET nonce lease".to_string(),
            ));
        }
        let now_unix_seconds = self.clock.now_unix_seconds()?;
        self.submit_signed_wallet_batch_inner(
            signed,
            nonce_lease.into_unexpired_reservation(now_unix_seconds)?,
        )
        .await
    }

    async fn submit_signed_wallet_batch_inner(
        &self,
        signed: SignedDepositWalletBatch,
        nonce_reservation: OwnerNonceReadReservation,
    ) -> Result<DepositWalletTransactionReceipt> {
        self.ensure_deadline_fresh(&signed)?;
        self.auth.headers()?;
        let preflight_hash = signed_digest_payload_hash(signed.digest());
        let mut reservation =
            self.promote_owner_nonce_read_to_submit(nonce_reservation, preflight_hash)?;

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
        self.auth.headers()?;
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
        let headers = self.authenticated_headers(true)?;
        reservation.arm_ambiguous_on_drop();
        match self
            .send_with_headers_success_limit(
                Method::POST,
                url,
                headers,
                Some(body),
                MAX_SUCCESS_BODY_BYTES,
            )
            .await
        {
            Ok(response) => match parse_submit_response(&response) {
                Ok(receipt) => {
                    let transaction_id = receipt.transaction_id.clone();
                    match self.handle_submit_receipt(owner, payload_hash.clone(), receipt) {
                        Ok(receipt) => {
                            reservation.disarm();
                            Ok(receipt)
                        }
                        Err(error)
                            if error.is_deposit_wallet_ambiguous_submit()
                                || error.is_deposit_wallet_reconciliation_required() =>
                        {
                            if !self
                                .ambiguous_submit_transaction_ids(owner)
                                .iter()
                                .any(|id| id == &transaction_id)
                            {
                                self.record_ambiguous_post_boundary(
                                    &mut reservation,
                                    owner,
                                    payload_hash.clone(),
                                )?;
                                self.record_unrecorded_transaction_id_observation(
                                    owner,
                                    &payload_hash,
                                )?;
                            }
                            Err(error)
                        }
                        Err(_) => {
                            self.record_ambiguous_post_boundary(
                                &mut reservation,
                                owner,
                                payload_hash.clone(),
                            )?;
                            self.record_unrecorded_transaction_id_observation(
                                owner,
                                &payload_hash,
                            )?;
                            Err(RelayerError::ambiguous_submit(format!(
                                "submit response included transaction id hash {} for owner {} payload {} but local owner state could not record it; owner-scoped reconciliation required",
                                external_token_hash(&transaction_id),
                                redacted_address(owner),
                                display_payload_hash(&payload_hash)
                            )))
                        }
                    }
                }
                Err(_error) => {
                    if let Some(transaction_id) = extract_submit_transaction_id(&response) {
                        if self
                            .record_transaction_owner(&transaction_id, owner, payload_hash.clone())
                            .is_err()
                        {
                            self.record_ambiguous_post_boundary(
                                &mut reservation,
                                owner,
                                payload_hash.clone(),
                            )?;
                            self.record_unrecorded_transaction_id_observation(
                                owner,
                                &payload_hash,
                            )?;
                            return Err(RelayerError::ambiguous_submit(format!(
                                "submit response included transaction id hash {} for owner {} payload {} but local owner state could not record it; owner-scoped reconciliation required",
                                external_token_hash(&transaction_id),
                                redacted_address(owner),
                                display_payload_hash(&payload_hash)
                            )));
                        }
                        return Err(RelayerError::ambiguous_submit(format!(
                            "submit response included transaction id hash {} for owner {} payload {} but was otherwise unusable; owner-scoped poll required",
                            external_token_hash(&transaction_id),
                            redacted_address(owner),
                            display_payload_hash(&payload_hash)
                        )));
                    }
                    self.record_ambiguous_post_boundary(&mut reservation, owner, payload_hash.clone())?;
                    Err(RelayerError::ambiguous_submit(format!(
                    "submit response did not include a usable transactionID for owner {} payload {}; manual reconciliation required",
                    redacted_address(owner),
                        display_payload_hash(&payload_hash)
                    )))
                }
            },
            Err(RelayerError::Http(error)) => {
                let category = if error.is_timeout() {
                    "timeout"
                } else if error.is_connect() {
                    "connect"
                } else if error.is_body() {
                    "body"
                } else if error.is_decode() {
                    "decode"
                } else if error.is_request() {
                    "request"
                } else {
                    "transport"
                };
                self.record_ambiguous_post_boundary(&mut reservation, owner, payload_hash.clone())?;
                Err(RelayerError::ambiguous_submit(format!(
                    "submit transport failed for owner {} payload {}; retry status is ambiguous; transport category: {}",
                    redacted_address(owner),
                    display_payload_hash(&payload_hash),
                    category
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
            Err(RelayerError::AuthError(_)) => {
                self.record_ambiguous_post_boundary(
                    &mut reservation,
                    owner,
                    payload_hash.clone(),
                )?;
                Err(RelayerError::ambiguous_submit(format!(
                    "submit authentication failed after owner reservation for owner {} payload {}; manual reconciliation required",
                    redacted_address(owner),
                    display_payload_hash(&payload_hash)
                )))
            }
            Err(_error) => {
                self.record_ambiguous_post_boundary(&mut reservation, owner, payload_hash.clone())?;
                Err(RelayerError::ambiguous_submit(format!(
                    "submit failed after POST boundary with an unclassified error for owner {} payload {}; manual reconciliation required",
                    redacted_address(owner),
                    display_payload_hash(&payload_hash)
                )))
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
        let now = U256::from(self.clock.now_unix_seconds()?);
        if signed.deadline() <= now {
            return Err(RelayerError::Signing(
                "signed deposit wallet batch deadline is expired".to_string(),
            ));
        }
        Ok(())
    }

}
