use ethers::types::Address;

use super::{
    DepositWalletDryRunEvidence, DepositWalletRelayerClient, DepositWalletSubmitReceipt,
    RelayerMutationPermit, RelayerReadPermit, RelayerSubmitOutcome,
};
use crate::deposit_wallet::{RelayerTransactionState, WALLET_CREATE_TRANSACTION_TYPE};
use crate::error::{RelayerError, Result};

/// Selects whether a caller may enter the WALLET-CREATE deployment path.
///
/// `Predeployed` is the default consumer policy: callers must choose a policy
/// explicitly, and this type intentionally does not implement [`Default`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DepositWalletDeploymentPolicy {
    /// Treats wallets as predeployed and blocks the WALLET-CREATE path.
    Predeployed,
    /// Allows permit-gated WALLET-CREATE submission after a missing-wallet read.
    DeployIfMissing,
}

/// Result of entering the deposit-wallet deployment lifecycle.
///
/// Only [`DepositWalletDeploymentStatus::AlreadyDeployed`] records observed
/// deployment. Submit and dry-run outcomes are not wallet readiness.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DepositWalletDeploymentStatus {
    /// `GET /deployed` returned `true`; no mutation path was entered.
    AlreadyDeployed,
    /// WALLET-CREATE was submitted; preserve the receipt for reconciliation.
    CreateSubmitted(DepositWalletSubmitReceipt),
    /// A dry-run permit produced evidence without submitting WALLET-CREATE.
    CreateDryRun(Box<DepositWalletDryRunEvidence>),
}

/// Confirmed-only readiness result for a submitted WALLET-CREATE transaction.
///
/// Failed, invalid, unknown, ambiguous, or malformed responses are errors and
/// must be reconciled rather than used as authority for another deployment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DepositWalletReadiness {
    /// `STATE_CONFIRMED` was observed with the required transaction evidence.
    Ready,
    /// The transaction remains non-terminal and requires later polling.
    Pending(RelayerTransactionState),
}

impl DepositWalletRelayerClient {
    /// Checks deployment fact before optionally entering permit-gated WALLET-CREATE.
    ///
    /// The deployed read validates the read permit, derives the wallet, and
    /// validates the configured factory and chain before HTTP. If deployment is
    /// missing, `submit_wallet_create` owns operation, owner, chain, expiry,
    /// one-way latch, and mode validation; this method does not duplicate those
    /// checks.
    ///
    /// A submitted receipt is not readiness. Consumers must preserve its
    /// `transaction_id` and `payload_keccak256`, then call
    /// `check_deposit_wallet_deployment_readiness`. Do not call this method again
    /// for an owner with a pending create: owner-scoped intent enforcement is
    /// deferred to PBRSDK-11, and an early re-entry could duplicate WALLET-CREATE.
    pub async fn ensure_deposit_wallet_deployment(
        &self,
        owner: Address,
        policy: DepositWalletDeploymentPolicy,
        read_permit: &RelayerReadPermit,
        mutation_permit: Option<&RelayerMutationPermit>,
    ) -> Result<DepositWalletDeploymentStatus> {
        if self
            .is_deposit_wallet_deployed(owner, read_permit)
            .await?
        {
            return Ok(DepositWalletDeploymentStatus::AlreadyDeployed);
        }

        match policy {
            DepositWalletDeploymentPolicy::Predeployed => {
                Err(RelayerError::mutation_blocked(
                    "deposit wallet is not deployed and the predeployed policy forbids WALLET-CREATE; reconcile wallet provisioning before enabling deployment",
                ))
            }
            DepositWalletDeploymentPolicy::DeployIfMissing => {
                let permit = mutation_permit.ok_or_else(|| {
                    RelayerError::mutation_blocked(
                        "deployment requires an explicit mutation permit",
                    )
                })?;
                match self.submit_wallet_create(owner, permit).await? {
                    RelayerSubmitOutcome::DryRun(evidence) => {
                        Ok(DepositWalletDeploymentStatus::CreateDryRun(evidence))
                    }
                    RelayerSubmitOutcome::Submitted(receipt) => {
                        Ok(DepositWalletDeploymentStatus::CreateSubmitted(receipt))
                    }
                }
            }
        }
    }

    /// Performs one WALLET-CREATE transaction read and applies confirmed-only readiness.
    ///
    /// This method does not poll, sleep, retry, or resubmit. `Pending` must be
    /// handed to the PBRSDK-10 polling layer. Any error requires reconciliation
    /// and is not authority to re-enter deployment.
    pub async fn check_deposit_wallet_deployment_readiness(
        &self,
        owner: Address,
        transaction_id: &str,
        read_permit: &RelayerReadPermit,
    ) -> Result<DepositWalletReadiness> {
        let receipt = self
            .get_transaction_for_owner_with_expected_type(
                owner,
                transaction_id,
                read_permit,
                WALLET_CREATE_TRANSACTION_TYPE,
            )
            .await?;

        match receipt.state {
            RelayerTransactionState::Confirmed => Ok(DepositWalletReadiness::Ready),
            state @ (RelayerTransactionState::New
            | RelayerTransactionState::Executed
            | RelayerTransactionState::Mined) => Ok(DepositWalletReadiness::Pending(state)),
            _ => Err(RelayerError::reconciliation_required(
                "WALLET-CREATE readiness received an unvalidated terminal or unknown state; manual reconciliation required",
            )),
        }
    }
}
