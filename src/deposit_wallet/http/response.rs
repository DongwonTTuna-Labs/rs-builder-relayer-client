use super::redaction::{
    external_token_hash, redacted_address, sanitized_external_token, unknown_state_error_summary,
};
use super::*;
use crate::deposit_wallet::WALLET_TRANSACTION_TYPE;

#[derive(Clone, PartialEq, Eq)]
pub struct DepositWalletTransactionReceipt {
    pub transaction_id: String,
    pub state: RelayerTransactionState,
    pub transaction_hash: Option<String>,
    pub owner: Option<Address>,
}

impl fmt::Debug for DepositWalletTransactionReceipt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DepositWalletTransactionReceipt")
            .field("transaction_id", &sanitized_external_token(&self.transaction_id))
            .field("state", &ReceiptStateDebug(&self.state))
            .field("transaction_hash", &self.transaction_hash)
            .field("owner", &self.owner.map(redacted_address))
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
}

pub(super) fn parse_transaction_response(
    expected_transaction_id: &str,
    expected_factory: Address,
    bytes: &[u8],
) -> std::result::Result<ParsedTransactionReceipt, TransactionParseError> {
    match bytes.iter().copied().find(|byte| !byte.is_ascii_whitespace()) {
        Some(b'{') => {
            let response = serde_json::from_slice::<RelayerTransactionResponseWithOwner>(bytes)
                .map_err(|_| {
                    TransactionParseError::new(RelayerError::Other(
                        "could not parse transaction response object".to_string(),
                    ))
                })?;
            return parse_verified_transaction_response(
                expected_transaction_id,
                expected_factory,
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
    parse_verified_transaction_response(expected_transaction_id, expected_factory, response)
}

fn parse_verified_transaction_response(
    expected_transaction_id: &str,
    expected_factory: Address,
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
    validate_transaction_wire_evidence(&response, expected_factory, owner)?;
    let parsed = receipt_from_submit_response(response.response, owner)
        .map_err(TransactionParseError::new)?;
    Ok(parsed)
}

fn validate_transaction_wire_evidence(
    response: &RelayerTransactionResponseWithOwner,
    expected_factory: Address,
    owner: Option<Address>,
) -> std::result::Result<(), TransactionParseError> {
    let tx_type = response.tx_type.as_deref().ok_or_else(|| {
        TransactionParseError::new(RelayerError::reconciliation_required(
            "transaction response did not include deposit-wallet transaction type; manual reconciliation required"
                .to_string(),
        ))
    })?;
    if tx_type != WALLET_TRANSACTION_TYPE {
        return Err(TransactionParseError::new(
            RelayerError::reconciliation_required(
                "transaction response type was not WALLET; manual reconciliation required"
                    .to_string(),
            ),
        ));
    }

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
    if to != expected_factory {
        return Err(TransactionParseError::new(
            RelayerError::reconciliation_required(format!(
                "transaction response to address {} did not match expected relayer target {}; manual reconciliation required",
                redacted_address(to),
                redacted_address(expected_factory)
            )),
        ));
    }

    Ok(())
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
            while let Some(response) = seq.next_element::<RelayerTransactionResponseWithOwner>()? {
                count += 1;
                if count > MAX_TRANSACTION_RESPONSE_ITEMS {
                    return Err(de::Error::custom(TRANSACTION_RESPONSE_ITEM_LIMIT_ERROR));
                }
                let response_transaction_id = validate_transaction_id(
                    &response.response.transaction_id,
                )
                .map_err(|_| de::Error::custom(TRANSACTION_RESPONSE_INVALID_ID_ERROR))?;
                if response_transaction_id == self.expected_transaction_id {
                    if matching_response.is_some() {
                        return Err(de::Error::custom(
                            TRANSACTION_RESPONSE_DUPLICATE_ID_ERROR,
                        ));
                    }
                    matching_response = Some(response);
                }
            }
            matching_response.ok_or_else(|| de::Error::custom(TRANSACTION_RESPONSE_MISSING_ID_ERROR))
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
                    RelayerError::reconciliation_required(format!(
                        "transaction response did not include requested transaction id hash {}",
                        external_token_hash(expected_transaction_id)
                    )),
                )
            } else if message.contains(TRANSACTION_RESPONSE_DUPLICATE_ID_ERROR) {
                TransactionParseError::new(RelayerError::reconciliation_required(format!(
                    "transaction response included duplicate requested transaction id hash {}; manual reconciliation required",
                    external_token_hash(expected_transaction_id)
                )))
            } else if message.contains(TRANSACTION_RESPONSE_INVALID_ID_ERROR) {
                TransactionParseError::new(RelayerError::reconciliation_required(
                    "transaction response included an invalid transactionID; manual reconciliation required"
                        .to_string(),
                ))
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

fn receipt_from_submit_response(
    response: RelayerSubmitResponse,
    owner: Option<Address>,
) -> Result<ParsedTransactionReceipt> {
    if response.transaction_id.trim().is_empty() {
        return Err(RelayerError::Other(
            "relayer response transactionID must not be empty".to_string(),
        ));
    }
    let transaction_id = validate_transaction_id(&response.transaction_id).map_err(|_| {
        RelayerError::Other("relayer response transactionID was invalid".to_string())
    })?;
    let transaction_hash = response
        .transaction_hash
        .as_deref()
        .map(str::trim)
        .filter(|hash| !hash.is_empty())
        .map(validate_transaction_hash)
        .transpose()?;

    Ok(ParsedTransactionReceipt {
        receipt: DepositWalletTransactionReceipt {
            transaction_id,
            state: response.state,
            transaction_hash,
            owner,
        },
        owner,
    })
}

pub(super) fn validate_transaction_id(transaction_id: &str) -> Result<String> {
    if transaction_id.is_empty()
        || transaction_id.len() > MAX_TRANSACTION_ID_LEN
        || transaction_id.trim() != transaction_id
        || !transaction_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(RelayerError::Other(
            "transaction id must be 1-128 ASCII letters, digits, dash, underscore, or dot"
                .to_string(),
        ));
    }

    Ok(transaction_id.to_string())
}

fn validate_transaction_hash(transaction_hash: &str) -> Result<String> {
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

fn deserialize_optional_address<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<Address>, D::Error>
where
    D: Deserializer<'de>,
{
    let Some(raw) = Option::<String>::deserialize(deserializer)? else {
        return Ok(None);
    };
    raw.parse().map(Some).map_err(serde::de::Error::custom)
}
