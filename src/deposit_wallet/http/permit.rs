use super::*;
use super::redaction::{display_payload_hash, payload_hash_summary, redacted_address};

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
    /// Creates an explicit owner-scoped live mutation permit.
    ///
    /// The evidence must come from a caller-side owner lock, nonce lease, or
    /// actor queue that prevents concurrent WALLET-CREATE/WALLET submits for
    /// the same owner. The crate validates the evidence shape and expiry before
    /// request construction; the caller remains responsible for enforcing the
    /// referenced guard in its runtime.
    pub fn from_owner_serialization_evidence(
        reason: impl Into<String>,
        owner_serialization_evidence: DepositWalletOwnerSerializationEvidence,
    ) -> Result<Self> {
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
    issuer: String,
    lease_id_hash: String,
    acquired_at_unix_seconds: u64,
    expires_at_unix_seconds: u64,
}

#[derive(Clone, PartialEq, Eq)]
pub struct DepositWalletSubmitReconciliationEvidence {
    owner: Address,
    payload_hash: String,
    reason: String,
    checked_at_unix_seconds: u64,
}

impl DepositWalletSubmitReconciliationEvidence {
    /// Records audited evidence that an ambiguous submit payload was manually
    /// reconciled outside this client before clearing the owner block.
    pub fn new(
        owner: Address,
        payload_hash: impl Into<String>,
        reason: impl Into<String>,
        checked_at_unix_seconds: u64,
    ) -> Result<Self> {
        let payload_hash = payload_hash.into();
        if payload_hash.trim().is_empty()
            || payload_hash.chars().any(char::is_whitespace)
            || payload_hash.len() > MAX_ERROR_TOKEN_LEN
        {
            return Err(RelayerError::mutation_blocked(
                "submit reconciliation evidence payload hash is invalid".to_string(),
            ));
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
            owner,
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

impl fmt::Debug for DepositWalletSubmitReconciliationEvidence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DepositWalletSubmitReconciliationEvidence")
            .field("owner", &redacted_address(self.owner))
            .field("payload_hash", &display_payload_hash(&self.payload_hash))
            .field("reason", &"<redacted>")
            .field("checked_at_unix_seconds", &self.checked_at_unix_seconds)
            .finish()
    }
}

impl DepositWalletOwnerSerializationEvidence {
    /// Records caller-side proof that same-owner submit work is serialized.
    ///
    /// `lease_id` is hashed before storage so debug output and errors never
    /// expose raw lock keys, queue ids, or database lease identifiers.
    pub fn new(
        owner: Address,
        issuer: impl Into<String>,
        lease_id: impl AsRef<[u8]>,
        acquired_at_unix_seconds: u64,
        expires_at_unix_seconds: u64,
    ) -> Result<Self> {
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
        Ok(Self {
            owner,
            issuer,
            lease_id_hash: payload_hash_summary(lease_id),
            acquired_at_unix_seconds,
            expires_at_unix_seconds,
        })
    }

    pub fn owner(&self) -> Address {
        self.owner
    }

    pub fn expires_at_unix_seconds(&self) -> u64 {
        self.expires_at_unix_seconds
    }
}

impl fmt::Debug for DepositWalletOwnerSerializationEvidence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DepositWalletOwnerSerializationEvidence")
            .field("owner", &redacted_address(self.owner))
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

pub(super) fn validate_permit_fresh(permit: &DepositWalletMutationPermit, now_unix_seconds: u64) -> Result<()> {
    if permit.reason.trim().is_empty() {
        return Err(RelayerError::mutation_blocked(
            "explicit deposit-wallet mutation permit reason required".to_string(),
        ));
    }
    if permit.owner_serialization_evidence.expires_at_unix_seconds <= now_unix_seconds {
        return Err(RelayerError::mutation_blocked(
            "deposit-wallet mutation permit owner serialization evidence is expired".to_string(),
        ));
    }
    Ok(())
}
