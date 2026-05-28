use super::redaction::{redacted_address, sanitized_external_token, unknown_state_error_summary};
use super::response::{
    parse_transaction_response, validate_transaction_id, ParsedTransactionReceipt,
};
use super::state::OwnerNonceReadReservation;
use super::*;
use serde_json::value::RawValue;
use std::marker::PhantomData;
use std::rc::Rc;

const MAX_WALLET_NONCE_DECIMAL_DIGITS: usize = 78;

#[derive(Deserialize)]
pub(super) struct WalletNonceResponse<'a> {
    #[serde(borrow)]
    nonce: &'a RawValue,
}

pub struct DepositWalletNonceLease {
    owner: Address,
    nonce: U256,
    expires_at_unix_seconds: u64,
    reservation: Option<OwnerNonceReadReservation>,
    _not_send_sync: PhantomData<Rc<()>>,
}

pub struct DepositWalletNonceLeaseSigningContext {
    owner: Address,
    nonce: U256,
    nonce_owner: Address,
    submit_from: Address,
    deposit_wallet: Address,
    chain_id: u64,
    _not_send_sync: PhantomData<Rc<()>>,
}

impl DepositWalletNonceLease {
    pub fn owner(&self) -> Address {
        self.owner
    }

    pub fn nonce(&self) -> U256 {
        self.nonce
    }

    pub(super) fn into_unexpired_reservation(
        mut self,
        now_unix_seconds: u64,
    ) -> Result<OwnerNonceReadReservation> {
        if self.expires_at_unix_seconds <= now_unix_seconds {
            return Err(RelayerError::mutation_blocked(
                "WALLET nonce lease expired before submit; fetch a fresh leased nonce".to_string(),
            ));
        }
        self.reservation.take().ok_or_else(|| {
            RelayerError::reconciliation_required(
                "WALLET nonce lease was already consumed; owner-scoped reconciliation required"
                    .to_string(),
            )
        })
    }
}

impl DepositWalletNonceLeaseSigningContext {
    pub(super) fn new(
        owner: Address,
        nonce: U256,
        deposit_wallet: Address,
        chain_id: u64,
    ) -> Self {
        Self {
            owner,
            nonce,
            nonce_owner: owner,
            submit_from: owner,
            deposit_wallet,
            chain_id,
            _not_send_sync: PhantomData,
        }
    }

    pub fn owner(&self) -> Address {
        self.owner
    }

    pub fn nonce(&self) -> U256 {
        self.nonce
    }

    pub fn nonce_owner(&self) -> Address {
        self.nonce_owner
    }

    pub fn submit_from(&self) -> Address {
        self.submit_from
    }

    pub fn deposit_wallet(&self) -> Address {
        self.deposit_wallet
    }

    pub fn chain_id(&self) -> u64 {
        self.chain_id
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

impl fmt::Debug for DepositWalletNonceLeaseSigningContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DepositWalletNonceLeaseSigningContext")
            .field("owner", &super::redaction::redacted_address(self.owner))
            .field("nonce", &self.nonce)
            .field("deposit_wallet", &super::redaction::redacted_address(self.deposit_wallet))
            .field("chain_id", &self.chain_id)
            .finish_non_exhaustive()
    }
}

impl DepositWalletRelayerClient {
    /// Fetches a WALLET nonce without returning an owner-scoped lease.
    ///
    /// This compatibility API is limited to test-loopback and non-production
    /// diagnostics. Production signing is not enabled in this PR; a later
    /// crate-owned nonce lease capability must keep the owner reservation alive
    /// through signing and submit.
    #[cfg(test)]
    pub(super) async fn get_wallet_nonce(
        &self,
        owner: Address,
        gate: DepositWalletMutationGate,
    ) -> Result<U256> {
        self.ensure_permitted_for_action(
            &gate,
            owner,
            DepositWalletMutationAction::WalletNonceRead,
        )?;
        if self.base_url.is_production_host() {
            return Err(RelayerError::mutation_blocked(
                "production WALLET nonce reads are disabled in this PR; future signing requires a crate-owned nonce lease capability"
                    .to_string(),
            ));
        }
        let _reservation = self.reserve_owner_nonce_read(owner)?;
        self.fetch_wallet_nonce(owner).await
    }

    /// Fetches a WALLET nonce and returns the owner-scoped lease that must be
    /// consumed by [`Self::sign_and_submit_wallet_batch_with_nonce_lease`].
    ///
    /// This is a non-live test-loopback surface in this PR: production permit
    /// construction is intentionally unavailable. Production signing needs a
    /// later crate-owned capability so consumers do not replace the owner lease
    /// with an out-of-band nonce reader.
    pub async fn get_wallet_nonce_with_lease(
        &self,
        owner: Address,
        gate: DepositWalletMutationGate,
    ) -> Result<DepositWalletNonceLease> {
        self.ensure_permitted_for_action(
            &gate,
            owner,
            DepositWalletMutationAction::WalletNonceRead,
        )?;
        if self.base_url.is_production_host() {
            return Err(RelayerError::mutation_blocked(
                "production WALLET nonce lease reads are disabled in this PR; future signing requires a crate-owned nonce lease capability"
                    .to_string(),
            ));
        }
        let expires_at_unix_seconds = match &gate {
            DepositWalletMutationGate::Permit(permit) => permit.expires_at_unix_seconds(),
            DepositWalletMutationGate::Deny => {
                return Err(RelayerError::mutation_blocked(
                    "explicit deposit-wallet mutation permit required".to_string(),
                ))
            }
        };
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
            expires_at_unix_seconds,
            reservation: Some(reservation),
            _not_send_sync: PhantomData,
        })
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

    pub async fn get_transaction_for_owner(
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
        let parsed = match self.fetch_transaction(transaction_id).await {
            Ok(parsed) => parsed,
            Err(error) if error.is_deposit_wallet_reconciliation_required() => {
                self.transition_inflight_transaction_to_ambiguous_if_current(
                    owner,
                    transaction_id,
                )?;
                return Err(error);
            }
            Err(error) => return Err(error),
        };
        let transaction_type = parsed.transaction_type;
        let receipt = match validate_owner_transaction_evidence(owner, parsed.receipt) {
            Ok(receipt) => receipt,
            Err(error) if error.is_deposit_wallet_reconciliation_required() => {
                self.transition_inflight_transaction_to_ambiguous_if_current(
                    owner,
                    transaction_id,
                )?;
                return Err(error);
            }
            Err(error) => return Err(error),
        };
        self.record_terminal_observation_from_receipt(owner, &receipt, transaction_type)?;
        classify_owner_transaction_receipt(receipt)
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
        url.query_pairs_mut()
            .append_pair("transactionID", &transaction_id);
        let response = self
            .send_with_success_limit(Method::GET, url, None, MAX_TRANSACTION_SUCCESS_BODY_BYTES)
            .await?;
        parse_transaction_response(&transaction_id, self.config, &response)
            .map_err(|parse_error| parse_error.error)
    }
}

fn validate_owner_transaction_evidence(
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
    Ok(receipt)
}

fn classify_owner_transaction_receipt(
    receipt: DepositWalletTransactionReceipt,
) -> Result<DepositWalletTransactionReceipt> {
    let transaction_id = sanitized_external_token(&receipt.transaction_id);
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
        | RelayerTransactionState::Mined => {
            return Err(RelayerError::transaction_absent(format!(
                "deposit wallet transaction {transaction_id} is not terminal yet"
            )));
        }
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
    U256::from_dec_str(raw).map_err(|_| {
        RelayerError::Other("invalid WALLET nonce: outside U256 range".to_string())
    })
}
