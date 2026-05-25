use super::*;
use super::permit::{validate_permit_fresh, validate_permit_owner};
use super::redaction::{
    display_payload_hash, recovered_payload_hash, redacted_address, sanitized_external_token,
    unknown_state_error_summary,
};
use super::response::ParsedTransactionReceipt;

#[derive(Default)]
pub(super) struct OwnerMutationState {
    pub(super) owner_blocks: HashMap<Address, OwnerMutationBlock>,
    pub(super) transaction_owners: HashMap<String, OwnerTransactionRecord>,
}

#[derive(Clone, Debug)]
pub(super) struct OwnerTransactionRecord {
    pub(super) owner: Address,
    pub(super) payload_hash: String,
    pub(super) source: OwnerTransactionSource,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum OwnerTransactionSource {
    LocalSubmit,
    OwnerRecovery,
}

#[derive(Clone, Debug)]
pub(super) enum OwnerMutationBlock {
    InFlight {
        payload_hash: String,
        transaction_id: Option<String>,
    },
    Ambiguous {
        payload_hash: String,
    },
}

impl OwnerMutationBlock {
    pub(super) fn payload_hash(&self) -> &str {
        match self {
            Self::InFlight { payload_hash, .. } | Self::Ambiguous { payload_hash } => {
                payload_hash
            }
        }
    }
}

pub(super) struct OwnerSubmitReservation {
    state: Arc<Mutex<OwnerMutationState>>,
    owner: Address,
    payload_hash: String,
    drop_action: OwnerSubmitReservationDropAction,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum OwnerSubmitReservationDropAction {
    Clear,
    Ambiguous,
    Disarmed,
}

impl OwnerSubmitReservation {
    pub(super) fn new(
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

    pub(super) fn owner(&self) -> Address {
        self.owner
    }

    pub(super) fn payload_hash(&self) -> &str {
        &self.payload_hash
    }

    pub(super) fn update_payload_hash(&mut self, payload_hash: String) -> Result<()> {
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

    pub(super) fn clear(&mut self) -> Result<()> {
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

    pub(super) fn arm_ambiguous_on_drop(&mut self) {
        self.drop_action = OwnerSubmitReservationDropAction::Ambiguous;
    }

    pub(super) fn disarm(&mut self) {
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

impl DepositWalletRelayerClient {
    pub fn clear_ambiguous_submit_after_manual_reconciliation(
        &self,
        evidence: DepositWalletSubmitReconciliationEvidence,
        permit: DepositWalletMutationPermit,
    ) -> Result<()> {
        let owner = evidence.owner();
        self.ensure_permitted(&DepositWalletMutationGate::Permit(permit), owner)?;

        let mut state = self.mutation_state()?;
        match state.owner_blocks.get(&owner).cloned() {
            Some(OwnerMutationBlock::Ambiguous { payload_hash })
                if payload_hash == evidence.payload_hash() =>
            {
                state.owner_blocks.remove(&owner);
                state.transaction_owners.retain(|_, record| {
                    record.owner != owner || record.payload_hash != evidence.payload_hash()
                });
            }
            Some(OwnerMutationBlock::Ambiguous { payload_hash }) => {
                return Err(RelayerError::reconciliation_required(format!(
                    "manual reconciliation evidence payload {} did not match current ambiguous payload {} for owner {}",
                    display_payload_hash(evidence.payload_hash()),
                    display_payload_hash(&payload_hash),
                    redacted_address(owner)
                )));
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

    pub(super) fn ensure_owner_unblocked(&self, owner: Address) -> Result<()> {
        let state = self.mutation_state()?;
        if let Some(block) = state.owner_blocks.get(&owner) {
            return Err(owner_block_error(owner, block));
        }
        Ok(())
    }

    pub(super) fn reserve_owner_submit(
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

    pub(super) fn handle_submit_receipt(
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

    pub(super) fn record_ambiguous(&self, owner: Address, payload_hash: String) -> Result<()> {
        let mut state = self.mutation_state()?;
        state
            .owner_blocks
            .insert(owner, OwnerMutationBlock::Ambiguous { payload_hash });
        Ok(())
    }

    pub(super) fn record_inflight_transaction(
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

    pub(super) fn record_transaction_owner(
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

    pub(super) fn record_recovered_inflight_transaction(
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

    pub(super) fn record_recovered_ambiguous_transaction(
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

    pub(super) fn has_recovery_owner_evidence(&self, owner: Address, transaction_id: &str) -> Result<bool> {
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

    pub(super) fn transaction_owner(&self, transaction_id: &str) -> Result<Option<Address>> {
        let state = self.mutation_state()?;
        Ok(state
            .transaction_owners
            .get(transaction_id)
            .map(|record| record.owner))
    }

    pub(super) fn current_recovery_payload_record(
        &self,
        transaction_id: &str,
        owner: Address,
    ) -> Result<Option<OwnerTransactionRecord>> {
        let state = self.mutation_state()?;
        current_recovery_payload_record(&state, transaction_id, owner)
    }

    pub(super) fn require_transaction_owner(
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

    pub(super) fn clear_transaction_block_if_current(
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

    pub(super) fn mark_transaction_reconciliation_required(&self, transaction_id: &str) -> Result<()> {
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

    pub(super) fn clear_owner_block_if_payload(&self, owner: Address, payload_hash: &str) -> Result<()> {
        let mut state = self.mutation_state()?;
        clear_owner_block_if_payload(&mut state, owner, payload_hash);
        Ok(())
    }

    pub(super) fn mutation_state(&self) -> Result<MutexGuard<'_, OwnerMutationState>> {
        self.mutation_state.lock().map_err(|_| {
            RelayerError::reconciliation_required(
                "deposit wallet owner mutation state lock is poisoned; manual reconciliation required"
                    .to_string(),
            )
        })
    }

    pub(super) fn ensure_permitted(&self, gate: &DepositWalletMutationGate, owner: Address) -> Result<()> {
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

}

pub(super) fn owner_block_error(owner: Address, block: &OwnerMutationBlock) -> RelayerError {
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

pub(super) fn clear_owner_block_if_payload(
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

pub(super) fn ensure_owner_mutation_capacity(
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

pub(super) fn ensure_transaction_owner_mapping_available(
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

pub(super) fn current_recovery_payload_record(
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
