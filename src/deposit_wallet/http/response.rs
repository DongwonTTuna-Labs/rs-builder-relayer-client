use super::*;
use super::redaction::sanitized_external_token;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DepositWalletTransactionReceipt {
    pub transaction_id: String,
    pub state: RelayerTransactionState,
    pub transaction_hash: Option<String>,
    pub owner: Option<Address>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ParsedTransactionReceipt {
    pub(super) receipt: DepositWalletTransactionReceipt,
    pub(super) owner: Option<Address>,
}

#[derive(Debug)]
pub(super) struct ResponseError {
    pub(super) error: RelayerError,
    pub(super) retry_after: Option<Duration>,
}

impl ResponseError {
    pub(super) fn new(error: RelayerError, retry_after: Option<Duration>) -> Self {
        Self { error, retry_after }
    }
}

#[derive(Debug)]
pub(super) struct PollFetchError {
    pub(super) error: RelayerError,
    pub(super) retry_after: Option<Duration>,
    pub(super) owner: Option<Address>,
}

impl PollFetchError {
    pub(super) fn from_response_error(error: ResponseError) -> Self {
        Self {
            error: error.error,
            retry_after: error.retry_after,
            owner: None,
        }
    }

    pub(super) fn from_transaction_parse_error(error: TransactionParseError) -> Self {
        Self {
            error: error.error,
            retry_after: None,
            owner: error.owner,
        }
    }
}

#[derive(Debug)]
pub(super) struct TransactionParseError {
    pub(super) error: RelayerError,
    pub(super) owner: Option<Address>,
}

impl TransactionParseError {
    pub(super) fn new(error: RelayerError, owner: Option<Address>) -> Self {
        Self { error, owner }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RelayerTransactionResponseWithOwner {
    #[serde(flatten)]
    response: RelayerSubmitResponse,
    // Official GET /transaction responses include owner as the owner address.
    // Keep it internal because the public receipt intentionally exposes only
    // non-sensitive polling identifiers.
    #[serde(default, deserialize_with = "deserialize_optional_address")]
    owner: Option<Address>,
}

pub(super) fn parse_submit_response(bytes: &[u8]) -> Result<DepositWalletTransactionReceipt> {
    let response = serde_json::from_slice::<RelayerSubmitResponse>(bytes)
        .map_err(|e| RelayerError::Other(format!("could not parse submit response: {e}")))?;
    receipt_from_submit_response(response, None).map(|parsed| parsed.receipt)
}

pub(super) fn parse_transaction_response(
    expected_transaction_id: &str,
    bytes: &[u8],
) -> std::result::Result<ParsedTransactionReceipt, TransactionParseError> {
    match bytes.iter().copied().find(|byte| !byte.is_ascii_whitespace()) {
        Some(b'{') => {
            let response = serde_json::from_slice::<RelayerTransactionResponseWithOwner>(bytes)
                .map_err(|e| {
                    TransactionParseError::new(
                        RelayerError::Other(format!("could not parse transaction response: {e}")),
                        None,
                    )
                })?;
            let owner = response.owner;
            let receipt = receipt_from_submit_response(response.response, owner)
                .map_err(|error| TransactionParseError::new(error, owner))?;
            return require_transaction_id_match(expected_transaction_id, receipt)
                .map_err(|error| TransactionParseError::new(error, owner));
        }
        Some(b'[') => {}
        _ => {
            return Err(TransactionParseError::new(
                RelayerError::Other(
                    "could not parse transaction response: expected JSON object or array"
                        .to_string(),
                ),
                None,
            ))
        }
    }

    let response = select_transaction_response_from_array(expected_transaction_id, bytes)?;
    let owner = response.owner;
    let receipt = receipt_from_submit_response(response.response, owner)
        .map_err(|error| TransactionParseError::new(error, owner))?;
    require_transaction_id_match(expected_transaction_id, receipt)
        .map_err(|error| TransactionParseError::new(error, owner))
}

pub(super) fn select_transaction_response_from_array(
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
                if response.response.transaction_id == self.expected_transaction_id {
                    if matching_response.is_some() {
                        return Err(de::Error::custom(TRANSACTION_RESPONSE_DUPLICATE_ID_ERROR));
                    }
                    matching_response = Some(response);
                }
            }
            matching_response
                .ok_or_else(|| de::Error::custom(TRANSACTION_RESPONSE_MISSING_ID_ERROR))
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
                TransactionParseError::new(
                    RelayerError::reconciliation_required(format!(
                        "transaction response included more than {MAX_TRANSACTION_RESPONSE_ITEMS} items"
                    )),
                    None,
                )
            } else if message.contains(TRANSACTION_RESPONSE_MISSING_ID_ERROR) {
                TransactionParseError::new(
                    RelayerError::reconciliation_required(format!(
                        "transaction response did not include requested transaction id {}",
                        sanitized_external_token(expected_transaction_id)
                    )),
                    None,
                )
            } else if message.contains(TRANSACTION_RESPONSE_DUPLICATE_ID_ERROR) {
                TransactionParseError::new(
                    RelayerError::reconciliation_required(format!(
                        "transaction response included duplicate requested transaction id {}; manual reconciliation required",
                        sanitized_external_token(expected_transaction_id)
                    )),
                    None,
                )
            } else {
                TransactionParseError::new(
                    RelayerError::Other(format!("could not parse transaction response: {error}")),
                    None,
                )
            }
        })?;
    deserializer.end().map_err(|error| {
        TransactionParseError::new(
            RelayerError::Other(format!("could not parse transaction response: {error}")),
            None,
        )
    })?;
    Ok(response)
}

pub(super) fn receipt_from_submit_response(
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

pub(super) fn require_transaction_id_match(
    expected_transaction_id: &str,
    parsed: ParsedTransactionReceipt,
) -> Result<ParsedTransactionReceipt> {
    if parsed.receipt.transaction_id != expected_transaction_id {
        return Err(RelayerError::reconciliation_required(format!(
            "transaction response id {} did not match requested id {}",
            sanitized_external_token(&parsed.receipt.transaction_id),
            sanitized_external_token(expected_transaction_id)
        )));
    }
    Ok(parsed)
}

pub(super) fn validate_transaction_id(transaction_id: &str) -> Result<String> {
    if transaction_id.is_empty()
        || transaction_id.len() > MAX_TRANSACTION_ID_LEN
        || !transaction_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(RelayerError::Other(
            "transaction id must be 1-128 ASCII alphanumeric, hyphen, underscore, or period characters"
                .to_string(),
        ));
    }

    Ok(transaction_id.to_string())
}

pub(super) fn validate_transaction_hash(transaction_hash: &str) -> Result<String> {
    if transaction_hash.len() == 66
        && transaction_hash.starts_with("0x")
        && transaction_hash[2..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Ok(transaction_hash.to_string());
    }

    Err(RelayerError::reconciliation_required(
        "relayer response transactionHash was invalid".to_string(),
    ))
}

pub(super) fn deserialize_optional_address<'de, D>(deserializer: D) -> std::result::Result<Option<Address>, D::Error>
where
    D: Deserializer<'de>,
{
    let Some(raw) = Option::<String>::deserialize(deserializer)? else {
        return Ok(None);
    };
    raw.parse().map(Some).map_err(serde::de::Error::custom)
}
