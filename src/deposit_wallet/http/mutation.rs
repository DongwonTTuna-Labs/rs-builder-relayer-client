use std::fmt;

use ethers::types::Address;
use ethers::utils::{keccak256, to_checksum};
use serde::{Deserialize, Serialize};

use super::redaction::{
    redacted_address, sanitized_external_token, unknown_state_error_summary,
};
use super::SUBMIT_PATH;
use crate::deposit_wallet::transaction::RelayerTransactionState;
use crate::deposit_wallet::types::{
    DepositWalletBatchRequest, WALLET_CREATE_TRANSACTION_TYPE, WALLET_TRANSACTION_TYPE,
};
use crate::error::{RelayerError, Result};

const MAX_MUTATION_REFERENCE_BYTES: usize = 256;
const DRY_RUN_REDACTION_MARKER: &str =
    "signature, auth headers, and full submit body are intentionally omitted";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelayerMutationMode {
    DryRun,
    Live,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RelayerMutationOperation {
    WalletCreate,
    WalletBatch,
}

/// An explicit, scoped capability for one deposit-wallet relayer mutation.
#[derive(Clone, PartialEq, Eq)]
pub struct RelayerMutationPermit {
    mode: RelayerMutationMode,
    operation: RelayerMutationOperation,
    owner: Address,
    chain_id: u64,
    expires_at_unix: u64,
    evidence_ref: String,
    operator_approval_ref: String,
}

impl RelayerMutationPermit {
    pub fn try_new(
        mode: RelayerMutationMode,
        operation: RelayerMutationOperation,
        owner: Address,
        chain_id: u64,
        expires_at_unix: u64,
        evidence_ref: impl Into<String>,
        operator_approval_ref: impl Into<String>,
    ) -> Result<Self> {
        if expires_at_unix == 0 {
            return Err(RelayerError::mutation_blocked(
                "mutation permit expiry must be non-zero",
            ));
        }

        let evidence_ref = validate_reference("evidence reference", evidence_ref.into())?;
        let operator_approval_ref = validate_reference(
            "operator approval reference",
            operator_approval_ref.into(),
        )?;

        Ok(Self {
            mode,
            operation,
            owner,
            chain_id,
            expires_at_unix,
            evidence_ref,
            operator_approval_ref,
        })
    }

    pub fn mode(&self) -> RelayerMutationMode {
        self.mode
    }

    pub fn operation(&self) -> RelayerMutationOperation {
        self.operation
    }

    pub fn owner(&self) -> Address {
        self.owner
    }

    pub fn chain_id(&self) -> u64 {
        self.chain_id
    }

    pub fn expires_at_unix(&self) -> u64 {
        self.expires_at_unix
    }
}

impl fmt::Debug for RelayerMutationPermit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RelayerMutationPermit")
            .field("mode", &self.mode)
            .field("operation", &self.operation)
            .field("owner", &redacted_address(self.owner))
            .field("chain_id", &self.chain_id)
            .field("expires_at_unix", &self.expires_at_unix)
            .field("evidence_ref_len", &self.evidence_ref.len())
            .field(
                "operator_approval_ref_len",
                &self.operator_approval_ref.len(),
            )
            .finish()
    }
}

fn validate_reference(label: &str, value: String) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(RelayerError::mutation_blocked(format!(
            "{label} must not be empty"
        )));
    }
    if trimmed.len() > MAX_MUTATION_REFERENCE_BYTES {
        return Err(RelayerError::mutation_blocked(format!(
            "{label} must not exceed {MAX_MUTATION_REFERENCE_BYTES} bytes"
        )));
    }
    if trimmed.chars().any(char::is_control) {
        return Err(RelayerError::mutation_blocked(format!(
            "{label} must not contain control characters"
        )));
    }
    Ok(trimmed.to_string())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DryRunCallSummary {
    target: String,
    value: String,
    selector: Option<String>,
    data_len: usize,
}

impl DryRunCallSummary {
    pub fn target(&self) -> &str {
        &self.target
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    pub fn selector(&self) -> Option<&str> {
        self.selector.as_deref()
    }

    pub fn data_len(&self) -> usize {
        self.data_len
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DepositWalletDryRunEvidence {
    operation: String,
    endpoint_path: String,
    chain_id: u64,
    owner: String,
    deposit_wallet: String,
    to: String,
    payload_keccak256: String,
    nonce: Option<String>,
    deadline: Option<String>,
    calls: Vec<DryRunCallSummary>,
    evidence_ref: String,
    operator_approval_ref: String,
    redaction: String,
}

impl DepositWalletDryRunEvidence {
    pub(super) fn for_wallet_create(
        permit: &RelayerMutationPermit,
        chain_id: u64,
        owner: Address,
        deposit_wallet: Address,
        to: Address,
        body: &[u8],
    ) -> Self {
        Self {
            operation: WALLET_CREATE_TRANSACTION_TYPE.to_string(),
            endpoint_path: SUBMIT_PATH.to_string(),
            chain_id,
            owner: redacted_address(owner),
            deposit_wallet: redacted_address(deposit_wallet),
            to: to_checksum(&to, None),
            payload_keccak256: payload_keccak256(body),
            nonce: None,
            deadline: None,
            calls: Vec::new(),
            evidence_ref: permit.evidence_ref.clone(),
            operator_approval_ref: permit.operator_approval_ref.clone(),
            redaction: DRY_RUN_REDACTION_MARKER.to_string(),
        }
    }

    pub(super) fn for_wallet_batch(
        permit: &RelayerMutationPermit,
        chain_id: u64,
        request: &DepositWalletBatchRequest,
        body: &[u8],
    ) -> Self {
        let calls = request
            .deposit_wallet_params
            .calls
            .iter()
            .map(|call| {
                let data = call.data.as_ref();
                DryRunCallSummary {
                    target: redacted_address(call.target),
                    value: call.value.to_string(),
                    // A four-byte calldata is entirely reproduced by its own
                    // selector, so the selector is withheld at that boundary.
                    // `get(..4)` would expose it, since it succeeds at len == 4.
                    selector: (data.len() > 4)
                        .then(|| format!("0x{}", hex::encode(&data[..4]))),
                    data_len: data.len(),
                }
            })
            .collect();

        Self {
            operation: WALLET_TRANSACTION_TYPE.to_string(),
            endpoint_path: SUBMIT_PATH.to_string(),
            chain_id,
            owner: redacted_address(request.from_address),
            deposit_wallet: redacted_address(request.deposit_wallet_params.deposit_wallet),
            to: to_checksum(&request.to, None),
            payload_keccak256: payload_keccak256(body),
            nonce: Some(request.nonce.to_string()),
            deadline: Some(request.deposit_wallet_params.deadline.to_string()),
            calls,
            evidence_ref: permit.evidence_ref.clone(),
            operator_approval_ref: permit.operator_approval_ref.clone(),
            redaction: DRY_RUN_REDACTION_MARKER.to_string(),
        }
    }

    pub fn operation(&self) -> &str {
        &self.operation
    }

    pub fn endpoint_path(&self) -> &str {
        &self.endpoint_path
    }

    pub fn chain_id(&self) -> u64 {
        self.chain_id
    }

    pub fn owner(&self) -> &str {
        &self.owner
    }

    pub fn deposit_wallet(&self) -> &str {
        &self.deposit_wallet
    }

    pub fn to(&self) -> &str {
        &self.to
    }

    pub fn payload_keccak256(&self) -> &str {
        &self.payload_keccak256
    }

    pub fn nonce(&self) -> Option<&str> {
        self.nonce.as_deref()
    }

    pub fn deadline(&self) -> Option<&str> {
        self.deadline.as_deref()
    }

    pub fn calls(&self) -> &[DryRunCallSummary] {
        &self.calls
    }

    pub fn evidence_ref(&self) -> &str {
        &self.evidence_ref
    }

    pub fn operator_approval_ref(&self) -> &str {
        &self.operator_approval_ref
    }

    pub fn redaction(&self) -> &str {
        &self.redaction
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct DepositWalletSubmitReceipt {
    transaction_id: String,
    state: RelayerTransactionState,
    payload_keccak256: String,
}

impl DepositWalletSubmitReceipt {
    pub(super) fn new(
        transaction_id: String,
        state: RelayerTransactionState,
        payload_keccak256: String,
    ) -> Self {
        Self {
            transaction_id,
            state,
            payload_keccak256,
        }
    }

    pub fn transaction_id(&self) -> &str {
        &self.transaction_id
    }

    pub fn state(&self) -> &RelayerTransactionState {
        &self.state
    }

    pub fn payload_keccak256(&self) -> &str {
        &self.payload_keccak256
    }
}

impl fmt::Debug for DepositWalletSubmitReceipt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DepositWalletSubmitReceipt")
            .field(
                "transaction_id",
                &sanitized_external_token(&self.transaction_id),
            )
            .field("state", &SubmitReceiptStateDebug(&self.state))
            .field("payload_keccak256", &self.payload_keccak256)
            .finish()
    }
}

struct SubmitReceiptStateDebug<'a>(&'a RelayerTransactionState);

impl fmt::Debug for SubmitReceiptStateDebug<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            RelayerTransactionState::New => f.write_str("New"),
            RelayerTransactionState::Executed => f.write_str("Executed"),
            RelayerTransactionState::Mined => f.write_str("Mined"),
            RelayerTransactionState::Confirmed => f.write_str("Confirmed"),
            RelayerTransactionState::Invalid => f.write_str("Invalid"),
            RelayerTransactionState::Failed => f.write_str("Failed"),
            RelayerTransactionState::Unknown(raw) => f
                .debug_tuple("Unknown")
                .field(&unknown_state_error_summary(raw))
                .finish(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RelayerSubmitOutcome {
    DryRun(Box<DepositWalletDryRunEvidence>),
    Submitted(DepositWalletSubmitReceipt),
}

pub(super) fn payload_keccak256(body: &[u8]) -> String {
    format!("0x{}", hex::encode(keccak256(body)))
}

#[cfg(test)]
mod selector_boundary_tests {
    use super::*;
    use crate::deposit_wallet::types::{DepositWalletCall, DepositWalletParams};
    use ethers::types::{Address, Bytes, U256};

    fn request_with_call_data(data: Vec<u8>) -> DepositWalletBatchRequest {
        DepositWalletBatchRequest {
            tx_type: "WALLET".to_string(),
            from_address: Address::from_low_u64_be(1),
            to: Address::from_low_u64_be(2),
            nonce: U256::from(7u64),
            signature: "0xsig".to_string(),
            deposit_wallet_params: DepositWalletParams {
                deposit_wallet: Address::from_low_u64_be(3),
                deadline: U256::from(99u64),
                calls: vec![DepositWalletCall {
                    target: Address::from_low_u64_be(4),
                    value: U256::zero(),
                    data: Bytes::from(data),
                }],
            },
        }
    }

    fn permit() -> RelayerMutationPermit {
        RelayerMutationPermit::try_new(
            RelayerMutationMode::DryRun,
            RelayerMutationOperation::WalletBatch,
            Address::from_low_u64_be(1),
            137,
            4_102_444_800,
            "evidence-ref",
            "operator-approval-ref",
        )
        .expect("permit is valid")
    }

    /// A four-byte calldata is entirely reproduced by its own selector, so the
    /// selector must be withheld at that boundary.
    #[test]
    fn four_byte_calldata_does_not_leak_through_selector() {
        let request = request_with_call_data(vec![0xde, 0xad, 0xbe, 0xef]);
        let evidence =
            DepositWalletDryRunEvidence::for_wallet_batch(&permit(), 137, &request, b"body");

        let summary = &evidence.calls()[0];
        assert_eq!(summary.selector(), None);
        assert_eq!(summary.data_len(), 4);

        let json = serde_json::to_string(&evidence).expect("evidence serializes");
        assert!(!json.contains("deadbeef"));
    }

    #[test]
    fn five_byte_calldata_still_exposes_its_selector() {
        let request = request_with_call_data(vec![0xde, 0xad, 0xbe, 0xef, 0x01]);
        let evidence =
            DepositWalletDryRunEvidence::for_wallet_batch(&permit(), 137, &request, b"body");

        let summary = &evidence.calls()[0];
        assert_eq!(summary.selector(), Some("0xdeadbeef"));
        assert_eq!(summary.data_len(), 5);

        let json = serde_json::to_string(&evidence).expect("evidence serializes");
        assert!(!json.contains("deadbeef01"));
    }
}
