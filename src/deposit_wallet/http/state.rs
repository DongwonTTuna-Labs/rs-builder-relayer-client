use super::*;
use super::permit::{
    validate_idless_reconciliation_evidence, validate_permit_fresh, validate_permit_owner,
    validate_permit_scope, validate_reconciliation_evidence,
};
use super::redaction::{
    display_payload_hash, recovered_payload_hash, redacted_address, sanitized_external_token,
    unknown_state_error_summary,
};
use super::response::ParsedTransactionReceipt;

#[derive(Default)]
pub(super) struct OwnerMutationState {
    pub(super) owner_blocks: HashMap<Address, OwnerMutationBlock>,
    pub(super) nonce_reads: HashMap<Address, u64>,
    pub(super) transaction_owners: HashMap<String, OwnerTransactionRecord>,
    pub(super) terminal_observations: HashMap<String, OwnerTransactionTerminalObservation>,
}

#[derive(Clone, Debug)]
pub(super) struct OwnerTransactionRecord {
    pub(super) owner: Address,
    pub(super) payload_hash: String,
    pub(super) source: OwnerTransactionSource,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct OwnerTransactionTerminalObservation {
    pub(super) observed_state: RelayerTransactionState,
    pub(super) transaction_hash: Option<String>,
}

impl OwnerTransactionTerminalObservation {
    pub(super) fn from_receipt(receipt: &DepositWalletTransactionReceipt) -> Option<Self> {
        match receipt.state {
            RelayerTransactionState::Confirmed
            | RelayerTransactionState::Invalid
            | RelayerTransactionState::Failed => Some(Self {
                observed_state: receipt.state.clone(),
                transaction_hash: receipt.transaction_hash.clone(),
            }),
            RelayerTransactionState::New
            | RelayerTransactionState::Executed
            | RelayerTransactionState::Mined
            | RelayerTransactionState::Unknown(_) => None,
        }
    }

    pub(super) fn matches_evidence(
        &self,
        evidence: &DepositWalletSubmitReconciliationEvidence,
    ) -> bool {
        &self.observed_state == evidence.observed_state()
            && self.transaction_hash.as_deref() == evidence.transaction_hash()
    }
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
        created_at_unix_seconds: u64,
    },
    Ambiguous {
        payload_hash: String,
        created_at_unix_seconds: u64,
    },
}

impl OwnerMutationBlock {
    pub(super) fn payload_hash(&self) -> &str {
        match self {
            Self::InFlight { payload_hash, .. } | Self::Ambiguous { payload_hash, .. } => {
                payload_hash
            }
        }
    }

    pub(super) fn created_at_unix_seconds(&self) -> u64 {
        match self {
            Self::InFlight {
                created_at_unix_seconds,
                ..
            }
            | Self::Ambiguous {
                created_at_unix_seconds,
                ..
            } => *created_at_unix_seconds,
        }
    }
}

pub(super) struct OwnerSubmitReservation {
    state: Arc<Mutex<OwnerMutationState>>,
    owner: Address,
    payload_hash: String,
    created_at_unix_seconds: u64,
    drop_action: OwnerSubmitReservationDropAction,
}

pub(super) struct OwnerNonceReadReservation {
    state: Arc<Mutex<OwnerMutationState>>,
    owner: Address,
    created_at_unix_seconds: u64,
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
        created_at_unix_seconds: u64,
    ) -> Self {
        Self {
            state,
            owner,
            payload_hash,
            created_at_unix_seconds,
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
                    created_at_unix_seconds: self.created_at_unix_seconds,
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
                let created_at_unix_seconds = state
                    .owner_blocks
                    .get(&self.owner)
                    .filter(|block| block.payload_hash() == self.payload_hash)
                    .map(OwnerMutationBlock::created_at_unix_seconds)
                    .unwrap_or(self.created_at_unix_seconds);
                state.owner_blocks.insert(
                    self.owner,
                    OwnerMutationBlock::Ambiguous {
                        payload_hash: self.payload_hash.clone(),
                        created_at_unix_seconds,
                    },
                );
            }
            OwnerSubmitReservationDropAction::Disarmed => {}
        }
    }
}

impl Drop for OwnerNonceReadReservation {
    fn drop(&mut self) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if state
            .nonce_reads
            .get(&self.owner)
            .is_some_and(|created_at| *created_at == self.created_at_unix_seconds)
        {
            state.nonce_reads.remove(&self.owner);
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
        self.ensure_permit_token_for_action(
            &permit,
            owner,
            DepositWalletMutationAction::ManualReconciliation,
        )?;

        let mut state = self.mutation_state()?;
        match state.owner_blocks.get(&owner).cloned() {
            Some(OwnerMutationBlock::Ambiguous {
                payload_hash,
                created_at_unix_seconds,
            })
                if payload_hash == evidence.payload_hash() =>
            {
                validate_reconciliation_evidence(
                    &evidence,
                    &permit,
                    self.try_mutation_scope(DepositWalletMutationAction::ManualReconciliation)?,
                    created_at_unix_seconds,
                    self.clock.now_unix_seconds(),
                )?;
                let has_payload_records = state
                    .transaction_owners
                    .values()
                    .any(|record| record.owner == owner && record.payload_hash == evidence.payload_hash());
                match state.transaction_owners.get(evidence.transaction_id()) {
                    Some(record)
                        if record.owner == owner
                            && record.payload_hash == evidence.payload_hash() =>
                    {
                        let Some(observation) =
                            state.terminal_observations.get(evidence.transaction_id())
                        else {
                            return Err(RelayerError::reconciliation_required(format!(
                                "manual reconciliation transaction {} has no trusted terminal poll observation; use owner-scoped transaction polling before clearing",
                                sanitized_external_token(evidence.transaction_id())
                            )));
                        };
                        if !observation.matches_evidence(&evidence) {
                            return Err(RelayerError::reconciliation_required(format!(
                                "manual reconciliation transaction {} did not match the trusted terminal poll observation",
                                sanitized_external_token(evidence.transaction_id())
                            )));
                        }
                        let has_additional_payload_records = state
                            .transaction_owners
                            .iter()
                            .any(|(transaction_id, record)| {
                                transaction_id.as_str() != evidence.transaction_id()
                                    && record.owner == owner
                                    && record.payload_hash == evidence.payload_hash()
                            });
                        state.transaction_owners.remove(evidence.transaction_id());
                        state.terminal_observations.remove(evidence.transaction_id());
                        if has_additional_payload_records {
                            return Err(RelayerError::reconciliation_required(format!(
                                "owner {} has additional ambiguous transactions for payload {}; reconcile each transaction before clearing the owner block",
                                redacted_address(owner),
                                display_payload_hash(evidence.payload_hash())
                            )));
                        }
                    }
                    Some(_) => {
                        return Err(RelayerError::reconciliation_required(format!(
                            "manual reconciliation transaction {} did not match current owner payload",
                            sanitized_external_token(evidence.transaction_id())
                        )));
                    }
                    None if has_payload_records => {
                        return Err(RelayerError::reconciliation_required(format!(
                            "manual reconciliation transaction {} did not match current owner payload",
                            sanitized_external_token(evidence.transaction_id())
                        )));
                    }
                    None => {
                        return Err(RelayerError::reconciliation_required(format!(
                            "manual reconciliation transaction {} has no local owner payload record; use id-less submit reconciliation evidence before clearing",
                            sanitized_external_token(evidence.transaction_id())
                        )));
                    }
                }
                state.owner_blocks.remove(&owner);
            }
            Some(OwnerMutationBlock::Ambiguous { payload_hash, .. }) => {
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

    pub fn clear_idless_ambiguous_submit_after_manual_reconciliation(
        &self,
        evidence: DepositWalletIdlessSubmitReconciliationEvidence,
        permit: DepositWalletMutationPermit,
    ) -> Result<()> {
        let owner = evidence.owner();
        self.ensure_permit_token_for_action(
            &permit,
            owner,
            DepositWalletMutationAction::ManualReconciliation,
        )?;

        let mut state = self.mutation_state()?;
        match state.owner_blocks.get(&owner).cloned() {
            Some(OwnerMutationBlock::Ambiguous {
                payload_hash,
                created_at_unix_seconds,
            })
                if payload_hash == evidence.payload_hash() =>
            {
                validate_idless_reconciliation_evidence(
                    &evidence,
                    &permit,
                    self.try_mutation_scope(DepositWalletMutationAction::ManualReconciliation)?,
                    created_at_unix_seconds,
                    self.clock.now_unix_seconds(),
                )?;
                if state
                    .transaction_owners
                    .values()
                    .any(|record| record.owner == owner && record.payload_hash == evidence.payload_hash())
                {
                    return Err(RelayerError::reconciliation_required(format!(
                        "owner {} has known transactions for payload {}; reconcile each transaction before id-less clearing",
                        redacted_address(owner),
                        display_payload_hash(evidence.payload_hash())
                    )));
                }
                state.owner_blocks.remove(&owner);
            }
            Some(OwnerMutationBlock::Ambiguous { payload_hash, .. }) => {
                return Err(RelayerError::reconciliation_required(format!(
                    "id-less reconciliation evidence payload {} did not match current ambiguous payload {} for owner {}",
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
                    "owner {} has known in-flight submit transaction {}; poll it to a terminal state before id-less clearing",
                    redacted_address(owner),
                    sanitized_external_token(&transaction_id)
                )));
            }
            Some(OwnerMutationBlock::InFlight { .. }) => {
                return Err(RelayerError::mutation_blocked(format!(
                    "owner {} has an active submit request; wait for the response before id-less clearing",
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
            Some(OwnerMutationBlock::Ambiguous { payload_hash, .. }) => Some(payload_hash.clone()),
            _ => None,
        }
    }

    #[cfg(test)]
    pub(super) fn ambiguous_submit_transaction_ids(&self, owner: Address) -> Vec<String> {
        let Ok(state) = self.mutation_state.lock() else {
            return Vec::new();
        };
        let Some(OwnerMutationBlock::Ambiguous { payload_hash, .. }) =
            state.owner_blocks.get(&owner)
        else {
            return Vec::new();
        };
        let mut transaction_ids = state
            .transaction_owners
            .iter()
            .filter_map(|(transaction_id, record)| {
                if record.owner == owner && record.payload_hash == *payload_hash {
                    Some(transaction_id.clone())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        transaction_ids.sort();
        transaction_ids
    }

    pub(super) fn ensure_owner_unblocked(&self, owner: Address) -> Result<()> {
        let state = self.mutation_state()?;
        if let Some(block) = state.owner_blocks.get(&owner) {
            return Err(owner_block_error(owner, block));
        }
        if let Some(created_at_unix_seconds) = state.nonce_reads.get(&owner) {
            return Err(owner_nonce_read_error(owner, *created_at_unix_seconds));
        }
        Ok(())
    }

    pub(super) fn reserve_owner_nonce_read(
        &self,
        owner: Address,
    ) -> Result<OwnerNonceReadReservation> {
        let mut state = self.mutation_state()?;
        if let Some(block) = state.owner_blocks.get(&owner) {
            return Err(owner_block_error(owner, block));
        }
        if let Some(created_at_unix_seconds) = state.nonce_reads.get(&owner) {
            return Err(owner_nonce_read_error(owner, *created_at_unix_seconds));
        }
        if state.nonce_reads.len() >= MAX_OWNER_MUTATION_RECORDS {
            return Err(RelayerError::mutation_blocked(format!(
                "owner mutation state already tracks {MAX_OWNER_MUTATION_RECORDS} nonce reads; retry after in-flight reads complete"
            )));
        }

        let created_at_unix_seconds = self.clock.now_unix_seconds();
        state.nonce_reads.insert(owner, created_at_unix_seconds);
        Ok(OwnerNonceReadReservation {
            state: self.mutation_state.clone(),
            owner,
            created_at_unix_seconds,
        })
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
        if let Some(created_at_unix_seconds) = state.nonce_reads.get(&owner) {
            return Err(owner_nonce_read_error(owner, *created_at_unix_seconds));
        }
        if state.transaction_owners.len() >= MAX_OWNER_MUTATION_RECORDS {
            return Err(RelayerError::mutation_blocked(format!(
                "owner mutation state already tracks {MAX_OWNER_MUTATION_RECORDS} transactions; reconcile terminal transactions before accepting another submit"
            )));
        }
        ensure_owner_mutation_capacity(&state, owner, None)?;

        let created_at_unix_seconds = self.clock.now_unix_seconds();
        state.owner_blocks.insert(
            owner,
            OwnerMutationBlock::InFlight {
                payload_hash: payload_hash.clone(),
                transaction_id: None,
                created_at_unix_seconds,
            },
        );
        Ok(OwnerSubmitReservation::new(
            self.mutation_state.clone(),
            owner,
            payload_hash,
            created_at_unix_seconds,
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
                    "submit response for owner {} transaction {} reached unknown state {}; owner-scoped poll or manual reconciliation required",
                    redacted_address(owner),
                    sanitized_external_token(&receipt.transaction_id),
                    unknown_state_error_summary(raw)
                )))
            }
            RelayerTransactionState::Invalid => {
                self.record_ambiguous(owner, payload_hash.clone())?;
                self.record_transaction_owner(&receipt.transaction_id, owner, payload_hash)?;
                Err(RelayerError::reconciliation_required(format!(
                    "deposit wallet submit transaction {} returned invalid before transaction polling; owner-scoped poll or manual reconciliation required",
                    sanitized_external_token(&receipt.transaction_id)
                )))
            }
            RelayerTransactionState::Failed => {
                self.record_ambiguous(owner, payload_hash.clone())?;
                self.record_transaction_owner(&receipt.transaction_id, owner, payload_hash)?;
                Err(RelayerError::reconciliation_required(format!(
                    "deposit wallet submit transaction {} returned failed before transaction polling; owner-scoped poll or manual reconciliation required",
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
        let created_at_unix_seconds = state
            .owner_blocks
            .get(&owner)
            .filter(|block| block.payload_hash() == payload_hash)
            .map(OwnerMutationBlock::created_at_unix_seconds)
            .unwrap_or_else(|| self.clock.now_unix_seconds());
        state.owner_blocks.insert(
            owner,
            OwnerMutationBlock::Ambiguous {
                payload_hash,
                created_at_unix_seconds,
            },
        );
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
                created_at_unix_seconds: self.clock.now_unix_seconds(),
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
                OwnerMutationBlock::Ambiguous { payload_hash, .. } => {
                    let Some(record) = state.transaction_owners.get(transaction_id) else {
                        return Ok(());
                    };
                    if record.owner != owner || record.payload_hash != *payload_hash {
                        return Err(RelayerError::reconciliation_required(format!(
                            "transaction {} is already associated with a different owner or payload; manual reconciliation required",
                            sanitized_external_token(transaction_id)
                        )));
                    }
                    source = record.source;
                    payload_hash.clone()
                }
                _ => return Err(owner_block_error(owner, block)),
            }
        } else {
            recovered_payload_hash(transaction_id)
        };
        ensure_owner_mutation_capacity(&state, owner, Some(transaction_id))?;
        ensure_transaction_owner_mapping_available(&state, transaction_id, owner, &payload_hash)?;
        state.owner_blocks.insert(
            owner,
            OwnerMutationBlock::InFlight {
                payload_hash: payload_hash.clone(),
                transaction_id: Some(transaction_id.to_string()),
                created_at_unix_seconds: self.clock.now_unix_seconds(),
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
                ..
            }) if existing_transaction_id == transaction_id => payload_hash.clone(),
            Some(OwnerMutationBlock::Ambiguous { payload_hash, .. })
                if state
                    .transaction_owners
                    .get(transaction_id)
                    .is_some_and(|record| record.owner == owner && record.payload_hash == *payload_hash) =>
            {
                return Ok(());
            }
            Some(OwnerMutationBlock::Ambiguous { payload_hash, .. }) => {
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
                created_at_unix_seconds: self.clock.now_unix_seconds(),
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

    pub(super) fn local_transaction_owner(&self, transaction_id: &str) -> Result<Option<Address>> {
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
            state.terminal_observations.remove(transaction_id);
            if state
                .transaction_owners
                .iter()
                .any(|(_, record)| record.owner == owner && record.payload_hash == payload_hash)
            {
                state.owner_blocks.insert(
                    owner,
                    OwnerMutationBlock::Ambiguous {
                        payload_hash: payload_hash.to_string(),
                        created_at_unix_seconds: self.clock.now_unix_seconds(),
                    },
                );
                return Err(RelayerError::reconciliation_required(format!(
                    "owner {} has additional ambiguous transactions for payload {}; reconcile each transaction before clearing the owner block",
                    redacted_address(owner),
                    display_payload_hash(payload_hash)
                )));
            }
            clear_owner_block_if_payload(&mut state, owner, payload_hash);
        }
        Ok(())
    }

    pub(super) fn record_terminal_observation(
        &self,
        transaction_id: &str,
        receipt: &DepositWalletTransactionReceipt,
    ) -> Result<()> {
        let Some(observation) = OwnerTransactionTerminalObservation::from_receipt(receipt) else {
            return Ok(());
        };
        let mut state = self.mutation_state()?;
        if state.transaction_owners.contains_key(transaction_id) {
            state
                .terminal_observations
                .insert(transaction_id.to_string(), observation);
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
                    created_at_unix_seconds: self.clock.now_unix_seconds(),
                },
            );
        }
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

    pub(super) fn ensure_permitted_for_action(
        &self,
        gate: &DepositWalletMutationGate,
        owner: Address,
        action: DepositWalletMutationAction,
    ) -> Result<()> {
        match gate {
            DepositWalletMutationGate::Permit(permit) => {
                self.ensure_permit_token_for_action(permit, owner, action)
            }
            DepositWalletMutationGate::Deny => Err(RelayerError::mutation_blocked(
                "explicit deposit-wallet mutation permit required".to_string(),
            )),
        }
    }

    pub(super) fn ensure_permit_token_for_action(
        &self,
        permit: &DepositWalletMutationPermit,
        owner: Address,
        action: DepositWalletMutationAction,
    ) -> Result<()> {
        validate_permit_owner(permit, owner)?;
        validate_permit_scope(permit, self.try_mutation_scope(action)?)?;
        validate_permit_fresh(permit, self.clock.now_unix_seconds())
    }

}

pub(super) fn owner_block_error(owner: Address, block: &OwnerMutationBlock) -> RelayerError {
    match block {
        OwnerMutationBlock::InFlight {
            payload_hash,
            transaction_id: Some(transaction_id),
            created_at_unix_seconds,
            ..
        } => RelayerError::reconciliation_required(format!(
            "owner {} has in-flight submit transaction {} payload {} since {}; poll to terminal state before another owner mutation",
            redacted_address(owner),
            sanitized_external_token(transaction_id),
            display_payload_hash(payload_hash),
            created_at_unix_seconds
        )),
        OwnerMutationBlock::InFlight {
            payload_hash,
            transaction_id: None,
            created_at_unix_seconds,
            ..
        } => RelayerError::reconciliation_required(format!(
            "owner {} has in-flight submit payload {} since {}; wait for the submit response before another owner mutation",
            redacted_address(owner),
            display_payload_hash(payload_hash),
            created_at_unix_seconds
        )),
        OwnerMutationBlock::Ambiguous { payload_hash, .. } => RelayerError::reconciliation_required(
            format!(
                "owner {} has ambiguous submit payload {}; manual reconciliation required",
                redacted_address(owner),
                display_payload_hash(payload_hash)
            ),
        ),
    }
}

pub(super) fn owner_nonce_read_error(
    owner: Address,
    created_at_unix_seconds: u64,
) -> RelayerError {
    RelayerError::mutation_blocked(format!(
        "owner {} has in-flight nonce read since {}; wait for it before another owner mutation",
        redacted_address(owner),
        created_at_unix_seconds
    ))
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
            ..
        }) if existing_transaction_id == transaction_id => Ok(Some(OwnerTransactionRecord {
            owner,
            payload_hash: payload_hash.clone(),
            source: OwnerTransactionSource::OwnerRecovery,
        })),
        Some(OwnerMutationBlock::Ambiguous { .. }) => Ok(None),
        Some(block) => Err(owner_block_error(owner, block)),
        None => Ok(None),
    }
}
