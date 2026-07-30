use ethers::types::Address;
use reqwest::Method;
use serde::Serialize;
use serde_json::Value;

use super::response::validate_transaction_id;
use super::{
    DepositWalletRelayerClient, RelayerTransactionState, MAX_TRANSACTION_RESPONSE_ITEMS,
    MAX_TRANSACTION_SUCCESS_BODY_BYTES, TRANSACTIONS_PATH,
};
use crate::deposit_wallet::{WALLET_CREATE_TRANSACTION_TYPE, WALLET_TRANSACTION_TYPE};
use crate::error::{RelayerError, Result};

const AMBIGUOUS_REPORT_REDACTION: &str =
    "auth material and raw bodies are intentionally omitted; candidates are read-only";
const MAX_CREATED_AT_BYTES: usize = 64;

/// One validated read-only candidate from the relayer's recent transaction list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AmbiguousCandidate {
    transaction_id: String,
    state_label: String,
    tx_type: String,
    created_at: Option<String>,
}

impl AmbiguousCandidate {
    pub fn transaction_id(&self) -> &str {
        &self.transaction_id
    }

    pub fn state_label(&self) -> &str {
        &self.state_label
    }

    pub fn tx_type(&self) -> &str {
        &self.tx_type
    }

    pub fn created_at(&self) -> Option<&str> {
        self.created_at.as_deref()
    }
}

/// Redacted, reviewable recent-transaction evidence for one unresolved intent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AmbiguousCandidateReport {
    owner: String,
    intent_status: String,
    intent_payload_keccak256: Option<String>,
    intent_epoch: u64,
    intent_created_at_unix: u64,
    candidates: Vec<AmbiguousCandidate>,
    skipped_items: usize,
    redaction: String,
}

impl AmbiguousCandidateReport {
    pub(super) fn new(
        owner: String,
        intent_status: String,
        intent_payload_keccak256: Option<String>,
        intent_epoch: u64,
        intent_created_at_unix: u64,
        candidates: Vec<AmbiguousCandidate>,
        skipped_items: usize,
    ) -> Self {
        Self {
            owner,
            intent_status,
            intent_payload_keccak256,
            intent_epoch,
            intent_created_at_unix,
            candidates,
            skipped_items,
            redaction: AMBIGUOUS_REPORT_REDACTION.to_string(),
        }
    }

    pub fn owner(&self) -> &str {
        &self.owner
    }

    pub fn intent_status(&self) -> &str {
        &self.intent_status
    }

    pub fn intent_payload_keccak256(&self) -> Option<&str> {
        self.intent_payload_keccak256.as_deref()
    }

    pub fn intent_epoch(&self) -> u64 {
        self.intent_epoch
    }

    pub fn intent_created_at_unix(&self) -> u64 {
        self.intent_created_at_unix
    }

    pub fn candidates(&self) -> &[AmbiguousCandidate] {
        &self.candidates
    }

    pub fn skipped_items(&self) -> usize {
        self.skipped_items
    }

    pub fn redaction(&self) -> &str {
        &self.redaction
    }
}

impl DepositWalletRelayerClient {
    pub(super) async fn fetch_recent_wallet_transactions(
        &self,
        owner: Address,
    ) -> Result<(Vec<AmbiguousCandidate>, usize)> {
        let url = self.base_url.endpoint(TRANSACTIONS_PATH);
        let response = self
            .send_with_success_limit(
                Method::GET,
                url,
                None,
                MAX_TRANSACTION_SUCCESS_BODY_BYTES,
            )
            .await?;
        parse_recent_wallet_transactions(&response, owner)
    }
}

fn parse_recent_wallet_transactions(
    response: &[u8],
    owner: Address,
) -> Result<(Vec<AmbiguousCandidate>, usize)> {
    let value: Value = serde_json::from_slice(response).map_err(|_| {
        RelayerError::Other("could not parse transactions response".to_string())
    })?;
    let Value::Array(items) = value else {
        return Err(RelayerError::Other(
            "could not parse transactions response".to_string(),
        ));
    };
    if items.len() > MAX_TRANSACTION_RESPONSE_ITEMS {
        return Err(RelayerError::Other(
            "transactions response item limit exceeded".to_string(),
        ));
    }

    let mut candidates = Vec::new();
    let mut skipped_items = 0usize;
    for item in &items {
        if let Some(candidate) = parse_candidate(item, owner) {
            candidates.push(candidate);
        } else {
            skipped_items += 1;
        }
    }
    Ok((candidates, skipped_items))
}

fn parse_candidate(value: &Value, owner: Address) -> Option<AmbiguousCandidate> {
    let object = value.as_object()?;
    let from = parse_wire_address(object.get("from")?.as_str()?)?;
    if from != owner {
        return None;
    }

    let tx_type = object.get("type")?.as_str()?;
    if tx_type != WALLET_TRANSACTION_TYPE && tx_type != WALLET_CREATE_TRANSACTION_TYPE {
        return None;
    }
    let transaction_id = validate_transaction_id(object.get("transactionID")?.as_str()?).ok()?;
    let state = RelayerTransactionState::parse(object.get("state")?.as_str()?);
    if matches!(state, RelayerTransactionState::Unknown(_)) {
        return None;
    }
    let created_at = object
        .get("createdAt")
        .and_then(Value::as_str)
        .filter(|created_at| is_safe_created_at(created_at))
        .map(str::to_string);

    Some(AmbiguousCandidate {
        transaction_id,
        state_label: state.label(),
        tx_type: tx_type.to_string(),
        created_at,
    })
}

fn parse_wire_address(value: &str) -> Option<Address> {
    if value.len() != 42
        || !value.starts_with("0x")
        || !value.as_bytes()[2..]
            .iter()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return None;
    }
    value.parse().ok()
}

fn is_safe_created_at(value: &str) -> bool {
    value.len() <= MAX_CREATED_AT_BYTES
        && value
            .bytes()
            .all(|byte| (0x20..=0x7e).contains(&byte))
}
