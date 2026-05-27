use super::*;
use super::redaction::{redacted_address, sanitized_external_token, unknown_state_error_summary};
use super::response::validate_transaction_id;
use super::state::OwnerTransactionSource;

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

    pub(super) fn validate(&self) -> Result<()> {
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

    pub(super) fn interval_for_attempt(&self, attempt: usize) -> Duration {
        let multiplier = 1u32 << attempt.min(4);
        self.interval
            .saturating_mul(multiplier)
            .min(MAX_POLL_INTERVAL)
    }

    pub(super) fn interval_for_transaction_attempt(&self, transaction_id: &str, attempt: usize) -> Duration {
        let base = self.interval_for_attempt(attempt);
        base.saturating_add(transaction_poll_jitter(transaction_id, attempt, base))
            .min(MAX_POLL_INTERVAL)
    }
}

impl Default for DepositWalletPollPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            interval: Duration::from_secs(1),
        }
    }
}

pub(super) trait DepositWalletClock: Send + Sync {
    fn now_unix_seconds(&self) -> u64;
}

pub(super) struct SystemClock;

impl DepositWalletClock for SystemClock {
    fn now_unix_seconds(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }
}

pub(super) trait DepositWalletSleeper: Send + Sync {
    fn sleep<'a>(&'a self, duration: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>;
}

pub(super) struct TokioSleeper;

impl DepositWalletSleeper for TokioSleeper {
    fn sleep<'a>(&'a self, duration: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(tokio::time::sleep(duration))
    }
}


impl DepositWalletRelayerClient {
    pub async fn poll_transaction(
        &self,
        transaction_id: &str,
        policy: DepositWalletPollPolicy,
    ) -> Result<DepositWalletTransactionReceipt> {
        policy.validate()?;
        let transaction_id = validate_transaction_id(transaction_id)?;
        // Public transaction polling is read-only. Owner mutation recovery must
        // use the owner-scoped polling APIs below, which require local owner
        // evidence or an explicit reconciliation permit.
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

    #[cfg(test)]
    pub(super) async fn poll_owner_transaction_with_reconciliation_permit(
        &self,
        owner: Address,
        transaction_id: &str,
        policy: DepositWalletPollPolicy,
        gate: DepositWalletMutationGate,
    ) -> Result<DepositWalletTransactionReceipt> {
        policy.validate()?;
        self.ensure_permitted_for_action(
            &gate,
            owner,
            DepositWalletMutationAction::OwnerRecoveryPoll,
        )?;
        let transaction_id = validate_transaction_id(transaction_id)?;
        self.poll_validated_transaction(transaction_id, policy, Some(owner), true)
            .await
    }

    pub(super) async fn poll_validated_transaction(
        &self,
        transaction_id: String,
        policy: DepositWalletPollPolicy,
        expected_owner: Option<Address>,
        trusted_owner_recovery: bool,
    ) -> Result<DepositWalletTransactionReceipt> {
        let transaction_id_for_error = sanitized_external_token(&transaction_id);
        if let Some(owner) = expected_owner {
            if !trusted_owner_recovery {
                let _ = self.has_recovery_owner_evidence(owner, &transaction_id)?;
            }
        }

        for attempt in 0..policy.max_attempts {
            let parsed = match self.fetch_transaction_for_poll(&transaction_id).await {
                Ok(parsed) => parsed,
                Err(poll_error) => {
                    if (poll_error.retryable_absence
                        || is_transient_poll_error(&poll_error.error))
                        && attempt + 1 < policy.max_attempts
                    {
                        let policy_interval =
                            policy.interval_for_transaction_attempt(&transaction_id, attempt);
                        let sleep_for = poll_error
                            .retry_after
                            .map(|retry_after| {
                                retry_after_poll_interval(
                                    &transaction_id,
                                    attempt,
                                    policy_interval,
                                    retry_after,
                                )
                            })
                            .unwrap_or(policy_interval);
                        self.sleeper
                            .sleep(sleep_for)
                            .await;
                        continue;
                    }
                    let response_owner = poll_error.owner;
                    let error = poll_error.error;
                    if let Some(owner) = expected_owner {
                        if response_owner == Some(owner) {
                            self.record_recovered_ambiguous_transaction(owner, &transaction_id)?;
                        } else if self
                            .current_recovery_payload_record(&transaction_id, owner)?
                            .is_some()
                        {
                            self.mark_transaction_reconciliation_required(&transaction_id)?;
                        }
                    }
                    return Err(error);
                }
            };
            let owner_to_verify = expected_owner;
            if let Some(owner) = owner_to_verify {
                if let Err(error) = self.require_transaction_owner(&transaction_id, &parsed, owner)
                {
                    self.mark_transaction_reconciliation_required(&transaction_id)?;
                    return Err(error);
                }
            }
            let mut receipt = parsed.receipt;
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
                        }
                        return Err(RelayerError::reconciliation_required(format!(
                            "confirmed deposit wallet transaction {} did not include transactionHash; manual reconciliation required",
                            transaction_id_for_error
                        )));
                    }
                    if let Some(owner) = owner_to_verify {
                        if trusted_owner_recovery && terminal_evidence.is_none() {
                            self.record_recovered_ambiguous_transaction(owner, &transaction_id)?;
                            self.record_terminal_observation(&transaction_id, &receipt)?;
                            return Err(RelayerError::reconciliation_required(format!(
                                "confirmed owner-scoped recovery transaction {} did not prove the ambiguous submit payload; manual reconciliation required",
                                transaction_id_for_error
                            )));
                        }
                    }
                    if let Some((owner, record)) = terminal_evidence {
                        if record.source == OwnerTransactionSource::OwnerRecovery {
                            self.record_terminal_observation(&transaction_id, &receipt)?;
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
                    if let Some(owner) = owner_to_verify {
                        receipt.owner = Some(owner);
                    }
                    return Ok(receipt);
                }
                RelayerTransactionState::Invalid => {
                    if let Some((owner, record)) = terminal_evidence {
                        self.record_terminal_observation(&transaction_id, &receipt)?;
                        match record.source {
                            OwnerTransactionSource::OwnerRecovery => {
                                self.record_recovered_ambiguous_transaction(owner, &transaction_id)?;
                            }
                            OwnerTransactionSource::LocalSubmit => {
                                self.mark_transaction_reconciliation_required(&transaction_id)?;
                            }
                        }
                    } else if let Some(owner) = owner_to_verify {
                        if trusted_owner_recovery {
                            self.record_recovered_ambiguous_transaction(owner, &transaction_id)?;
                            self.record_terminal_observation(&transaction_id, &receipt)?;
                        }
                    }
                    return Err(RelayerError::TransactionInvalid(format!(
                        "deposit wallet transaction {} invalid",
                        transaction_id_for_error
                    )));
                }
                RelayerTransactionState::Failed => {
                    if let Some((owner, record)) = terminal_evidence {
                        self.record_terminal_observation(&transaction_id, &receipt)?;
                        match record.source {
                            OwnerTransactionSource::OwnerRecovery => {
                                self.record_recovered_ambiguous_transaction(owner, &transaction_id)?;
                            }
                            OwnerTransactionSource::LocalSubmit => {
                                self.mark_transaction_reconciliation_required(&transaction_id)?;
                            }
                        }
                    } else if let Some(owner) = owner_to_verify {
                        if trusted_owner_recovery {
                            self.record_recovered_ambiguous_transaction(owner, &transaction_id)?;
                            self.record_terminal_observation(&transaction_id, &receipt)?;
                        }
                    }
                    return Err(RelayerError::TransactionFailed(format!(
                        "deposit wallet transaction {} failed",
                        transaction_id_for_error
                    )));
                }
                RelayerTransactionState::Unknown(raw) => {
                    if let Some(owner) = owner_to_verify {
                        self.record_recovered_ambiguous_transaction(owner, &transaction_id)?;
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

        if let Some(owner) = expected_owner {
            self.mark_transaction_reconciliation_required(&transaction_id)?;
            return Err(RelayerError::reconciliation_required(format!(
                "owner-scoped transaction {} for owner {} did not reach a terminal state before poll timeout; owner-scoped repoll or manual reconciliation required",
                transaction_id_for_error,
                redacted_address(owner)
            )));
        }
        Err(RelayerError::Timeout)
    }

}

pub(super) fn is_transient_poll_error(error: &RelayerError) -> bool {
    match error {
        RelayerError::QuotaExhausted | RelayerError::Timeout | RelayerError::Http(_) => true,
        RelayerError::Api { status, .. } => {
            matches!(*status, 404 | 408 | 425 | 429) || (500..=599).contains(status)
        }
        RelayerError::Other(message) if message == RESPONSE_BODY_TOO_LARGE_MESSAGE => false,
        _ => false,
    }
}

pub(super) fn transaction_poll_jitter(transaction_id: &str, attempt: usize, base: Duration) -> Duration {
    let max_jitter_ms = (base.as_millis() / 4).min(250) as u64;
    if max_jitter_ms == 0 {
        return Duration::ZERO;
    }

    let transaction_id = transaction_id.as_bytes();
    let attempt = attempt.to_be_bytes();
    let mut input = [0u8; MAX_TRANSACTION_ID_LEN + std::mem::size_of::<usize>()];
    input[..transaction_id.len()].copy_from_slice(transaction_id);
    input[transaction_id.len()..transaction_id.len() + attempt.len()].copy_from_slice(&attempt);
    let digest = keccak256(&input[..transaction_id.len() + attempt.len()]);
    Duration::from_millis((u64::from(digest[0]) % max_jitter_ms) + 1)
}

pub(super) fn retry_after_poll_interval(
    transaction_id: &str,
    attempt: usize,
    policy_interval: Duration,
    retry_after: Duration,
) -> Duration {
    let retry_after = retry_after.min(MAX_RETRY_AFTER_INTERVAL);
    let base = retry_after.max(policy_interval);
    if retry_after <= policy_interval {
        return base;
    }

    base.saturating_add(transaction_poll_jitter(transaction_id, attempt, base))
        .min(MAX_RETRY_AFTER_INTERVAL.saturating_add(MAX_RETRY_AFTER_JITTER))
}
