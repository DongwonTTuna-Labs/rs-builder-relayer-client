use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use ethers::signers::Signer;
use ethers::types::{Address, U256};
use serde::{Deserialize, Serialize};

use super::clock::{RelayerClock, SystemClock};
use super::redaction::{redacted_address, sanitized_external_token};
use super::response::validate_transaction_id;
use super::{
    DepositWalletDeploymentPolicy, DepositWalletDeploymentStatus,
    DepositWalletRelayerClient, DepositWalletTransactionReceipt, RelayerMutationMode,
    RelayerMutationOperation, RelayerMutationPermit, RelayerPollOutcome, RelayerReadPermit,
    RelayerSubmitOutcome,
};
use crate::deposit_wallet::config::deposit_wallet_contract_chain_id;
use crate::deposit_wallet::{
    DepositWalletCall, DepositWalletRequestContext, RelayerTransactionState,
};
use crate::error::{RelayerError, Result};

const STALE_LEASE_ERROR: &str =
    "stale mutation intent lease; a newer write superseded this lease";
const TERMINAL_FAILURE_BINDING_ERROR: &str =
    "only bound terminal failures may resolve an intent; ambiguous or unknown errors require reconciliation";

/// Result of atomically attempting to begin an owner-scoped mutation intent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TryBeginOutcome {
    /// The store issued an epoch and persisted a revision-zero record.
    Started(MutationIntentRecord),
    /// An unresolved record already existed and no new record was written.
    Rejected(MutationIntentRecord),
}

/// Synchronous persistence boundary for owner-scoped mutation intents.
///
/// Implementations must make `try_begin` and `update` atomic. These methods are
/// intended for fast local storage. Implementors backed by blocking I/O must
/// keep that blocking work away from an async executor, for example by calling
/// the registry from `spawn_blocking` at the consumer boundary.
pub trait MutationIntentStore: Send + Sync {
    /// Loads the current record for one owner and chain.
    fn load(&self, owner: Address, chain_id: u64) -> Result<Option<MutationIntentRecord>>;

    /// Atomically starts a new intent or returns the unresolved current intent.
    ///
    /// The store must ignore the template's epoch and revision. In the same
    /// critical section that checks the current record, it assigns revision
    /// zero and epoch zero for the first record or the previous epoch plus one
    /// after a resolved record. Epoch overflow must fail closed.
    fn try_begin(&self, template: MutationIntentRecord) -> Result<TryBeginOutcome>;

    /// Replaces a record only when its epoch and revision match the stored row.
    ///
    /// The store must ignore the candidate record's revision and persist
    /// `expected_revision + 1`. Owner, chain, epoch, and creation time are
    /// immutable within one intent generation. Revision overflow must fail
    /// closed.
    fn update(
        &self,
        expected_epoch: u64,
        expected_revision: u64,
        record: MutationIntentRecord,
    ) -> Result<bool>;
}

/// Process-local mutation intent storage for tests and development only.
///
/// Records are lost on process restart. Live use requires a durable
/// [`MutationIntentStore`] implementation; the PBRSDK-24/25 live gates require
/// that durable implementation before this registry may protect live traffic.
#[derive(Default)]
pub struct InMemoryMutationIntentStore {
    records: Mutex<HashMap<(Address, u64), MutationIntentRecord>>,
}

impl InMemoryMutationIntentStore {
    fn records(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, HashMap<(Address, u64), MutationIntentRecord>>> {
        self.records.lock().map_err(|_| {
            RelayerError::Other("mutation intent store lock was poisoned".to_string())
        })
    }

    #[cfg(test)]
    pub(super) fn seed_for_test(&self, record: MutationIntentRecord) -> Result<()> {
        self.records()?
            .insert((record.owner, record.chain_id), record);
        Ok(())
    }
}

impl MutationIntentStore for InMemoryMutationIntentStore {
    fn load(&self, owner: Address, chain_id: u64) -> Result<Option<MutationIntentRecord>> {
        Ok(self.records()?.get(&(owner, chain_id)).cloned())
    }

    fn try_begin(&self, mut template: MutationIntentRecord) -> Result<TryBeginOutcome> {
        let key = (template.owner, template.chain_id);
        let mut records = self.records()?;

        let epoch = match records.get(&key) {
            Some(current) if current.status.is_unresolved() => {
                return Ok(TryBeginOutcome::Rejected(current.clone()));
            }
            Some(current) => current.epoch.checked_add(1).ok_or_else(|| {
                RelayerError::Other(
                    "mutation intent epoch exhausted; refusing to wrap generation".to_string(),
                )
            })?,
            None => 0,
        };

        template.epoch = epoch;
        template.revision = 0;
        records.insert(key, template.clone());
        Ok(TryBeginOutcome::Started(template))
    }

    fn update(
        &self,
        expected_epoch: u64,
        expected_revision: u64,
        mut record: MutationIntentRecord,
    ) -> Result<bool> {
        let next_revision = expected_revision.checked_add(1).ok_or_else(|| {
            RelayerError::Other(
                "mutation intent revision exhausted; refusing to wrap version".to_string(),
            )
        })?;
        let key = (record.owner, record.chain_id);
        let mut records = self.records()?;
        let Some(current) = records.get(&key) else {
            return Ok(false);
        };
        if current.owner != record.owner
            || current.chain_id != record.chain_id
            || current.epoch != expected_epoch
            || current.revision != expected_revision
        {
            return Ok(false);
        }

        record.created_at_unix = current.created_at_unix;
        record.epoch = expected_epoch;
        record.revision = next_revision;
        records.insert(key, record);
        Ok(true)
    }
}

/// Persisted state of one owner- and chain-scoped mutation generation.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MutationIntentRecord {
    owner: Address,
    chain_id: u64,
    epoch: u64,
    revision: u64,
    operation: RelayerMutationOperation,
    status: MutationIntentStatus,
    nonce: Option<String>,
    payload_keccak256: Option<String>,
    deadline_unix: Option<u64>,
    transaction_id: Option<String>,
    last_observed_state: Option<String>,
    created_at_unix: u64,
    updated_at_unix: u64,
}

impl MutationIntentRecord {
    fn preparing(
        owner: Address,
        chain_id: u64,
        operation: RelayerMutationOperation,
        now_unix: u64,
    ) -> Self {
        Self {
            owner,
            chain_id,
            epoch: 0,
            revision: 0,
            operation,
            status: MutationIntentStatus::Preparing,
            nonce: None,
            payload_keccak256: None,
            deadline_unix: None,
            transaction_id: None,
            last_observed_state: None,
            created_at_unix: now_unix,
            updated_at_unix: now_unix,
        }
    }

    pub fn owner(&self) -> Address {
        self.owner
    }

    pub fn chain_id(&self) -> u64 {
        self.chain_id
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn operation(&self) -> RelayerMutationOperation {
        self.operation
    }

    pub fn status(&self) -> MutationIntentStatus {
        self.status
    }

    pub fn nonce(&self) -> Option<&str> {
        self.nonce.as_deref()
    }

    pub fn payload_keccak256(&self) -> Option<&str> {
        self.payload_keccak256.as_deref()
    }

    pub fn deadline_unix(&self) -> Option<u64> {
        self.deadline_unix
    }

    pub fn transaction_id(&self) -> Option<&str> {
        self.transaction_id.as_deref()
    }

    pub fn last_observed_state(&self) -> Option<&str> {
        self.last_observed_state.as_deref()
    }

    pub fn created_at_unix(&self) -> u64 {
        self.created_at_unix
    }

    pub fn updated_at_unix(&self) -> u64 {
        self.updated_at_unix
    }
}

impl fmt::Debug for MutationIntentRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MutationIntentRecord")
            .field("owner", &redacted_address(self.owner))
            .field("chain_id", &self.chain_id)
            .field("epoch", &self.epoch)
            .field("revision", &self.revision)
            .field("operation", &self.operation)
            .field("status", &self.status)
            .field("nonce", &self.nonce)
            .field("payload_keccak256", &self.payload_keccak256)
            .field("deadline_unix", &self.deadline_unix)
            .field(
                "transaction_id",
                &self
                    .transaction_id
                    .as_deref()
                    .map(sanitized_external_token),
            )
            .field(
                "last_observed_state",
                &safe_observed_state_debug(self.last_observed_state.as_deref()),
            )
            .field("created_at_unix", &self.created_at_unix)
            .field("updated_at_unix", &self.updated_at_unix)
            .finish()
    }
}

/// State of an owner-scoped mutation intent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MutationIntentStatus {
    Preparing,
    Submitted,
    AmbiguousNoId,
    Confirmed,
    Failed,
    Reconciled,
}

impl MutationIntentStatus {
    fn is_unresolved(self) -> bool {
        matches!(
            self,
            Self::Preparing | Self::Submitted | Self::AmbiguousNoId
        )
    }

    fn label(self) -> &'static str {
        match self {
            Self::Preparing => "Preparing",
            Self::Submitted => "Submitted",
            Self::AmbiguousNoId => "AmbiguousNoId",
            Self::Confirmed => "Confirmed",
            Self::Failed => "Failed",
            Self::Reconciled => "Reconciled",
        }
    }
}

/// Coordinates owner-scoped mutation intents through a caller-provided store.
#[derive(Clone)]
pub struct OwnerMutationRegistry {
    store: Arc<dyn MutationIntentStore>,
    clock: Arc<dyn RelayerClock>,
}

impl OwnerMutationRegistry {
    pub fn new(store: Arc<dyn MutationIntentStore>) -> Self {
        Self {
            store,
            clock: Arc::new(SystemClock),
        }
    }

    #[cfg(test)]
    pub(super) fn with_clock(
        store: Arc<dyn MutationIntentStore>,
        clock: Arc<dyn RelayerClock>,
    ) -> Self {
        Self { store, clock }
    }

    pub fn gate<'a>(
        &'a self,
        client: &'a DepositWalletRelayerClient,
    ) -> IntentGatedClient<'a> {
        IntentGatedClient {
            client,
            registry: self,
        }
    }

    pub fn intent(
        &self,
        owner: Address,
        chain_id: u64,
    ) -> Result<Option<MutationIntentRecord>> {
        self.store.load(owner, chain_id)
    }

    /// Begins a write-ahead owner mutation intent before nonce, signing, or HTTP work.
    pub fn begin_intent(
        &self,
        owner: Address,
        chain_id: u64,
        operation: RelayerMutationOperation,
    ) -> Result<MutationIntentLease<'_>> {
        let now_unix = self.clock.now_unix();
        let template = MutationIntentRecord::preparing(owner, chain_id, operation, now_unix);
        match self.store.try_begin(template)? {
            TryBeginOutcome::Started(record) => Ok(MutationIntentLease {
                registry: self,
                expected_epoch: record.epoch,
                expected_revision: record.revision,
                record,
                unusable: false,
            }),
            TryBeginOutcome::Rejected(record) => Err(RelayerError::mutation_blocked(format!(
                "owner has an unresolved mutation intent (status {}); reconcile before starting another mutation",
                record.status.label()
            ))),
        }
    }

    /// Applies a poll result only to the submitted intent bound to the polled ID.
    ///
    /// Cancellation records nothing. Exhaustion records only a present last
    /// state and remains unresolved. A stale CAS is a benign no-op because a
    /// newer writer already superseded this observation.
    pub fn record_poll_outcome(
        &self,
        owner: Address,
        chain_id: u64,
        polled_transaction_id: &str,
        outcome: &RelayerPollOutcome,
    ) -> Result<()> {
        let Some(mut record) = self.store.load(owner, chain_id)? else {
            return Ok(());
        };
        if record.status != MutationIntentStatus::Submitted
            || record.transaction_id.as_deref() != Some(polled_transaction_id)
        {
            return Ok(());
        }

        match outcome {
            RelayerPollOutcome::Confirmed(receipt)
                if receipt.transaction_id == polled_transaction_id
                    && receipt.state == RelayerTransactionState::Confirmed =>
            {
                record.status = MutationIntentStatus::Confirmed;
                record.last_observed_state = Some(RelayerTransactionState::Confirmed.label());
            }
            RelayerPollOutcome::Confirmed(_) => return Ok(()),
            RelayerPollOutcome::Exhausted {
                last_state: Some(last_state),
                ..
            } => {
                record.last_observed_state = Some(last_state.label());
            }
            RelayerPollOutcome::Exhausted {
                last_state: None, ..
            }
            | RelayerPollOutcome::Cancelled { .. } => return Ok(()),
        }

        let expected_epoch = record.epoch;
        let expected_revision = record.revision;
        record.updated_at_unix = self.clock.now_unix();
        let _ = self
            .store
            .update(expected_epoch, expected_revision, record)?;
        Ok(())
    }

    /// Records a bound terminal polling failure without requiring a live lease.
    pub fn record_terminal_failure(
        &self,
        owner: Address,
        chain_id: u64,
        polled_transaction_id: &str,
        error: &RelayerError,
    ) -> Result<()> {
        let Some(mut record) = self.store.load(owner, chain_id)? else {
            return Ok(());
        };
        if record.status != MutationIntentStatus::Submitted
            || record.transaction_id.as_deref() != Some(polled_transaction_id)
        {
            return Ok(());
        }

        let observed_state = match error {
            RelayerError::TransactionFailed(_) => RelayerTransactionState::Failed.label(),
            RelayerError::TransactionInvalid(_) => RelayerTransactionState::Invalid.label(),
            _ => return Err(RelayerError::Other(TERMINAL_FAILURE_BINDING_ERROR.to_string())),
        };
        let expected_epoch = record.epoch;
        let expected_revision = record.revision;
        record.status = MutationIntentStatus::Failed;
        record.last_observed_state = Some(observed_state);
        record.updated_at_unix = self.clock.now_unix();
        let _ = self
            .store
            .update(expected_epoch, expected_revision, record)?;
        Ok(())
    }
}

/// A fenced handle for one mutation intent generation.
///
/// Dropping a lease does not resolve or delete its record. This is intentional:
/// a crash or abandoned caller leaves the owner blocked until authoritative
/// terminal evidence or reconciliation updates the durable store.
pub struct MutationIntentLease<'a> {
    registry: &'a OwnerMutationRegistry,
    record: MutationIntentRecord,
    expected_epoch: u64,
    expected_revision: u64,
    unusable: bool,
}

impl MutationIntentLease<'_> {
    /// Records the non-replayable payload digest while the intent is preparing.
    pub fn record_payload(
        &mut self,
        payload_keccak256: &str,
        deadline: Option<U256>,
    ) -> Result<()> {
        self.ensure_status(MutationIntentStatus::Preparing, "record payload")?;
        validate_payload_keccak256(payload_keccak256)?;
        let mut next = self.record.clone();
        next.payload_keccak256 = Some(payload_keccak256.to_string());
        next.deadline_unix = deadline.map(saturating_u256_to_u64);
        self.persist(next)
    }

    /// Records a validated relayer transaction ID after submit.
    pub fn record_submitted(&mut self, transaction_id: &str) -> Result<()> {
        self.ensure_status(MutationIntentStatus::Preparing, "record submitted transaction")?;
        let transaction_id = validate_transaction_id(transaction_id)?;
        let mut next = self.record.clone();
        next.status = MutationIntentStatus::Submitted;
        next.transaction_id = Some(transaction_id);
        self.persist(next)
    }

    /// Records a transaction-bound receipt while the intent is submitted.
    ///
    /// Valid receipts structurally carry only pending states or Confirmed;
    /// Failed, Invalid, and Unknown are converted to errors by the existing
    /// transaction parser and must use registry-level terminal handling.
    pub fn record_observed_receipt(
        &mut self,
        receipt: &DepositWalletTransactionReceipt,
    ) -> Result<()> {
        self.ensure_status(MutationIntentStatus::Submitted, "record observed receipt")?;
        if self.record.transaction_id.as_deref() != Some(receipt.transaction_id.as_str()) {
            return Err(RelayerError::Other(
                "observed receipt does not match this intent's transaction".to_string(),
            ));
        }

        let mut next = self.record.clone();
        match &receipt.state {
            RelayerTransactionState::Confirmed => {
                next.status = MutationIntentStatus::Confirmed;
                next.last_observed_state = Some(receipt.state.label());
            }
            RelayerTransactionState::New
            | RelayerTransactionState::Executed
            | RelayerTransactionState::Mined => {
                next.last_observed_state = Some(receipt.state.label());
            }
            RelayerTransactionState::Failed
            | RelayerTransactionState::Invalid
            | RelayerTransactionState::Unknown(_) => {
                return Err(RelayerError::Other(
                    "observed receipt contained a state that requires terminal failure or reconciliation handling"
                        .to_string(),
                ));
            }
        }
        self.persist(next)
    }

    /// Blocks the owner after a submit may have been dispatched without an ID.
    pub fn record_ambiguous_without_id(&mut self) -> Result<()> {
        self.ensure_status(
            MutationIntentStatus::Preparing,
            "record ambiguous submit without transaction id",
        )?;
        let mut next = self.record.clone();
        next.status = MutationIntentStatus::AmbiguousNoId;
        self.persist(next)
    }

    /// Resolves a local failure proven to have happened before submit dispatch.
    pub fn abandon_before_submit(&mut self) -> Result<()> {
        self.ensure_status(
            MutationIntentStatus::Preparing,
            "abandon intent before submit",
        )?;
        let mut next = self.record.clone();
        next.status = MutationIntentStatus::Failed;
        next.last_observed_state = None;
        self.persist(next)
    }

    fn ensure_status(&self, required: MutationIntentStatus, action: &str) -> Result<()> {
        if self.unusable {
            return Err(stale_lease_error());
        }
        if self.record.status != required {
            return Err(RelayerError::Other(format!(
                "invalid mutation intent transition: {action} requires {} but current status is {}",
                required.label(),
                self.record.status.label()
            )));
        }
        Ok(())
    }

    fn persist(&mut self, mut next: MutationIntentRecord) -> Result<()> {
        if self.unusable {
            return Err(stale_lease_error());
        }
        next.updated_at_unix = self.registry.clock.now_unix();
        match self.registry.store.update(
            self.expected_epoch,
            self.expected_revision,
            next.clone(),
        ) {
            Ok(true) => {
                let next_revision = self.expected_revision.checked_add(1).ok_or_else(|| {
                    self.unusable = true;
                    RelayerError::Other(
                        "mutation intent revision exhausted; refusing to wrap version".to_string(),
                    )
                })?;
                next.epoch = self.expected_epoch;
                next.revision = next_revision;
                self.expected_revision = next_revision;
                self.record = next;
                Ok(())
            }
            Ok(false) => {
                self.unusable = true;
                Err(stale_lease_error())
            }
            Err(error) => {
                self.unusable = true;
                Err(error)
            }
        }
    }
}

/// A deposit-wallet client view that requires the registry before live mutation.
pub struct IntentGatedClient<'a> {
    client: &'a DepositWalletRelayerClient,
    registry: &'a OwnerMutationRegistry,
}

impl IntentGatedClient<'_> {
    pub async fn execute_wallet_batch<S: Signer>(
        &self,
        ctx: DepositWalletRequestContext,
        calls: Vec<DepositWalletCall>,
        deadline: U256,
        signer: &S,
        read_permit: &RelayerReadPermit,
        mutation_permit: &RelayerMutationPermit,
    ) -> Result<RelayerSubmitOutcome> {
        if mutation_permit.mode() == RelayerMutationMode::DryRun {
            return self
                .client
                .execute_wallet_batch(
                    ctx,
                    calls,
                    deadline,
                    signer,
                    read_permit,
                    mutation_permit,
                )
                .await;
        }

        let owner = ctx.owner_address;
        let chain_id = deposit_wallet_contract_chain_id(self.client.config)?;
        let mut lease = self.registry.begin_intent(
            owner,
            chain_id,
            RelayerMutationOperation::WalletBatch,
        )?;
        let outcome = self
            .client
            .execute_wallet_batch(
                ctx,
                calls,
                deadline,
                signer,
                read_permit,
                mutation_permit,
            )
            .await;
        finish_submit_outcome(&mut lease, outcome, Some(deadline))
    }

    pub async fn submit_wallet_create(
        &self,
        owner: Address,
        mutation_permit: &RelayerMutationPermit,
    ) -> Result<RelayerSubmitOutcome> {
        if mutation_permit.mode() == RelayerMutationMode::DryRun {
            return self
                .client
                .submit_wallet_create(owner, mutation_permit)
                .await;
        }

        let chain_id = deposit_wallet_contract_chain_id(self.client.config)?;
        let mut lease = self.registry.begin_intent(
            owner,
            chain_id,
            RelayerMutationOperation::WalletCreate,
        )?;
        let outcome = self
            .client
            .submit_wallet_create(owner, mutation_permit)
            .await;
        finish_submit_outcome(&mut lease, outcome, None)
    }

    pub async fn ensure_deposit_wallet_deployment(
        &self,
        owner: Address,
        policy: DepositWalletDeploymentPolicy,
        read_permit: &RelayerReadPermit,
        mutation_permit: Option<&RelayerMutationPermit>,
    ) -> Result<DepositWalletDeploymentStatus> {
        if self
            .client
            .is_deposit_wallet_deployed(owner, read_permit)
            .await?
        {
            return Ok(DepositWalletDeploymentStatus::AlreadyDeployed);
        }

        let live_create = policy == DepositWalletDeploymentPolicy::DeployIfMissing
            && matches!(
                mutation_permit,
                Some(permit) if permit.mode() == RelayerMutationMode::Live
            );
        if !live_create {
            return self
                .client
                .ensure_deposit_wallet_deployment(
                    owner,
                    policy,
                    read_permit,
                    mutation_permit,
                )
                .await;
        }

        let chain_id = deposit_wallet_contract_chain_id(self.client.config)?;
        let mut lease = self.registry.begin_intent(
            owner,
            chain_id,
            RelayerMutationOperation::WalletCreate,
        )?;
        let outcome = self
            .client
            .ensure_deposit_wallet_deployment(
                owner,
                policy,
                read_permit,
                mutation_permit,
            )
            .await;

        match outcome {
            Ok(DepositWalletDeploymentStatus::CreateSubmitted(receipt)) => {
                lease.record_payload(receipt.payload_keccak256(), None)?;
                lease.record_submitted(receipt.transaction_id())?;
                Ok(DepositWalletDeploymentStatus::CreateSubmitted(receipt))
            }
            Ok(DepositWalletDeploymentStatus::AlreadyDeployed) => {
                lease.abandon_before_submit()?;
                Ok(DepositWalletDeploymentStatus::AlreadyDeployed)
            }
            Ok(DepositWalletDeploymentStatus::CreateDryRun(_)) => {
                lease.abandon_before_submit()?;
                Err(unexpected_live_dry_run_error())
            }
            Err(error) => {
                record_delegated_error(&mut lease, &error)?;
                Err(error)
            }
        }
    }
}

fn finish_submit_outcome(
    lease: &mut MutationIntentLease<'_>,
    outcome: Result<RelayerSubmitOutcome>,
    deadline: Option<U256>,
) -> Result<RelayerSubmitOutcome> {
    match outcome {
        Ok(RelayerSubmitOutcome::Submitted(receipt)) => {
            lease.record_payload(receipt.payload_keccak256(), deadline)?;
            lease.record_submitted(receipt.transaction_id())?;
            Ok(RelayerSubmitOutcome::Submitted(receipt))
        }
        Ok(RelayerSubmitOutcome::DryRun(_)) => {
            lease.abandon_before_submit()?;
            Err(unexpected_live_dry_run_error())
        }
        Err(error) => {
            record_delegated_error(lease, &error)?;
            Err(error)
        }
    }
}

fn record_delegated_error(
    lease: &mut MutationIntentLease<'_>,
    error: &RelayerError,
) -> Result<()> {
    if error.is_deposit_wallet_reconciliation_required()
        || matches!(error, RelayerError::Api { .. })
    {
        lease.record_ambiguous_without_id()
    } else {
        lease.abandon_before_submit()
    }
}

fn validate_payload_keccak256(value: &str) -> Result<()> {
    let Some(hex_value) = value.strip_prefix("0x") else {
        return Err(RelayerError::Other(
            "mutation intent payload digest must be a 0x-prefixed keccak256 hash".to_string(),
        ));
    };
    if hex_value.len() != 64 || !hex_value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(RelayerError::Other(
            "mutation intent payload digest must be a 32-byte hexadecimal hash".to_string(),
        ));
    }
    Ok(())
}

fn saturating_u256_to_u64(value: U256) -> u64 {
    if value > U256::from(u64::MAX) {
        u64::MAX
    } else {
        value.as_u64()
    }
}

fn safe_observed_state_debug(value: Option<&str>) -> Option<&str> {
    match value {
        Some("New" | "Executed" | "Mined" | "Confirmed" | "Invalid" | "Failed") => value,
        Some(_) => Some("<unrecognized relayer state>"),
        None => None,
    }
}

fn stale_lease_error() -> RelayerError {
    RelayerError::Other(STALE_LEASE_ERROR.to_string())
}

fn unexpected_live_dry_run_error() -> RelayerError {
    RelayerError::Other("unexpected dry-run outcome under a live permit".to_string())
}
