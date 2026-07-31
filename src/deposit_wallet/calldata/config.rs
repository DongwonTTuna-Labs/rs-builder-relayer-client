use std::str::FromStr;

use ethers::types::Address;
use ethers::utils::to_checksum;
use serde::{Serialize, Serializer};

use crate::error::{RelayerError, Result};

pub const POLYGON_PUSD: &str = "0xC011a7E12a19f7B1f670d46F03B03f3342E82DFB";
pub const POLYGON_CTF: &str = "0x4D97DCd97eC945f40cF65F87097ACe5EA0476045";
pub const POLYGON_STANDARD_EXCHANGE: &str = "0xE111180000d2663C0091e4f400237545B87B996B";
pub const POLYGON_NEG_RISK_EXCHANGE: &str = "0xe2222d279d744050d28e00520010520000310F59";
pub const POLYGON_NEG_RISK_ADAPTER: &str = "0xd91E80cF2E7be2e162c6513ceD06f1dD0dA35296";
pub const PUSD_DECIMALS: u8 = 6;

const POLYGON_CHAIN_ID: u64 = 137;
const MAX_SOURCE_TEXT_BYTES: usize = 256;
const MAX_SOURCE_URL_BYTES: usize = 512;

const DEPOSIT_WALLET_DOCS_SOURCE_NAME: &str = "Polymarket deposit-wallets docs";
const DEPOSIT_WALLET_DOCS_VERSION: &str = "retrieved 2026-07-31";
const DEPOSIT_WALLET_DOCS_URL: &str =
    "https://docs.polymarket.com/trading/deposit-wallets";
const RUST_CLOB_SDK_SOURCE_NAME: &str = "Polymarket Rust CLOB SDK";
const RUST_CLOB_SDK_COMMIT: &str = "3ae1aae5e9ded38f984464c9fc0f307f8a9f41fb";
const RUST_CLOB_SDK_UTILITIES_URL: &str =
    "https://github.com/Polymarket/rs-clob-client-v2/blob/3ae1aae5e9ded38f984464c9fc0f307f8a9f41fb/src/clob/utilities.rs";
const RUST_CLOB_SDK_LIB_URL: &str =
    "https://github.com/Polymarket/rs-clob-client-v2/blob/3ae1aae5e9ded38f984464c9fc0f307f8a9f41fb/src/lib.rs";

/// Source metadata required for every calldata address and decimals value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CalldataSourceRef {
    source_name: String,
    version_or_commit: String,
    url: String,
}

impl CalldataSourceRef {
    pub fn try_new(
        source_name: impl Into<String>,
        version_or_commit: impl Into<String>,
        url: impl Into<String>,
    ) -> Result<Self> {
        let source_name = validate_source_text(
            "source_name",
            source_name.into(),
            MAX_SOURCE_TEXT_BYTES,
        )?;
        let version_or_commit = validate_source_text(
            "version_or_commit",
            version_or_commit.into(),
            MAX_SOURCE_TEXT_BYTES,
        )?;
        let url = validate_source_text("url", url.into(), MAX_SOURCE_URL_BYTES)?;
        if !url.starts_with("https://") {
            return Err(RelayerError::Other(
                "calldata source url must start with https://".to_string(),
            ));
        }

        Ok(Self {
            source_name,
            version_or_commit,
            url,
        })
    }

    pub fn source_name(&self) -> &str {
        &self.source_name
    }

    pub fn version_or_commit(&self) -> &str {
        &self.version_or_commit
    }

    pub fn url(&self) -> &str {
        &self.url
    }
}

/// An address coupled to the source that establishes its intended use.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SourcedAddress {
    #[serde(serialize_with = "serialize_address")]
    address: Address,
    source: CalldataSourceRef,
}

impl SourcedAddress {
    pub fn new(address: Address, source: CalldataSourceRef) -> Self {
        Self { address, source }
    }

    pub fn address(&self) -> Address {
        self.address
    }

    pub fn source(&self) -> &CalldataSourceRef {
        &self.source
    }
}

/// Unvalidated input for [`DepositWalletCalldataConfig::try_new`].
pub struct CalldataConfigInput {
    pub chain_id: u64,
    pub pusd: SourcedAddress,
    pub ctf: SourcedAddress,
    pub pusd_decimals: u8,
    pub pusd_decimals_source: CalldataSourceRef,
    pub pusd_spender_allowlist: Vec<SourcedAddress>,
    pub ctf_operator_allowlist: Vec<SourcedAddress>,
    pub adapter_allowlist: Vec<SourcedAddress>,
}

/// A source-backed, wire-truth-bound configuration for calldata builders.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DepositWalletCalldataConfig {
    chain_id: u64,
    pusd: SourcedAddress,
    ctf: SourcedAddress,
    pusd_decimals: u8,
    pusd_decimals_source: CalldataSourceRef,
    pusd_spender_allowlist: Vec<SourcedAddress>,
    ctf_operator_allowlist: Vec<SourcedAddress>,
    adapter_allowlist: Vec<SourcedAddress>,
}

impl DepositWalletCalldataConfig {
    pub fn try_new(input: CalldataConfigInput) -> Result<Self> {
        if input.chain_id != POLYGON_CHAIN_ID {
            return Err(RelayerError::Other(format!(
                "calldata config is not supported for chain {}",
                input.chain_id
            )));
        }
        if input.pusd_spender_allowlist.is_empty() {
            return Err(RelayerError::Other(
                "calldata config pUSD spender allowlist must not be empty".to_string(),
            ));
        }
        if input.ctf_operator_allowlist.is_empty() {
            return Err(RelayerError::Other(
                "calldata config CTF operator allowlist must not be empty".to_string(),
            ));
        }

        validate_nonzero("pUSD address", input.pusd.address())?;
        validate_nonzero("CTF address", input.ctf.address())?;
        validate_nonzero_entries("pUSD spender allowlist", &input.pusd_spender_allowlist)?;
        validate_nonzero_entries("CTF operator allowlist", &input.ctf_operator_allowlist)?;
        validate_nonzero_entries("adapter allowlist", &input.adapter_allowlist)?;

        if input.pusd.address() == input.ctf.address() {
            return Err(RelayerError::Other(
                "calldata config pUSD and CTF addresses must differ".to_string(),
            ));
        }
        validate_no_duplicates("pUSD spender allowlist", &input.pusd_spender_allowlist)?;
        validate_no_duplicates("CTF operator allowlist", &input.ctf_operator_allowlist)?;
        validate_no_duplicates("adapter allowlist", &input.adapter_allowlist)?;

        if input
            .pusd_spender_allowlist
            .iter()
            .chain(input.ctf_operator_allowlist.iter())
            .chain(input.adapter_allowlist.iter())
            .any(|entry| entry.address() == input.pusd.address())
        {
            return Err(RelayerError::Other(
                "calldata config allowlists must not contain the pUSD token itself".to_string(),
            ));
        }

        let polygon_pusd = parse_embedded_address(POLYGON_PUSD)?;
        let polygon_ctf = parse_embedded_address(POLYGON_CTF)?;
        let polygon_standard_exchange = parse_embedded_address(POLYGON_STANDARD_EXCHANGE)?;
        let polygon_neg_risk_exchange = parse_embedded_address(POLYGON_NEG_RISK_EXCHANGE)?;
        let polygon_neg_risk_adapter = parse_embedded_address(POLYGON_NEG_RISK_ADAPTER)?;

        if input.pusd.address() != polygon_pusd {
            return Err(RelayerError::Other(
                "calldata config pUSD address does not match Polygon wire truth".to_string(),
            ));
        }
        if input.ctf.address() != polygon_ctf {
            return Err(RelayerError::Other(
                "calldata config CTF address does not match Polygon wire truth".to_string(),
            ));
        }
        if input.pusd_decimals != PUSD_DECIMALS {
            return Err(RelayerError::Other(format!(
                "calldata config pUSD decimals must match Polygon wire truth ({PUSD_DECIMALS})"
            )));
        }

        let allowed_pusd_spenders = [
            polygon_ctf,
            polygon_standard_exchange,
            polygon_neg_risk_exchange,
        ];
        if input
            .pusd_spender_allowlist
            .iter()
            .any(|entry| !allowed_pusd_spenders.contains(&entry.address()))
        {
            return Err(RelayerError::Other(
                "calldata config pUSD spender allowlist contains an address outside Polygon wire truth"
                    .to_string(),
            ));
        }

        let allowed_ctf_operators = [polygon_standard_exchange, polygon_neg_risk_exchange];
        if input
            .ctf_operator_allowlist
            .iter()
            .any(|entry| !allowed_ctf_operators.contains(&entry.address()))
        {
            return Err(RelayerError::Other(
                "calldata config CTF operator allowlist contains an address outside Polygon wire truth"
                    .to_string(),
            ));
        }
        let allowed_adapters = [polygon_neg_risk_adapter];
        if input
            .adapter_allowlist
            .iter()
            .any(|entry| !allowed_adapters.contains(&entry.address()))
        {
            return Err(RelayerError::Other(
                "calldata config adapter allowlist contains an address outside Polygon wire truth"
                    .to_string(),
            ));
        }

        Ok(Self {
            chain_id: input.chain_id,
            pusd: input.pusd,
            ctf: input.ctf,
            pusd_decimals: input.pusd_decimals,
            pusd_decimals_source: input.pusd_decimals_source,
            pusd_spender_allowlist: input.pusd_spender_allowlist,
            ctf_operator_allowlist: input.ctf_operator_allowlist,
            adapter_allowlist: input.adapter_allowlist,
        })
    }

    pub fn chain_id(&self) -> u64 {
        self.chain_id
    }

    pub fn pusd(&self) -> &SourcedAddress {
        &self.pusd
    }

    pub fn ctf(&self) -> &SourcedAddress {
        &self.ctf
    }

    pub fn pusd_decimals(&self) -> u8 {
        self.pusd_decimals
    }

    pub fn pusd_decimals_source(&self) -> &CalldataSourceRef {
        &self.pusd_decimals_source
    }

    pub fn pusd_spender_allowlist(&self) -> &[SourcedAddress] {
        &self.pusd_spender_allowlist
    }

    pub fn ctf_operator_allowlist(&self) -> &[SourcedAddress] {
        &self.ctf_operator_allowlist
    }

    pub fn adapter_allowlist(&self) -> &[SourcedAddress] {
        &self.adapter_allowlist
    }

    pub fn is_allowed_pusd_spender(&self, address: Address) -> bool {
        self.pusd_spender_allowlist
            .iter()
            .any(|entry| entry.address() == address)
    }

    pub fn is_allowed_ctf_operator(&self, address: Address) -> bool {
        self.ctf_operator_allowlist
            .iter()
            .any(|entry| entry.address() == address)
    }

    pub fn is_allowed_adapter(&self, address: Address) -> bool {
        self.adapter_allowlist
            .iter()
            .any(|entry| entry.address() == address)
    }
}

/// Build the canonical Polygon config from the reviewed PBRSDK-17 wire truth.
pub fn polygon_calldata_config() -> Result<DepositWalletCalldataConfig> {
    let docs_source = deposit_wallet_docs_source()?;
    let decimals_source = CalldataSourceRef::try_new(
        RUST_CLOB_SDK_SOURCE_NAME,
        RUST_CLOB_SDK_COMMIT,
        RUST_CLOB_SDK_UTILITIES_URL,
    )?;
    let adapter_source = CalldataSourceRef::try_new(
        RUST_CLOB_SDK_SOURCE_NAME,
        RUST_CLOB_SDK_COMMIT,
        RUST_CLOB_SDK_LIB_URL,
    )?;

    let pusd = sourced_embedded_address(POLYGON_PUSD, docs_source.clone())?;
    let ctf = sourced_embedded_address(POLYGON_CTF, docs_source.clone())?;
    let standard_exchange =
        sourced_embedded_address(POLYGON_STANDARD_EXCHANGE, docs_source.clone())?;
    let neg_risk_exchange =
        sourced_embedded_address(POLYGON_NEG_RISK_EXCHANGE, docs_source)?;
    let neg_risk_adapter =
        sourced_embedded_address(POLYGON_NEG_RISK_ADAPTER, adapter_source)?;

    DepositWalletCalldataConfig::try_new(CalldataConfigInput {
        chain_id: POLYGON_CHAIN_ID,
        pusd,
        ctf: ctf.clone(),
        pusd_decimals: PUSD_DECIMALS,
        pusd_decimals_source: decimals_source,
        pusd_spender_allowlist: vec![
            ctf,
            standard_exchange.clone(),
            neg_risk_exchange.clone(),
        ],
        ctf_operator_allowlist: vec![standard_exchange, neg_risk_exchange],
        adapter_allowlist: vec![neg_risk_adapter],
    })
}

fn deposit_wallet_docs_source() -> Result<CalldataSourceRef> {
    CalldataSourceRef::try_new(
        DEPOSIT_WALLET_DOCS_SOURCE_NAME,
        DEPOSIT_WALLET_DOCS_VERSION,
        DEPOSIT_WALLET_DOCS_URL,
    )
}

fn sourced_embedded_address(
    value: &str,
    source: CalldataSourceRef,
) -> Result<SourcedAddress> {
    Ok(SourcedAddress::new(
        parse_embedded_address(value)?,
        source,
    ))
}

fn parse_embedded_address(value: &str) -> Result<Address> {
    Address::from_str(value).map_err(|error| {
        RelayerError::InvalidAddress(format!(
            "invalid embedded calldata address {value}: {error}"
        ))
    })
}

fn validate_source_text(label: &str, value: String, max_bytes: usize) -> Result<String> {
    if value.chars().any(char::is_control) {
        return Err(RelayerError::Other(format!(
            "calldata source {label} must not contain control characters"
        )));
    }
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(RelayerError::Other(format!(
            "calldata source {label} must not be empty"
        )));
    }
    if trimmed.len() > max_bytes {
        return Err(RelayerError::Other(format!(
            "calldata source {label} must not exceed {max_bytes} bytes"
        )));
    }
    Ok(trimmed.to_string())
}

fn validate_nonzero(label: &str, address: Address) -> Result<()> {
    if address == Address::zero() {
        return Err(RelayerError::Other(format!(
            "calldata config {label} must not be zero"
        )));
    }
    Ok(())
}

fn validate_nonzero_entries(label: &str, entries: &[SourcedAddress]) -> Result<()> {
    for entry in entries {
        validate_nonzero(label, entry.address())?;
    }
    Ok(())
}

fn validate_no_duplicates(label: &str, entries: &[SourcedAddress]) -> Result<()> {
    for (index, entry) in entries.iter().enumerate() {
        if entries[index + 1..]
            .iter()
            .any(|candidate| candidate.address() == entry.address())
        {
            return Err(RelayerError::Other(format!(
                "calldata config {label} must not contain duplicate addresses"
            )));
        }
    }
    Ok(())
}

fn serialize_address<S>(address: &Address, serializer: S) -> std::result::Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(&to_checksum(address, None))
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use serde_json::Value;

    use super::*;

    #[test]
    fn canonical_polygon_config_exposes_reviewed_values_sources_and_membership() {
        let config = polygon_calldata_config().expect("canonical Polygon config should validate");
        let pusd = address(POLYGON_PUSD);
        let ctf = address(POLYGON_CTF);
        let standard_exchange = address(POLYGON_STANDARD_EXCHANGE);
        let neg_risk_exchange = address(POLYGON_NEG_RISK_EXCHANGE);
        let neg_risk_adapter = address(POLYGON_NEG_RISK_ADAPTER);
        let outsider = arbitrary_address(1);

        assert_eq!(config.chain_id(), POLYGON_CHAIN_ID);
        assert_eq!(config.pusd().address(), pusd);
        assert_eq!(config.ctf().address(), ctf);
        assert_eq!(config.pusd_decimals(), PUSD_DECIMALS);
        assert_source(
            config.pusd().source(),
            DEPOSIT_WALLET_DOCS_SOURCE_NAME,
            DEPOSIT_WALLET_DOCS_VERSION,
            DEPOSIT_WALLET_DOCS_URL,
        );
        assert_source(
            config.ctf().source(),
            DEPOSIT_WALLET_DOCS_SOURCE_NAME,
            DEPOSIT_WALLET_DOCS_VERSION,
            DEPOSIT_WALLET_DOCS_URL,
        );
        assert_source(
            config.pusd_decimals_source(),
            RUST_CLOB_SDK_SOURCE_NAME,
            RUST_CLOB_SDK_COMMIT,
            RUST_CLOB_SDK_UTILITIES_URL,
        );

        let spenders = config.pusd_spender_allowlist();
        assert_eq!(spenders.len(), 3);
        assert_eq!(spenders[0].address(), ctf);
        assert_eq!(spenders[1].address(), standard_exchange);
        assert_eq!(spenders[2].address(), neg_risk_exchange);
        for spender in spenders {
            assert_source(
                spender.source(),
                DEPOSIT_WALLET_DOCS_SOURCE_NAME,
                DEPOSIT_WALLET_DOCS_VERSION,
                DEPOSIT_WALLET_DOCS_URL,
            );
        }

        let operators = config.ctf_operator_allowlist();
        assert_eq!(operators.len(), 2);
        assert_eq!(operators[0].address(), standard_exchange);
        assert_eq!(operators[1].address(), neg_risk_exchange);
        for operator in operators {
            assert_source(
                operator.source(),
                DEPOSIT_WALLET_DOCS_SOURCE_NAME,
                DEPOSIT_WALLET_DOCS_VERSION,
                DEPOSIT_WALLET_DOCS_URL,
            );
        }

        let adapters = config.adapter_allowlist();
        assert_eq!(adapters.len(), 1);
        assert_eq!(adapters[0].address(), neg_risk_adapter);
        assert_source(
            adapters[0].source(),
            RUST_CLOB_SDK_SOURCE_NAME,
            RUST_CLOB_SDK_COMMIT,
            RUST_CLOB_SDK_LIB_URL,
        );
        assert!(config.is_allowed_pusd_spender(ctf));
        assert!(config.is_allowed_pusd_spender(standard_exchange));
        assert!(config.is_allowed_pusd_spender(neg_risk_exchange));
        assert!(!config.is_allowed_pusd_spender(outsider));
        assert!(config.is_allowed_ctf_operator(standard_exchange));
        assert!(config.is_allowed_ctf_operator(neg_risk_exchange));
        assert!(!config.is_allowed_ctf_operator(ctf));
        assert!(!config.is_allowed_ctf_operator(outsider));
        assert!(config.is_allowed_adapter(neg_risk_adapter));
        assert!(!config.is_allowed_adapter(outsider));
    }

    #[test]
    fn strict_subsets_are_preserved_without_global_allowlist_expansion() {
        let mut input = canonical_input();
        input.pusd_spender_allowlist = vec![input.pusd_spender_allowlist[0].clone()];
        input.ctf_operator_allowlist = vec![input.ctf_operator_allowlist[0].clone()];

        let config = DepositWalletCalldataConfig::try_new(input)
            .expect("reviewed strict subsets should remain valid");
        let ctf = address(POLYGON_CTF);
        let standard_exchange = address(POLYGON_STANDARD_EXCHANGE);
        let neg_risk_exchange = address(POLYGON_NEG_RISK_EXCHANGE);

        assert_eq!(config.pusd_spender_allowlist().len(), 1);
        assert_eq!(config.pusd_spender_allowlist()[0].address(), ctf);
        assert_eq!(config.ctf_operator_allowlist().len(), 1);
        assert_eq!(
            config.ctf_operator_allowlist()[0].address(),
            standard_exchange
        );
        assert!(config.is_allowed_pusd_spender(ctf));
        assert!(!config.is_allowed_pusd_spender(standard_exchange));
        assert!(!config.is_allowed_pusd_spender(neg_risk_exchange));
        assert!(config.is_allowed_ctf_operator(standard_exchange));
        assert!(!config.is_allowed_ctf_operator(neg_risk_exchange));
    }

    #[test]
    fn unsupported_chain_is_rejected() {
        let mut input = canonical_input();
        input.chain_id = 80002;

        assert_eq!(
            config_error(input),
            "calldata config is not supported for chain 80002"
        );
    }

    #[test]
    fn structural_duplicates_collisions_zero_and_empty_lists_are_rejected() {
        let mut duplicate_spender = canonical_input();
        duplicate_spender
            .pusd_spender_allowlist
            .push(duplicate_spender.pusd_spender_allowlist[0].clone());
        assert_eq!(
            config_error(duplicate_spender),
            "calldata config pUSD spender allowlist must not contain duplicate addresses"
        );

        let mut duplicate_operator = canonical_input();
        duplicate_operator
            .ctf_operator_allowlist
            .push(duplicate_operator.ctf_operator_allowlist[0].clone());
        assert_eq!(
            config_error(duplicate_operator),
            "calldata config CTF operator allowlist must not contain duplicate addresses"
        );

        let mut same_tokens = canonical_input();
        same_tokens.ctf = same_tokens.pusd.clone();
        assert_eq!(
            config_error(same_tokens),
            "calldata config pUSD and CTF addresses must differ"
        );

        let mut self_approval = canonical_input();
        self_approval.pusd_spender_allowlist = vec![self_approval.pusd.clone()];
        assert_eq!(
            config_error(self_approval),
            "calldata config allowlists must not contain the pUSD token itself"
        );

        let mut zero = canonical_input();
        zero.pusd_spender_allowlist[0] =
            SourcedAddress::new(Address::zero(), docs_source());
        assert_eq!(
            config_error(zero),
            "calldata config pUSD spender allowlist must not be zero"
        );

        let mut empty_spenders = canonical_input();
        empty_spenders.pusd_spender_allowlist.clear();
        assert_eq!(
            config_error(empty_spenders),
            "calldata config pUSD spender allowlist must not be empty"
        );

        let mut empty_operators = canonical_input();
        empty_operators.ctf_operator_allowlist.clear();
        assert_eq!(
            config_error(empty_operators),
            "calldata config CTF operator allowlist must not be empty"
        );
    }

    #[test]
    fn every_wire_truth_binding_has_a_distinct_failure() {
        let mut messages = Vec::new();

        let mut wrong_pusd = canonical_input();
        wrong_pusd.pusd = SourcedAddress::new(arbitrary_address(1), docs_source());
        messages.push(config_error(wrong_pusd));

        let mut wrong_ctf = canonical_input();
        wrong_ctf.ctf = SourcedAddress::new(arbitrary_address(2), docs_source());
        messages.push(config_error(wrong_ctf));

        let mut wrong_decimals = canonical_input();
        wrong_decimals.pusd_decimals = 18;
        messages.push(config_error(wrong_decimals));

        let mut wrong_spender = canonical_input();
        wrong_spender
            .pusd_spender_allowlist
            .push(SourcedAddress::new(arbitrary_address(3), docs_source()));
        messages.push(config_error(wrong_spender));

        let mut wrong_operator = canonical_input();
        wrong_operator
            .ctf_operator_allowlist
            .push(wrong_operator.ctf.clone());
        messages.push(config_error(wrong_operator));

        let mut adapter = canonical_input();
        adapter
            .adapter_allowlist
            .push(SourcedAddress::new(arbitrary_address(4), docs_source()));
        messages.push(config_error(adapter));

        assert_eq!(
            messages,
            vec![
                "calldata config pUSD address does not match Polygon wire truth",
                "calldata config CTF address does not match Polygon wire truth",
                "calldata config pUSD decimals must match Polygon wire truth (6)",
                "calldata config pUSD spender allowlist contains an address outside Polygon wire truth",
                "calldata config CTF operator allowlist contains an address outside Polygon wire truth",
                "calldata config adapter allowlist contains an address outside Polygon wire truth",
            ]
        );
        let distinct: HashSet<&str> = messages.iter().map(String::as_str).collect();
        assert_eq!(distinct.len(), messages.len());
    }

    #[test]
    fn source_metadata_rejects_empty_oversized_control_and_insecure_values() {
        assert_source_error("", "commit", "https://example.com", "source_name");
        assert_source_error(
            &"s".repeat(MAX_SOURCE_TEXT_BYTES + 1),
            "commit",
            "https://example.com",
            "source_name",
        );
        assert_source_error(
            "source\nname",
            "commit",
            "https://example.com",
            "source_name",
        );
        assert_source_error(
            "\nsource",
            "commit",
            "https://example.com",
            "control characters",
        );

        assert_source_error("source", "", "https://example.com", "version_or_commit");
        assert_source_error(
            "source",
            &"v".repeat(MAX_SOURCE_TEXT_BYTES + 1),
            "https://example.com",
            "version_or_commit",
        );
        assert_source_error(
            "source",
            "commit\tvalue",
            "https://example.com",
            "version_or_commit",
        );
        assert_source_error(
            "source",
            "commit\t",
            "https://example.com",
            "control characters",
        );

        assert_source_error("source", "commit", "http://example.com", "https://");
        let oversized_url = format!("https://{}", "u".repeat(MAX_SOURCE_URL_BYTES));
        assert_source_error("source", "commit", &oversized_url, "url");
        assert_source_error(
            "source",
            "commit",
            "https://exam\nple.com",
            "control characters",
        );
        assert_source_error(
            "source",
            "commit",
            "https://example.com\n",
            "control characters",
        );
    }

    #[test]
    fn serialize_uses_checksum_addresses_and_includes_source_metadata() {
        let config = polygon_calldata_config().expect("canonical Polygon config should validate");
        let value = serde_json::to_value(&config).expect("config should serialize");

        assert_eq!(value["chain_id"], Value::from(POLYGON_CHAIN_ID));
        assert_eq!(value["pusd"]["address"], Value::from(POLYGON_PUSD));
        assert_eq!(value["ctf"]["address"], Value::from(POLYGON_CTF));
        assert_eq!(
            value["pusd_spender_allowlist"][1]["address"],
            Value::from(POLYGON_STANDARD_EXCHANGE)
        );
        assert_eq!(
            value["ctf_operator_allowlist"][1]["address"],
            Value::from(POLYGON_NEG_RISK_EXCHANGE)
        );
        assert_eq!(
            value["pusd"]["source"]["source_name"],
            Value::from(DEPOSIT_WALLET_DOCS_SOURCE_NAME)
        );
        assert_eq!(
            value["pusd"]["source"]["version_or_commit"],
            Value::from(DEPOSIT_WALLET_DOCS_VERSION)
        );
        assert_eq!(
            value["pusd"]["source"]["url"],
            Value::from(DEPOSIT_WALLET_DOCS_URL)
        );
        assert_eq!(
            value["pusd_decimals_source"]["version_or_commit"],
            Value::from(RUST_CLOB_SDK_COMMIT)
        );

        let serialized = serde_json::to_string(&config).expect("config should serialize");
        assert!(!serialized.contains(&format!(
            "\"{}\"",
            POLYGON_PUSD.to_ascii_lowercase()
        )));
        assert!(!serialized.contains(&format!(
            "\"{}\"",
            POLYGON_CTF.to_ascii_lowercase()
        )));
        assert!(value.get("sourceName").is_none());
    }

    fn canonical_input() -> CalldataConfigInput {
        let docs_source = docs_source();
        let pusd = SourcedAddress::new(address(POLYGON_PUSD), docs_source.clone());
        let ctf = SourcedAddress::new(address(POLYGON_CTF), docs_source.clone());
        let standard_exchange =
            SourcedAddress::new(address(POLYGON_STANDARD_EXCHANGE), docs_source.clone());
        let neg_risk_exchange =
            SourcedAddress::new(address(POLYGON_NEG_RISK_EXCHANGE), docs_source);

        CalldataConfigInput {
            chain_id: POLYGON_CHAIN_ID,
            pusd,
            ctf: ctf.clone(),
            pusd_decimals: PUSD_DECIMALS,
            pusd_decimals_source: decimals_source(),
            pusd_spender_allowlist: vec![
                ctf,
                standard_exchange.clone(),
                neg_risk_exchange.clone(),
            ],
            ctf_operator_allowlist: vec![standard_exchange, neg_risk_exchange],
            adapter_allowlist: Vec::new(),
        }
    }

    fn docs_source() -> CalldataSourceRef {
        CalldataSourceRef::try_new(
            DEPOSIT_WALLET_DOCS_SOURCE_NAME,
            DEPOSIT_WALLET_DOCS_VERSION,
            DEPOSIT_WALLET_DOCS_URL,
        )
        .expect("reviewed docs source should validate")
    }

    fn decimals_source() -> CalldataSourceRef {
        CalldataSourceRef::try_new(
            RUST_CLOB_SDK_SOURCE_NAME,
            RUST_CLOB_SDK_COMMIT,
            RUST_CLOB_SDK_UTILITIES_URL,
        )
        .expect("reviewed SDK source should validate")
    }

    fn address(value: &str) -> Address {
        Address::from_str(value).expect("reviewed address should parse")
    }

    fn arbitrary_address(value: u64) -> Address {
        Address::from_low_u64_be(value)
    }

    fn config_error(input: CalldataConfigInput) -> String {
        DepositWalletCalldataConfig::try_new(input)
            .expect_err("invalid config should be rejected")
            .to_string()
    }

    fn assert_source(
        source: &CalldataSourceRef,
        expected_name: &str,
        expected_version: &str,
        expected_url: &str,
    ) {
        assert_eq!(source.source_name(), expected_name);
        assert_eq!(source.version_or_commit(), expected_version);
        assert_eq!(source.url(), expected_url);
    }

    fn assert_source_error(source_name: &str, version: &str, url: &str, expected: &str) {
        let error = CalldataSourceRef::try_new(source_name, version, url)
            .expect_err("invalid source metadata should be rejected")
            .to_string();
        assert!(
            error.contains(expected),
            "expected {expected:?} in source validation error: {error}"
        );
    }
}
