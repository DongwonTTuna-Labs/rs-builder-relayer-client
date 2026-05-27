use super::*;
use super::redaction::{
    external_token_hash, redacted_address, sanitized_external_token, unknown_state_error_summary,
};
use crate::deposit_wallet::{WALLET_CREATE_TRANSACTION_TYPE, WALLET_TRANSACTION_TYPE};

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
    pub(super) retryable_absence: bool,
}

impl PollFetchError {
    pub(super) fn from_response_error(error: ResponseError) -> Self {
        Self {
            error: error.error,
            retry_after: error.retry_after,
            owner: None,
            retryable_absence: false,
        }
    }

    pub(super) fn from_transaction_parse_error(error: TransactionParseError) -> Self {
        Self {
            error: error.error,
            retry_after: None,
            owner: error.owner,
            retryable_absence: error.retryable_absence,
        }
    }
}

#[derive(Debug)]
pub(super) struct TransactionParseError {
    pub(super) error: RelayerError,
    pub(super) owner: Option<Address>,
    pub(super) retryable_absence: bool,
}

impl TransactionParseError {
    pub(super) fn new(error: RelayerError, owner: Option<Address>) -> Self {
        Self {
            error,
            owner,
            retryable_absence: false,
        }
    }

    pub(super) fn retryable_absence(error: RelayerError) -> Self {
        Self {
            error,
            owner: None,
            retryable_absence: true,
        }
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
    // Official GET /transaction responses include owner as the owner address.
    // The parsed owner is retained separately so owner-scoped polling can
    // validate it before mutating local state.
    #[serde(default, deserialize_with = "deserialize_optional_address")]
    owner: Option<Address>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubmitTransactionIdOnly {
    #[serde(rename = "transactionID", alias = "transactionId")]
    transaction_id: String,
}

pub(super) fn parse_submit_response(bytes: &[u8]) -> Result<DepositWalletTransactionReceipt> {
    let response = parse_submit_response_body(bytes)?;
    receipt_from_submit_response(response, None).map(|parsed| parsed.receipt)
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
        Some(b'{') => serde_json::from_slice::<RelayerSubmitResponse>(bytes).map_err(|_| {
            RelayerError::Other("could not parse submit response object".to_string())
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
    expected_to: Address,
    bytes: &[u8],
) -> std::result::Result<ParsedTransactionReceipt, TransactionParseError> {
    match bytes.iter().copied().find(|byte| !byte.is_ascii_whitespace()) {
        Some(b'{') => {
            let response = serde_json::from_slice::<RelayerTransactionResponseWithOwner>(bytes)
                .map_err(|_| {
                    TransactionParseError::new(
                        RelayerError::Other("could not parse transaction response object".to_string()),
                        None,
                    )
                })?;
            return parse_verified_transaction_response(expected_transaction_id, expected_to, response);
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
    parse_verified_transaction_response(expected_transaction_id, expected_to, response)
}

fn parse_verified_transaction_response(
    expected_transaction_id: &str,
    expected_to: Address,
    response: RelayerTransactionResponseWithOwner,
) -> std::result::Result<ParsedTransactionReceipt, TransactionParseError> {
    let owner = response.owner;
    let response_transaction_id = validate_transaction_id(&response.response.transaction_id)
        .map_err(|_| {
            TransactionParseError::new(
                RelayerError::Other("relayer response transactionID was invalid".to_string()),
                None,
            )
        })?;
    if response_transaction_id != expected_transaction_id {
        return Err(TransactionParseError::new(
            RelayerError::reconciliation_required(format!(
                "transaction response id hash {} did not match requested id hash {}",
                external_token_hash(&response_transaction_id),
                external_token_hash(expected_transaction_id)
            )),
            None,
        ));
    }
    validate_transaction_wire_evidence(&response, expected_to, owner)?;
    let parsed = receipt_from_submit_response(response.response, owner)
        .map_err(|error| TransactionParseError::new(error, owner))?;
    Ok(parsed)
}

fn validate_transaction_wire_evidence(
    response: &RelayerTransactionResponseWithOwner,
    expected_to: Address,
    owner: Option<Address>,
) -> std::result::Result<(), TransactionParseError> {
    let tx_type = response.tx_type.as_deref().ok_or_else(|| {
        TransactionParseError::new(
            RelayerError::reconciliation_required(
                "transaction response did not include deposit-wallet transaction type; manual reconciliation required"
                    .to_string(),
            ),
            owner,
        )
    })?;
    if !matches!(tx_type, WALLET_CREATE_TRANSACTION_TYPE | WALLET_TRANSACTION_TYPE) {
        return Err(TransactionParseError::new(
            RelayerError::reconciliation_required(
                "transaction response type was not WALLET or WALLET-CREATE; manual reconciliation required"
                    .to_string(),
            ),
            owner,
        ));
    }

    let owner = owner.ok_or_else(|| {
        TransactionParseError::new(
            RelayerError::reconciliation_required(
                "deposit wallet transaction response did not include owner evidence; manual reconciliation required"
                    .to_string(),
            ),
            None,
        )
    })?;
    let from_address = response.from_address.ok_or_else(|| {
        TransactionParseError::new(
            RelayerError::reconciliation_required(
                "transaction response did not include from address; manual reconciliation required"
                    .to_string(),
            ),
            Some(owner),
        )
    })?;
    if from_address != owner {
        return Err(TransactionParseError::new(
            RelayerError::reconciliation_required(format!(
                "transaction response from address {} did not match owner evidence {}; manual reconciliation required",
                redacted_address(from_address),
                redacted_address(owner)
            )),
            Some(owner),
        ));
    }

    let to = response.to.ok_or_else(|| {
        TransactionParseError::new(
            RelayerError::reconciliation_required(
                "transaction response did not include to address; manual reconciliation required"
                    .to_string(),
            ),
            Some(owner),
        )
    })?;
    if to != expected_to {
        return Err(TransactionParseError::new(
            RelayerError::reconciliation_required(format!(
                "transaction response to address {} did not match expected relayer target {}; manual reconciliation required",
                redacted_address(to),
                redacted_address(expected_to)
            )),
            Some(owner),
        ));
    }

    Ok(())
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
                let response_transaction_id =
                    validate_transaction_id(&response.response.transaction_id)
                        .map_err(|_| de::Error::custom(TRANSACTION_RESPONSE_INVALID_ID_ERROR))?;
                if response_transaction_id == self.expected_transaction_id {
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
                TransactionParseError::retryable_absence(
                    RelayerError::reconciliation_required(format!(
                        "transaction response did not include requested transaction id hash {}",
                        external_token_hash(expected_transaction_id)
                    )),
                )
            } else if message.contains(TRANSACTION_RESPONSE_DUPLICATE_ID_ERROR) {
                TransactionParseError::new(
                    RelayerError::reconciliation_required(format!(
                        "transaction response included duplicate requested transaction id hash {}; manual reconciliation required",
                        external_token_hash(expected_transaction_id)
                    )),
                    None,
                )
            } else if message.contains(TRANSACTION_RESPONSE_INVALID_ID_ERROR) {
                TransactionParseError::new(
                    RelayerError::reconciliation_required(
                        "transaction response included an invalid transactionID; manual reconciliation required"
                            .to_string(),
                    ),
                    None,
                )
            } else {
                TransactionParseError::new(
                    RelayerError::Other("could not parse transaction response array".to_string()),
                    None,
                )
            }
        })?;
    deserializer.end().map_err(|_| {
        TransactionParseError::new(
            RelayerError::Other("could not parse transaction response array".to_string()),
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

pub(super) fn validate_transaction_id(transaction_id: &str) -> Result<String> {
    if transaction_id.is_empty()
        || transaction_id.len() > MAX_TRANSACTION_ID_LEN
        || transaction_id.trim() != transaction_id
        || !transaction_id
            .chars()
            .all(|character| !character.is_control() && !character.is_whitespace())
    {
        return Err(RelayerError::Other(
            "transaction id must be 1-128 non-whitespace, non-control characters".to_string(),
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
