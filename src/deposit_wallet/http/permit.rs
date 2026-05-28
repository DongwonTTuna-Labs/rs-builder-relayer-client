use super::*;
#[cfg(test)]
use super::redaction::payload_hash_summary;
use super::redaction::{display_payload_hash, redacted_address, sanitized_external_token};
use super::response::{validate_transaction_hash, validate_transaction_id};
use super::MAX_ERROR_TOKEN_LEN;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DepositWalletMutationEnvironment {
    Production,
    /// Crate-local mock relayer environment used by unit tests.
    ///
    /// Production builds cannot construct a matching `DepositWalletRelayerUrl`;
    /// public production URLs always produce [`Self::Production`] scopes and
    /// reject permits carrying this environment.
    TestLoopback,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DepositWalletMutationAction {
    WalletNonceRead,
    WalletCreate,
    WalletBatch,
    OwnerRecoveryPoll,
    ManualReconciliation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DepositWalletMutationScope {
    chain_id: u64,
    factory: Address,
    implementation: Address,
    environment: DepositWalletMutationEnvironment,
    action: DepositWalletMutationAction,
}

impl DepositWalletMutationScope {
    pub(crate) fn new(
        chain_id: u64,
        factory: Address,
        implementation: Address,
        environment: DepositWalletMutationEnvironment,
        action: DepositWalletMutationAction,
    ) -> Self {
        Self {
            chain_id,
            factory,
            implementation,
            environment,
            action,
        }
    }

    pub fn action(&self) -> DepositWalletMutationAction {
        self.action
    }

    pub fn chain_id(&self) -> u64 {
        self.chain_id
    }

    pub fn factory(&self) -> Address {
        self.factory
    }

    pub fn implementation(&self) -> Address {
        self.implementation
    }

    pub fn environment(&self) -> DepositWalletMutationEnvironment {
        self.environment
    }
}

#[derive(Clone, Default, PartialEq, Eq)]
pub enum DepositWalletMutationGate {
    #[default]
    Deny,
    Permit(DepositWalletMutationPermit),
}

impl fmt::Debug for DepositWalletMutationGate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Deny => f.write_str("Deny"),
            Self::Permit(permit) => f.debug_tuple("Permit").field(permit).finish(),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct DepositWalletMutationPermit {
    pub(super) owner: Address,
    pub(super) reason: String,
    pub(super) owner_serialization_evidence: DepositWalletOwnerSerializationEvidence,
}

impl DepositWalletMutationPermit {
    /// Creates an explicit owner-scoped permit for test-loopback mutations.
    ///
    /// The evidence must come from a caller-side owner lock, nonce lease, or
    /// actor queue that prevents concurrent WALLET-CREATE/WALLET submits for the
    /// same owner. The crate validates the evidence shape and expiry before
    /// request construction.
    ///
    /// Production nonce-read, POST mutation, owner-recovery, and manual-clear
    /// permits are intentionally not publicly constructible in this PR because
    /// caller-attested evidence cannot prove a durable owner lease across
    /// processes. Future production signing must preserve a crate-trusted nonce
    /// lease with [`DepositWalletNonceLease`].
    /// A later live-submit PR must add durable owner state and a crate-owned
    /// trusted capability before production POST /submit or manual clear can be
    /// enabled.
    pub fn from_owner_serialization_evidence(
        reason: impl Into<String>,
        owner_serialization_evidence: DepositWalletOwnerSerializationEvidence,
    ) -> Result<Self> {
        if production_scope_requires_trusted_capability(owner_serialization_evidence.scope) {
            return Err(RelayerError::mutation_blocked(
                "production deposit-wallet mutation permits require durable owner state and a crate-owned trusted capability; no public production permit construction is enabled in this PR".to_string(),
            ));
        }
        let reason = reason.into();
        if reason.trim().is_empty() {
            return Err(RelayerError::mutation_blocked(
                "explicit deposit-wallet mutation permit reason required".to_string(),
            ));
        }
        Ok(Self {
            owner: owner_serialization_evidence.owner,
            reason,
            owner_serialization_evidence,
        })
    }

    pub(super) fn expires_at_unix_seconds(&self) -> u64 {
        self.owner_serialization_evidence
            .expires_at_unix_seconds()
    }
}

fn production_scope_requires_trusted_capability(scope: DepositWalletMutationScope) -> bool {
    scope.environment == DepositWalletMutationEnvironment::Production
}

impl fmt::Debug for DepositWalletMutationPermit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DepositWalletMutationPermit")
            .field("owner", &redacted_address(self.owner))
            .field("reason", &"<redacted>")
            .field("owner_serialization_evidence", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct DepositWalletOwnerSerializationEvidence {
    owner: Address,
    scope: DepositWalletMutationScope,
    issuer: String,
    lease_id_hash: String,
    acquired_at_unix_seconds: u64,
    expires_at_unix_seconds: u64,
}

#[derive(Clone, PartialEq, Eq)]
pub struct DepositWalletSubmitReconciliationEvidence {
    owner: Address,
    scope: DepositWalletMutationScope,
    issuer: String,
    payload_hash: String,
    observation: DepositWalletSubmitReconciliationObservation,
}

#[derive(Clone, PartialEq, Eq)]
pub struct DepositWalletIdlessSubmitReconciliationEvidence {
    owner: Address,
    scope: DepositWalletMutationScope,
    issuer: String,
    payload_hash: String,
    reason: String,
    checked_at_unix_seconds: u64,
}

#[derive(Clone, PartialEq, Eq)]
pub struct DepositWalletSubmitReconciliationObservation {
    transaction_id: String,
    observed_state: RelayerTransactionState,
    transaction_hash: Option<String>,
    reason: String,
    checked_at_unix_seconds: u64,
}

impl DepositWalletSubmitReconciliationObservation {
    pub fn new(
        transaction_id: impl AsRef<str>,
        observed_state: RelayerTransactionState,
        transaction_hash: Option<impl AsRef<str>>,
        reason: impl Into<String>,
        checked_at_unix_seconds: u64,
    ) -> Result<Self> {
        let transaction_id = validate_transaction_id(transaction_id.as_ref()).map_err(|_| {
            RelayerError::mutation_blocked(
                "submit reconciliation evidence transaction id is invalid".to_string(),
            )
        })?;
        let mut transaction_hash = transaction_hash
            .map(|hash| {
                validate_transaction_hash(hash.as_ref()).map_err(|_| {
                    RelayerError::mutation_blocked(
                        "submit reconciliation evidence transaction hash is invalid".to_string(),
                    )
                })
            })
            .transpose()?;
        match observed_state {
            RelayerTransactionState::Confirmed => {
                if transaction_hash.is_none() {
                    return Err(RelayerError::mutation_blocked(
                        "confirmed submit reconciliation evidence requires transaction hash"
                            .to_string(),
                    ));
                }
            }
            RelayerTransactionState::Invalid | RelayerTransactionState::Failed => {
                transaction_hash = None;
            }
            RelayerTransactionState::New
            | RelayerTransactionState::Executed
            | RelayerTransactionState::Mined
            | RelayerTransactionState::Unknown(_) => {
                return Err(RelayerError::mutation_blocked(
                    "submit reconciliation evidence requires a terminal observed state"
                        .to_string(),
                ));
            }
        }
        let reason = reason.into();
        if reason.trim().is_empty() {
            return Err(RelayerError::mutation_blocked(
                "submit reconciliation evidence reason required".to_string(),
            ));
        }
        if checked_at_unix_seconds == 0 {
            return Err(RelayerError::mutation_blocked(
                "submit reconciliation evidence check timestamp required".to_string(),
            ));
        }
        Ok(Self {
            transaction_id,
            observed_state,
            transaction_hash,
            reason,
            checked_at_unix_seconds,
        })
    }
}

impl fmt::Debug for DepositWalletSubmitReconciliationObservation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DepositWalletSubmitReconciliationObservation")
            .field("transaction_id", &sanitized_external_token(&self.transaction_id))
            .field("observed_state", &self.observed_state)
            .field("transaction_hash", &self.transaction_hash.as_deref().map(sanitized_external_token))
            .field("reason", &"<redacted>")
            .field("checked_at_unix_seconds", &self.checked_at_unix_seconds)
            .finish()
    }
}

impl DepositWalletSubmitReconciliationEvidence {
    /// Records audited evidence that an ambiguous submit payload was manually
    /// reconciled outside this client before clearing the owner block.
    pub fn new(
        owner: Address,
        scope: DepositWalletMutationScope,
        issuer: impl Into<String>,
        payload_hash: impl Into<String>,
        observation: DepositWalletSubmitReconciliationObservation,
    ) -> Result<Self> {
        if scope.action != DepositWalletMutationAction::ManualReconciliation {
            return Err(RelayerError::mutation_blocked(
                "submit reconciliation evidence scope must be manual reconciliation".to_string(),
            ));
        }
        let issuer = issuer.into();
        if issuer.trim().is_empty() {
            return Err(RelayerError::mutation_blocked(
                "submit reconciliation evidence issuer required".to_string(),
            ));
        }
        let payload_hash = validate_reconciliation_payload_hash(
            payload_hash.into(),
            "submit reconciliation evidence payload hash is invalid",
        )?;
        Ok(Self {
            owner,
            scope,
            issuer,
            payload_hash,
            observation,
        })
    }

    pub fn owner(&self) -> Address {
        self.owner
    }

    pub fn payload_hash(&self) -> &str {
        &self.payload_hash
    }

    pub fn transaction_id(&self) -> &str {
        &self.observation.transaction_id
    }

    pub(super) fn observed_state(&self) -> &RelayerTransactionState {
        &self.observation.observed_state
    }

    pub(super) fn transaction_hash(&self) -> Option<&str> {
        self.observation.transaction_hash.as_deref()
    }
}

impl fmt::Debug for DepositWalletSubmitReconciliationEvidence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DepositWalletSubmitReconciliationEvidence")
            .field("owner", &redacted_address(self.owner))
            .field("scope", &self.scope)
            .field("issuer", &"<redacted>")
            .field("payload_hash", &display_payload_hash(&self.payload_hash))
            .field("observation", &self.observation)
            .finish()
    }
}

impl DepositWalletIdlessSubmitReconciliationEvidence {
    /// Records audited evidence that an ambiguous submit without a local
    /// transaction id was not accepted by the relayer before clearing the
    /// owner block.
    pub fn new(
        owner: Address,
        scope: DepositWalletMutationScope,
        issuer: impl Into<String>,
        payload_hash: impl Into<String>,
        reason: impl Into<String>,
        checked_at_unix_seconds: u64,
    ) -> Result<Self> {
        if scope.action != DepositWalletMutationAction::ManualReconciliation {
            return Err(RelayerError::mutation_blocked(
                "id-less submit reconciliation evidence scope must be manual reconciliation"
                    .to_string(),
            ));
        }
        let issuer = issuer.into();
        if issuer.trim().is_empty() {
            return Err(RelayerError::mutation_blocked(
                "id-less submit reconciliation evidence issuer required".to_string(),
            ));
        }
        let payload_hash = validate_reconciliation_payload_hash(
            payload_hash.into(),
            "id-less submit reconciliation evidence payload hash is invalid",
        )?;
        let reason = reason.into();
        if reason.trim().is_empty() {
            return Err(RelayerError::mutation_blocked(
                "id-less submit reconciliation evidence reason required".to_string(),
            ));
        }
        if checked_at_unix_seconds == 0 {
            return Err(RelayerError::mutation_blocked(
                "id-less submit reconciliation evidence check timestamp required".to_string(),
            ));
        }
        Ok(Self {
            owner,
            scope,
            issuer,
            payload_hash,
            reason,
            checked_at_unix_seconds,
        })
    }

    pub fn owner(&self) -> Address {
        self.owner
    }

    pub fn payload_hash(&self) -> &str {
        &self.payload_hash
    }
}

fn validate_reconciliation_payload_hash(payload_hash: String, error_message: &str) -> Result<String> {
    let Some(canonical) = canonical_reconciliation_payload_hash(&payload_hash) else {
        return Err(RelayerError::mutation_blocked(error_message.to_string()));
    };
    if canonical.len() > MAX_ERROR_TOKEN_LEN {
        return Err(RelayerError::mutation_blocked(error_message.to_string()));
    }
    Ok(canonical)
}

fn canonical_reconciliation_payload_hash(payload_hash: &str) -> Option<String> {
    let (prefix, raw_hex) = payload_hash
        .strip_prefix("signed-digest:0x")
        .map(|hex| ("signed-digest:", hex))
        .or_else(|| payload_hash.strip_prefix("recovered:0x").map(|hex| ("recovered:", hex)))
        .or_else(|| payload_hash.strip_prefix("0x").map(|hex| ("", hex)))?;

    if raw_hex.len() == 64 && raw_hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Some(format!("{prefix}0x{}", raw_hex.to_ascii_lowercase()))
    } else {
        None
    }
}

impl fmt::Debug for DepositWalletIdlessSubmitReconciliationEvidence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DepositWalletIdlessSubmitReconciliationEvidence")
            .field("owner", &redacted_address(self.owner))
            .field("scope", &self.scope)
            .field("issuer", &"<redacted>")
            .field("payload_hash", &display_payload_hash(&self.payload_hash))
            .field("reason", &"<redacted>")
            .field("checked_at_unix_seconds", &self.checked_at_unix_seconds)
            .finish()
    }
}

impl DepositWalletOwnerSerializationEvidence {
    /// Records crate-side proof that same-owner submit work is serialized.
    ///
    /// `lease_id` is hashed before storage so debug output and errors never
    /// expose raw lock keys, queue ids, or database lease identifiers.
    #[cfg(test)]
    pub(super) fn new(
        owner: Address,
        scope: DepositWalletMutationScope,
        issuer: impl Into<String>,
        lease_id: impl AsRef<[u8]>,
        acquired_at_unix_seconds: u64,
        expires_at_unix_seconds: u64,
    ) -> Result<Self> {
        if acquired_at_unix_seconds == 0 {
            return Err(RelayerError::mutation_blocked(
                "owner serialization evidence acquisition timestamp required".to_string(),
            ));
        }
        let issuer = issuer.into();
        if issuer.trim().is_empty() {
            return Err(RelayerError::mutation_blocked(
                "owner serialization evidence issuer required".to_string(),
            ));
        }
        let lease_id = lease_id.as_ref();
        if lease_id.is_empty() {
            return Err(RelayerError::mutation_blocked(
                "owner serialization evidence lease id required".to_string(),
            ));
        }
        if expires_at_unix_seconds <= acquired_at_unix_seconds {
            return Err(RelayerError::mutation_blocked(
                "owner serialization evidence must expire after acquisition".to_string(),
            ));
        }
        if expires_at_unix_seconds - acquired_at_unix_seconds
            > MAX_OWNER_SERIALIZATION_LEASE_SECONDS
        {
            return Err(RelayerError::mutation_blocked(format!(
                "owner serialization evidence lease must not exceed {MAX_OWNER_SERIALIZATION_LEASE_SECONDS} seconds"
            )));
        }
        Ok(Self {
            owner,
            scope,
            issuer,
            lease_id_hash: payload_hash_summary(lease_id),
            acquired_at_unix_seconds,
            expires_at_unix_seconds,
        })
    }

    pub fn owner(&self) -> Address {
        self.owner
    }

    pub fn scope(&self) -> DepositWalletMutationScope {
        self.scope
    }

    pub fn expires_at_unix_seconds(&self) -> u64 {
        self.expires_at_unix_seconds
    }
}

impl fmt::Debug for DepositWalletOwnerSerializationEvidence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DepositWalletOwnerSerializationEvidence")
            .field("owner", &redacted_address(self.owner))
            .field("scope", &self.scope)
            .field("issuer", &"<redacted>")
            .field("lease_id_hash", &display_payload_hash(&self.lease_id_hash))
            .field("acquired_at_unix_seconds", &self.acquired_at_unix_seconds)
            .field("expires_at_unix_seconds", &self.expires_at_unix_seconds)
            .finish()
    }
}

pub(super) fn validate_permit_owner(permit: &DepositWalletMutationPermit, owner: Address) -> Result<()> {
    if permit.owner != permit.owner_serialization_evidence.owner {
        return Err(RelayerError::mutation_blocked(
            "deposit-wallet mutation permit owner does not match owner serialization evidence"
                .to_string(),
        ));
    }
    if permit.owner != owner {
        return Err(RelayerError::mutation_blocked(format!(
            "deposit-wallet mutation permit owner {} does not match request owner {}",
            redacted_address(permit.owner),
            redacted_address(owner)
        )));
    }
    Ok(())
}

pub(super) fn validate_permit_scope(
    permit: &DepositWalletMutationPermit,
    expected_scope: DepositWalletMutationScope,
) -> Result<()> {
    if permit.owner_serialization_evidence.scope != expected_scope {
        return Err(RelayerError::mutation_blocked(
            "deposit-wallet mutation permit scope does not match client submit scope".to_string(),
        ));
    }
    Ok(())
}

pub(super) fn validate_permit_fresh(
    permit: &DepositWalletMutationPermit,
    now_unix_seconds: u64,
) -> Result<()> {
    if permit.reason.trim().is_empty() {
        return Err(RelayerError::mutation_blocked(
            "explicit deposit-wallet mutation permit reason required".to_string(),
        ));
    }
    let acquired_at = permit.owner_serialization_evidence.acquired_at_unix_seconds;
    if acquired_at > now_unix_seconds {
        return Err(RelayerError::mutation_blocked(
            "owner serialization evidence acquisition is in the future".to_string(),
        ));
    }
    if now_unix_seconds > acquired_at.saturating_add(MAX_OWNER_SERIALIZATION_LEASE_SECONDS) {
        return Err(RelayerError::mutation_blocked(
            "owner serialization evidence acquisition is stale".to_string(),
        ));
    }
    if permit.owner_serialization_evidence.expires_at_unix_seconds <= now_unix_seconds {
        return Err(RelayerError::mutation_blocked(
            "deposit-wallet mutation permit owner serialization evidence is expired".to_string(),
        ));
    }
    Ok(())
}

pub(super) fn validate_reconciliation_evidence(
    evidence: &DepositWalletSubmitReconciliationEvidence,
    permit: &DepositWalletMutationPermit,
    expected_scope: DepositWalletMutationScope,
    block_created_at_unix_seconds: u64,
    now_unix_seconds: u64,
) -> Result<()> {
    if evidence.scope != expected_scope {
        return Err(RelayerError::mutation_blocked(
            "submit reconciliation evidence scope does not match client reconciliation scope"
                .to_string(),
        ));
    }
    if permit.owner_serialization_evidence.scope != expected_scope {
        return Err(RelayerError::mutation_blocked(
            "manual reconciliation permit scope does not match client reconciliation scope"
                .to_string(),
        ));
    }
    if permit.owner_serialization_evidence.issuer != evidence.issuer {
        return Err(RelayerError::mutation_blocked(
            "submit reconciliation evidence issuer does not match mutation permit issuer"
                .to_string(),
        ));
    }
    if evidence.observation.checked_at_unix_seconds < block_created_at_unix_seconds {
        return Err(RelayerError::mutation_blocked(
            "submit reconciliation evidence predates the ambiguous owner block".to_string(),
        ));
    }
    if evidence.observation.checked_at_unix_seconds
        > now_unix_seconds.saturating_add(MAX_EVIDENCE_CLOCK_SKEW_SECONDS)
    {
        return Err(RelayerError::mutation_blocked(
            "submit reconciliation evidence check time is in the future".to_string(),
        ));
    }
    Ok(())
}

pub(super) fn validate_idless_reconciliation_evidence(
    evidence: &DepositWalletIdlessSubmitReconciliationEvidence,
    permit: &DepositWalletMutationPermit,
    expected_scope: DepositWalletMutationScope,
    block_created_at_unix_seconds: u64,
    now_unix_seconds: u64,
) -> Result<()> {
    if evidence.scope != expected_scope {
        return Err(RelayerError::mutation_blocked(
            "id-less submit reconciliation evidence scope does not match client reconciliation scope"
                .to_string(),
        ));
    }
    if permit.owner_serialization_evidence.scope != expected_scope {
        return Err(RelayerError::mutation_blocked(
            "manual reconciliation permit scope does not match client reconciliation scope"
                .to_string(),
        ));
    }
    if permit.owner_serialization_evidence.issuer != evidence.issuer {
        return Err(RelayerError::mutation_blocked(
            "id-less submit reconciliation evidence issuer does not match mutation permit issuer"
                .to_string(),
        ));
    }
    if evidence.checked_at_unix_seconds < block_created_at_unix_seconds {
        return Err(RelayerError::mutation_blocked(
            "id-less submit reconciliation evidence predates the ambiguous owner block".to_string(),
        ));
    }
    if evidence.checked_at_unix_seconds
        > now_unix_seconds.saturating_add(MAX_EVIDENCE_CLOCK_SKEW_SECONDS)
    {
        return Err(RelayerError::mutation_blocked(
            "id-less submit reconciliation evidence check time is in the future".to_string(),
        ));
    }
    Ok(())
}
