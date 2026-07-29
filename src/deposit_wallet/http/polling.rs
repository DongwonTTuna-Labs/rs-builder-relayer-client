use std::future::Future;
use std::time::Duration;

use ethers::types::Address;

use super::{
    DepositWalletRelayerClient, DepositWalletTransactionReceipt, RelayerReadPermit,
};
use crate::deposit_wallet::{
    RelayerTransactionState, WALLET_CREATE_TRANSACTION_TYPE, WALLET_TRANSACTION_TYPE,
};
use crate::error::{RelayerError, Result};

const MIN_POLL_INTERVAL: Duration = Duration::from_millis(1);
const MAX_POLL_INTERVAL: Duration = Duration::from_secs(600);
const MAX_POLL_ATTEMPTS: u32 = 100;

/// Finite polling limits and the fixed exponential-backoff interval range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RelayerPollPolicy {
    max_attempts: u32,
    initial_interval: Duration,
    max_interval: Duration,
}

impl RelayerPollPolicy {
    /// Creates a bounded polling policy.
    ///
    /// Attempts must be in `1..=100`, the initial interval must be at least one
    /// millisecond and no greater than the maximum interval, and the maximum
    /// interval must not exceed 600 seconds.
    pub fn try_new(
        max_attempts: u32,
        initial_interval: Duration,
        max_interval: Duration,
    ) -> Result<Self> {
        if !(1..=MAX_POLL_ATTEMPTS).contains(&max_attempts) {
            return Err(invalid_poll_policy(format!(
                "max_attempts must be in 1..={MAX_POLL_ATTEMPTS}"
            )));
        }
        if initial_interval < MIN_POLL_INTERVAL {
            return Err(invalid_poll_policy(
                "initial_interval must be at least 1ms",
            ));
        }
        if max_interval > MAX_POLL_INTERVAL {
            return Err(invalid_poll_policy(
                "max_interval must not exceed 600s",
            ));
        }
        if initial_interval > max_interval {
            return Err(invalid_poll_policy(
                "initial_interval must not exceed max_interval",
            ));
        }

        Ok(Self {
            max_attempts,
            initial_interval,
            max_interval,
        })
    }

    /// Returns the maximum number of transaction reads.
    pub fn max_attempts(&self) -> u32 {
        self.max_attempts
    }

    /// Returns the interval before the second attempt.
    pub fn initial_interval(&self) -> Duration {
        self.initial_interval
    }

    /// Returns the upper bound applied to doubled polling intervals.
    pub fn max_interval(&self) -> Duration {
        self.max_interval
    }
}

/// Confirmed-only result of bounded relayer transaction polling.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RelayerPollOutcome {
    /// `STATE_CONFIRMED` was observed with all required transaction evidence.
    Confirmed(DepositWalletTransactionReceipt),
    /// Attempts were exhausted. This does not authorize signing or submission.
    Exhausted {
        attempts: u32,
        last_state: Option<RelayerTransactionState>,
    },
    /// The caller cancelled polling. This does not authorize signing or submission.
    Cancelled { attempts: u32 },
}

impl DepositWalletRelayerClient {
    /// Polls a WALLET transaction under a finite, confirmed-only policy.
    ///
    /// Only [`RelayerPollOutcome::Confirmed`] is evidence that effects may be
    /// trusted. Exhaustion, cancellation, and every error are not authority to
    /// re-sign or resubmit; preserve the original transaction identity for
    /// reconciliation. Pass [`std::future::pending::<()>`] when cancellation is
    /// not required.
    pub async fn poll_wallet_transaction(
        &self,
        owner: Address,
        transaction_id: &str,
        policy: RelayerPollPolicy,
        read_permit: &RelayerReadPermit,
        cancel: impl Future<Output = ()> + Send,
    ) -> Result<RelayerPollOutcome> {
        self.poll_with_expected_type(
            owner,
            transaction_id,
            policy,
            read_permit,
            WALLET_TRANSACTION_TYPE,
            cancel,
        )
        .await
    }

    /// Polls a WALLET-CREATE transaction under a finite, confirmed-only policy.
    ///
    /// Only [`RelayerPollOutcome::Confirmed`] is evidence that deployment may be
    /// trusted. Exhaustion, cancellation, and every error are not authority to
    /// deploy again; preserve the original transaction identity for
    /// reconciliation. Pass [`std::future::pending::<()>`] when cancellation is
    /// not required.
    pub async fn poll_deposit_wallet_deployment(
        &self,
        owner: Address,
        transaction_id: &str,
        policy: RelayerPollPolicy,
        read_permit: &RelayerReadPermit,
        cancel: impl Future<Output = ()> + Send,
    ) -> Result<RelayerPollOutcome> {
        self.poll_with_expected_type(
            owner,
            transaction_id,
            policy,
            read_permit,
            WALLET_CREATE_TRANSACTION_TYPE,
            cancel,
        )
        .await
    }

    async fn poll_with_expected_type(
        &self,
        owner: Address,
        transaction_id: &str,
        policy: RelayerPollPolicy,
        read_permit: &RelayerReadPermit,
        expected_type: &str,
        cancel: impl Future<Output = ()> + Send,
    ) -> Result<RelayerPollOutcome> {
        tokio::pin!(cancel);

        let mut last_state = None;
        let mut interval = policy.initial_interval();
        for attempt in 1..=policy.max_attempts() {
            let result = tokio::select! {
                biased;
                () = &mut cancel => {
                    return Ok(RelayerPollOutcome::Cancelled {
                        attempts: attempt - 1,
                    });
                }
                result = self.get_transaction_for_owner_with_expected_type(
                    owner,
                    transaction_id,
                    read_permit,
                    expected_type,
                ) => result,
            };

            match result {
                Ok(receipt) => match receipt.state.clone() {
                    RelayerTransactionState::Confirmed => {
                        return Ok(RelayerPollOutcome::Confirmed(receipt));
                    }
                    state @ (RelayerTransactionState::New
                    | RelayerTransactionState::Executed
                    | RelayerTransactionState::Mined) => {
                        last_state = Some(state);
                    }
                    _ => {
                        return Err(RelayerError::reconciliation_required(format!(
                            "{expected_type} polling received an unvalidated terminal or unknown state; manual reconciliation required"
                        )));
                    }
                },
                Err(error) if is_transient_poll_error(&error) => {}
                Err(error) => return Err(error),
            }

            if attempt == policy.max_attempts() {
                return Ok(RelayerPollOutcome::Exhausted {
                    attempts: attempt,
                    last_state,
                });
            }

            tokio::select! {
                biased;
                () = &mut cancel => {
                    return Ok(RelayerPollOutcome::Cancelled { attempts: attempt });
                }
                () = tokio::time::sleep(interval) => {}
            }
            interval = interval
                .checked_mul(2)
                .unwrap_or(policy.max_interval())
                .min(policy.max_interval());
        }

        Err(RelayerError::reconciliation_required(
            "bounded poll loop ended without an outcome; manual reconciliation required",
        ))
    }
}

fn invalid_poll_policy(message: impl Into<String>) -> RelayerError {
    RelayerError::Other(format!("invalid poll policy: {}", message.into()))
}

fn is_transient_poll_error(error: &RelayerError) -> bool {
    matches!(error, RelayerError::Http(_) | RelayerError::Api { .. })
        || error.is_deposit_wallet_transaction_absent()
}
