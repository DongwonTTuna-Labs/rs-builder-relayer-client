use super::redaction::{
    redacted_address, sanitized_external_token, unknown_state_error_summary,
};
use super::*;
use super::redaction::external_token_hash;
use crate::deposit_wallet::{
    derive_deposit_wallet_address, DepositWalletContractConfig, WALLET_CREATE_TRANSACTION_TYPE,
    WALLET_TRANSACTION_TYPE,
};
use serde_json::value::RawValue;
use serde_json::Value;

const DEPOSIT_WALLET_RECONCILIATION_REQUIRED_PREFIX: &str =
    "Deposit-wallet reconciliation required: ";

#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct DepositWalletTransactionReceipt {
    pub transaction_id: String,
    pub state: RelayerTransactionState,
    pub transaction_hash: Option<String>,
    pub owner: Option<Address>,
    pub deposit_wallet: Option<Address>,
}

impl fmt::Debug for DepositWalletTransactionReceipt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DepositWalletTransactionReceipt")
            .field("transaction_id", &sanitized_external_token(&self.transaction_id))
            .field("state", &ReceiptStateDebug(&self.state))
            .field(
                "transaction_hash",
                &self.transaction_hash.as_deref().map(sanitized_external_token),
            )
            .field("owner", &self.owner.map(redacted_address))
            .field(
                "deposit_wallet",
                &self.deposit_wallet.map(redacted_address),
            )
            .finish()
    }
}

struct ReceiptStateDebug<'a>(&'a RelayerTransactionState);

impl fmt::Debug for ReceiptStateDebug<'_> {
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
pub(super) struct ParsedTransactionReceipt {
    pub(super) receipt: DepositWalletTransactionReceipt,
    pub(super) owner: Option<Address>,
    pub(super) transaction_type: &'static str,
}

struct TransactionWireEvidence {
    deposit_wallet: Address,
    transaction_type: &'static str,
}

#[derive(Debug)]
pub(super) struct TransactionParseError {
    pub(super) error: RelayerError,
}

impl TransactionParseError {
    pub(super) fn new(error: RelayerError) -> Self {
        Self { error }
    }

    pub(super) fn retryable_absence(error: RelayerError) -> Self {
        Self { error }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RelayerTransactionResponseWithOwner {
    #[serde(flatten)]
    response: RelayerSubmitResponse,
    #[serde(default, rename = "type")]
    tx_type: Option<String>,
    #[serde(default, rename = "from", deserialize_with = "deserialize_optional_address")]
    from_address: Option<Address>,
    #[serde(default, deserialize_with = "deserialize_optional_address")]
    to: Option<Address>,
    #[serde(default, deserialize_with = "deserialize_optional_address")]
    owner: Option<Address>,
    #[serde(
        default,
        rename = "proxyAddress",
        deserialize_with = "deserialize_optional_address"
    )]
    proxy_address: Option<Address>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubmitTransactionIdOnly {
    #[serde(rename = "transactionID")]
    transaction_id: String,
}

#[derive(Deserialize)]
struct TransactionIdProbe {
    #[serde(default, rename = "transactionID")]
    transaction_id: Option<Value>,
}

pub(super) fn parse_submit_response(bytes: &[u8]) -> Result<DepositWalletTransactionReceipt> {
    let response = parse_submit_response_body(bytes)?;
    receipt_from_submit_response(response, None, None)
}

pub(super) fn extract_submit_transaction_id(bytes: &[u8]) -> Option<String> {
    let response = match bytes.iter().copied().find(|byte| !byte.is_ascii_whitespace())? {
        b'{' => serde_json::from_slice::<SubmitTransactionIdOnly>(bytes).ok()?,
        b'[' => return None,
        _ => return None,
    };
    validate_transaction_id(&response.transaction_id).ok()
}

fn parse_submit_response_body(bytes: &[u8]) -> Result<RelayerSubmitResponse> {
    match bytes.iter().copied().find(|byte| !byte.is_ascii_whitespace()) {
        Some(b'{') => serde_json::from_slice::<RelayerSubmitResponse>(bytes).or_else(|_| {
            let value = serde_json::from_slice::<Value>(bytes).map_err(|_| {
                RelayerError::Other("could not parse submit response object".to_string())
            })?;
            let Some(object) = value.as_object() else {
                return Err(RelayerError::Other(
                    "could not parse submit response object".to_string(),
                ));
            };
            if object.len() != 1 || !object.contains_key("transactionID") {
                return Err(RelayerError::Other(
                    "could not parse submit response object".to_string(),
                ));
            }
            let id_only = serde_json::from_value::<SubmitTransactionIdOnly>(value).map_err(|_| {
                RelayerError::Other("could not parse submit response object".to_string())
            })?;
            Ok(RelayerSubmitResponse {
                transaction_id: id_only.transaction_id,
                state: RelayerTransactionState::New,
                transaction_hash: None,
            })
        }),
        Some(b'[') => Err(RelayerError::reconciliation_required(
            "submit response arrays are not an official relayer wire format; manual reconciliation required"
                .to_string(),
        )),
        _ => Err(RelayerError::Other(
            "could not parse submit response: expected JSON object".to_string(),
        )),
    }
}

pub(super) fn parse_transaction_response(
    expected_transaction_id: &str,
    config: DepositWalletContractConfig,
    bytes: &[u8],
) -> std::result::Result<ParsedTransactionReceipt, TransactionParseError> {
    match bytes.iter().copied().find(|byte| !byte.is_ascii_whitespace()) {
        Some(b'{') => {
            let value = serde_json::from_slice::<Value>(bytes)
                .map_err(|_| {
                    TransactionParseError::new(RelayerError::Other(
                        "could not parse transaction response object".to_string(),
                    ))
                })?;
            validate_transaction_address_evidence_shape(&value)?;
            let response = serde_json::from_value::<RelayerTransactionResponseWithOwner>(value)
                .map_err(|_| {
                    TransactionParseError::new(RelayerError::Other(
                        "could not parse transaction response object".to_string(),
                    ))
                })?;
            return parse_verified_transaction_response(
                expected_transaction_id,
                config,
                response,
            );
        }
        Some(b'[') => {}
        _ => {
            return Err(TransactionParseError::new(RelayerError::Other(
                "could not parse transaction response: expected JSON object or array".to_string(),
            )))
        }
    }

    let response = select_transaction_response_from_array(expected_transaction_id, bytes)?;
    parse_verified_transaction_response(expected_transaction_id, config, response)
}

fn parse_verified_transaction_response(
    expected_transaction_id: &str,
    config: DepositWalletContractConfig,
    response: RelayerTransactionResponseWithOwner,
) -> std::result::Result<ParsedTransactionReceipt, TransactionParseError> {
    let owner = response.owner;
    let response_transaction_id =
        validate_transaction_id(&response.response.transaction_id).map_err(|_| {
            TransactionParseError::new(RelayerError::Other(
                "relayer response transactionID was invalid".to_string(),
            ))
        })?;
    if response_transaction_id != expected_transaction_id {
        return Err(TransactionParseError::new(
            RelayerError::reconciliation_required(format!(
                "transaction response id hash {} did not match requested id hash {}",
                external_token_hash(&response_transaction_id),
                external_token_hash(expected_transaction_id)
            )),
        ));
    }
    let wire_evidence = validate_transaction_wire_evidence(&response, config, owner)?;
    let receipt =
        receipt_from_submit_response(response.response, owner, Some(wire_evidence.deposit_wallet))
            .map_err(TransactionParseError::new)?;
    Ok(ParsedTransactionReceipt {
        receipt,
        owner,
        transaction_type: wire_evidence.transaction_type,
    })
}

fn validate_transaction_wire_evidence(
    response: &RelayerTransactionResponseWithOwner,
    config: DepositWalletContractConfig,
    owner: Option<Address>,
) -> std::result::Result<TransactionWireEvidence, TransactionParseError> {
    let tx_type = response.tx_type.as_deref().ok_or_else(|| {
        TransactionParseError::new(RelayerError::reconciliation_required(
            "transaction response did not include deposit-wallet transaction type; manual reconciliation required"
                .to_string(),
        ))
    })?;
    let transaction_type = match tx_type {
        WALLET_TRANSACTION_TYPE => WALLET_TRANSACTION_TYPE,
        WALLET_CREATE_TRANSACTION_TYPE => WALLET_CREATE_TRANSACTION_TYPE,
        _ => {
            return Err(TransactionParseError::new(
                RelayerError::reconciliation_required(
                    "transaction response type was not WALLET or WALLET-CREATE; manual reconciliation required"
                        .to_string(),
                ),
            ))
        }
    };

    let owner = owner.ok_or_else(|| {
        TransactionParseError::new(RelayerError::reconciliation_required(
            "deposit wallet transaction response did not include owner evidence; manual reconciliation required"
                .to_string(),
        ))
    })?;
    let from_address = response.from_address.ok_or_else(|| {
        TransactionParseError::new(RelayerError::reconciliation_required(
            "transaction response did not include from address; manual reconciliation required"
                .to_string(),
        ))
    })?;
    if from_address != owner {
        return Err(TransactionParseError::new(
            RelayerError::reconciliation_required(format!(
                "transaction response from address {} did not match owner evidence {}; manual reconciliation required",
                redacted_address(from_address),
                redacted_address(owner)
            )),
        ));
    }

    let to = response.to.ok_or_else(|| {
        TransactionParseError::new(RelayerError::reconciliation_required(
            "transaction response did not include to address; manual reconciliation required"
                .to_string(),
        ))
    })?;
    if to != config.factory {
        return Err(TransactionParseError::new(
            RelayerError::reconciliation_required(format!(
                "transaction response to address {} did not match configured factory {}; manual reconciliation required",
                redacted_address(to),
                redacted_address(config.factory)
            )),
        ));
    }
    let proxy_address = response.proxy_address.ok_or_else(|| {
        TransactionParseError::new(RelayerError::reconciliation_required(
            "transaction response did not include proxyAddress deposit wallet evidence; manual reconciliation required"
                .to_string(),
        ))
    })?;
    let expected_proxy_address = derive_deposit_wallet_address(owner, config)
        .map_err(|error| TransactionParseError::new(RelayerError::Other(error.to_string())))?;
    if proxy_address != expected_proxy_address {
        return Err(TransactionParseError::new(
            RelayerError::reconciliation_required(format!(
                "transaction response proxyAddress {} did not match derived deposit wallet {}; manual reconciliation required",
                redacted_address(proxy_address),
                redacted_address(expected_proxy_address)
            )),
        ));
    }

    Ok(TransactionWireEvidence {
        deposit_wallet: proxy_address,
        transaction_type,
    })
}

fn select_transaction_response_from_array(
    expected_transaction_id: &str,
    bytes: &[u8],
) -> std::result::Result<RelayerTransactionResponseWithOwner, TransactionParseError> {
    struct SelectTransactionVisitor<'a> {
        expected_transaction_id: &'a str,
    }

    impl<'de> Visitor<'de> for SelectTransactionVisitor<'_> {
        type Value = RelayerTransactionResponseWithOwner;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a transaction response array")
        }

        fn visit_seq<A>(self, mut seq: A) -> std::result::Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let mut count = 0usize;
            let mut matching_response = None;
            while let Some(response_value) = seq.next_element::<&RawValue>()? {
                count += 1;
                if count > MAX_TRANSACTION_RESPONSE_ITEMS {
                    return Err(de::Error::custom(TRANSACTION_RESPONSE_ITEM_LIMIT_ERROR));
                }
                let Ok(probe) =
                    serde_json::from_str::<TransactionIdProbe>(response_value.get())
                else {
                    continue;
                };
                let Some(response_transaction_id) =
                    probe.transaction_id.as_ref().and_then(Value::as_str)
                else {
                    continue;
                };
                let Ok(response_transaction_id) = validate_transaction_id(response_transaction_id)
                else {
                    continue;
                };
                if response_transaction_id != self.expected_transaction_id {
                    continue;
                }
                if matching_response.is_some() {
                    return Err(de::Error::custom(
                        TRANSACTION_RESPONSE_DUPLICATE_ID_ERROR,
                    ));
                }
                let response_value =
                    serde_json::from_str::<Value>(response_value.get()).map_err(de::Error::custom)?;
                validate_transaction_address_evidence_shape(&response_value)
                    .map_err(|error| de::Error::custom(error.error.to_string()))?;
                let response =
                    serde_json::from_value::<RelayerTransactionResponseWithOwner>(response_value)
                        .map_err(de::Error::custom)?;
                matching_response = Some(response);
            }
            if let Some(response) = matching_response {
                Ok(response)
            } else {
                Err(de::Error::custom(TRANSACTION_RESPONSE_MISSING_ID_ERROR))
            }
        }
    }

    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let response = deserializer
        .deserialize_seq(SelectTransactionVisitor {
            expected_transaction_id,
        })
        .map_err(|error| {
            let message = error.to_string();
            if message.contains(TRANSACTION_RESPONSE_ITEM_LIMIT_ERROR) {
                TransactionParseError::new(RelayerError::reconciliation_required(format!(
                    "transaction response included more than {MAX_TRANSACTION_RESPONSE_ITEMS} items"
                )))
            } else if message.contains(TRANSACTION_RESPONSE_MISSING_ID_ERROR) {
                TransactionParseError::retryable_absence(
                    RelayerError::transaction_absent(format!(
                        "transaction response did not include requested transaction id hash {}",
                        external_token_hash(expected_transaction_id)
                    )),
                )
            } else if message.contains(TRANSACTION_RESPONSE_DUPLICATE_ID_ERROR) {
                TransactionParseError::new(RelayerError::reconciliation_required(format!(
                    "transaction response included duplicate requested transaction id hash {}; manual reconciliation required",
                    external_token_hash(expected_transaction_id)
                )))
            } else if let Some(reason) =
                reconciliation_reason_from_deserializer_error(&message)
            {
                TransactionParseError::new(RelayerError::reconciliation_required(reason))
            } else {
                TransactionParseError::new(RelayerError::Other(
                    "could not parse transaction response array".to_string(),
                ))
            }
        })?;
    deserializer.end().map_err(|_| {
        TransactionParseError::new(RelayerError::Other(
            "could not parse transaction response array".to_string(),
        ))
    })?;
    Ok(response)
}

fn reconciliation_reason_from_deserializer_error(message: &str) -> Option<String> {
    let start = message.find(DEPOSIT_WALLET_RECONCILIATION_REQUIRED_PREFIX)?
        + DEPOSIT_WALLET_RECONCILIATION_REQUIRED_PREFIX.len();
    Some(message[start..].to_string())
}

fn validate_transaction_address_evidence_shape(
    value: &Value,
) -> std::result::Result<(), TransactionParseError> {
    let Some(object) = value.as_object() else {
        return Err(TransactionParseError::new(RelayerError::Other(
            "could not parse transaction response object".to_string(),
        )));
    };
    for field in ["from", "to", "owner", "proxyAddress"] {
        validate_optional_address_evidence_value(object, field)?;
    }
    Ok(())
}

fn validate_optional_address_evidence_value(
    object: &serde_json::Map<String, Value>,
    field: &str,
) -> std::result::Result<(), TransactionParseError> {
    let Some(value) = object.get(field) else {
        return Ok(());
    };

    match value {
        Value::Null => Ok(()),
        Value::String(raw) if is_official_address_wire_format(raw) => Ok(()),
        Value::String(_) => Err(TransactionParseError::new(
            RelayerError::reconciliation_required(format!(
                "transaction response {field} address evidence was malformed; manual reconciliation required"
            )),
        )),
        _ => Err(TransactionParseError::new(
            RelayerError::reconciliation_required(format!(
                "transaction response {field} address evidence was not a string; manual reconciliation required"
            )),
        )),
    }
}

fn receipt_from_submit_response(
    response: RelayerSubmitResponse,
    owner: Option<Address>,
    deposit_wallet: Option<Address>,
) -> Result<DepositWalletTransactionReceipt> {
    if response.transaction_id.trim().is_empty() {
        return Err(RelayerError::Other(
            "relayer response transactionID must not be empty".to_string(),
        ));
    }
    let transaction_id = validate_transaction_id(&response.transaction_id).map_err(|_| {
        RelayerError::Other("relayer response transactionID was invalid".to_string())
    })?;
    let transaction_hash = match response.transaction_hash.as_deref().map(str::trim) {
        Some("") | None => None,
        Some(hash) => Some(validate_transaction_hash(hash)?),
    };

    Ok(DepositWalletTransactionReceipt {
        transaction_id,
        state: response.state,
        transaction_hash,
        owner,
        deposit_wallet,
    })
}

pub(super) fn validate_transaction_id(transaction_id: &str) -> Result<String> {
    if transaction_id.is_empty()
        || transaction_id.len() > MAX_TRANSACTION_ID_LEN
        || transaction_id.trim() != transaction_id
        || transaction_id.bytes().any(|byte| byte.is_ascii_control())
    {
        return Err(RelayerError::Other(
            "transaction id must be 1-128 bytes without leading/trailing whitespace or control characters"
                .to_string(),
        ));
    }

    Ok(transaction_id.to_string())
}

pub(super) fn validate_transaction_hash(transaction_hash: &str) -> Result<String> {
    if transaction_hash.len() == 66 {
        if let Some(hex) = transaction_hash
            .strip_prefix("0x")
            .or_else(|| transaction_hash.strip_prefix("0X"))
        {
            if hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Ok(format!("0x{}", hex.to_ascii_lowercase()));
            }
        }
    }

    Err(RelayerError::reconciliation_required(
        "relayer response transactionHash was invalid".to_string(),
    ))
}

fn is_official_address_wire_format(raw: &str) -> bool {
    raw.len() == 42
        && raw.starts_with("0x")
        && raw[2..].bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn deserialize_optional_address<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<Address>, D::Error>
where
    D: Deserializer<'de>,
{
    let Some(raw) = Option::<String>::deserialize(deserializer)? else {
        return Ok(None);
    };
    if !is_official_address_wire_format(&raw) {
        return Err(serde::de::Error::custom(
            "address evidence must match ^0x[a-fA-F0-9]{40}$",
        ));
    }
    raw.parse().map(Some).map_err(serde::de::Error::custom)
}
