use ethers::abi::{encode, Token};
use ethers::types::{Address, Bytes, H256, U256};
use serde::Serialize;

use crate::deposit_wallet::types::DepositWalletCall;
use crate::error::{RelayerError, Result};

use super::{CtfPositionAmount, DepositWalletCalldataConfig, PusdAmount};

const SPLIT_POSITION_SELECTOR: [u8; 4] = [0x72, 0xce, 0x42, 0x75];
const MERGE_POSITIONS_SELECTOR: [u8; 4] = [0x9e, 0x72, 0x12, 0xad];
const REDEEM_POSITIONS_SELECTOR: [u8; 4] = [0x01, 0xb7, 0x03, 0x7c];
const NEG_RISK_REDEEM_POSITIONS_SELECTOR: [u8; 4] = [0xdb, 0xec, 0xcb, 0x23];
const MAX_CTF_ARRAY_LENGTH: usize = 64;

/// CTF routes whose target, selector, arguments, and units this crate verifies.
///
/// Unsupported routes are deliberately absent from this enum and therefore
/// cannot be selected through the typed deposit-wallet calldata surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum CtfRoute {
    /// Verified `ConditionalTokens.splitPosition` route.
    ConditionalTokensSplit,
    /// Verified `ConditionalTokens.mergePositions` route.
    ConditionalTokensMerge,
    /// Verified `ConditionalTokens.redeemPositions` route.
    ConditionalTokensRedeem,
    /// Verified `NegRiskAdapter.redeemPositions` route.
    NegRiskAdapterRedeem,
}

impl CtfRoute {
    /// Return the selector pinned to this verified route.
    pub fn selector(&self) -> [u8; 4] {
        match self {
            Self::ConditionalTokensSplit => SPLIT_POSITION_SELECTOR,
            Self::ConditionalTokensMerge => MERGE_POSITIONS_SELECTOR,
            Self::ConditionalTokensRedeem => REDEEM_POSITIONS_SELECTOR,
            Self::NegRiskAdapterRedeem => NEG_RISK_REDEEM_POSITIONS_SELECTOR,
        }
    }

    /// Resolve the target for every verified route/adapter input combination.
    pub fn target(
        &self,
        config: &DepositWalletCalldataConfig,
        adapter: Option<Address>,
    ) -> Result<Address> {
        match (*self, adapter) {
            (
                Self::ConditionalTokensSplit
                | Self::ConditionalTokensMerge
                | Self::ConditionalTokensRedeem,
                None,
            ) => Ok(config.ctf().address()),
            (
                Self::ConditionalTokensSplit
                | Self::ConditionalTokensMerge
                | Self::ConditionalTokensRedeem,
                Some(_),
            ) => Err(RelayerError::Other(
                "ConditionalTokens routes must not be given an adapter address".to_string(),
            )),
            (Self::NegRiskAdapterRedeem, None) => Err(RelayerError::Other(
                "NegRisk redeem requires an adapter address".to_string(),
            )),
            (Self::NegRiskAdapterRedeem, Some(address)) => {
                if address == Address::zero() {
                    return Err(RelayerError::Other(
                        "CTF adapter must not be zero".to_string(),
                    ));
                }
                if !config.is_allowed_adapter(address) {
                    return Err(RelayerError::Other(
                        "CTF adapter is not in the verified allowlist".to_string(),
                    ));
                }
                Ok(address)
            }
        }
    }
}

/// Build a verified `ConditionalTokens.splitPosition` call.
pub fn build_split_position_call(
    config: &DepositWalletCalldataConfig,
    condition_id: H256,
    partition: &[U256],
    amount: PusdAmount,
) -> Result<DepositWalletCall> {
    let route = CtfRoute::ConditionalTokensSplit;
    let target = route.target(config, None)?;
    let selector = route.selector();
    validate_index_values(
        partition,
        "CTF split partition must not be empty",
        "CTF split partition entries must be non-zero",
        "CTF split partition entries must be unique",
        "CTF split partition exceeds the supported length limit",
    )?;
    if amount.base_units().is_zero() {
        return Err(RelayerError::Other(
            "CTF split amount must be non-zero".to_string(),
        ));
    }
    if amount.is_unlimited() {
        return Err(RelayerError::Other(
            "CTF split amount must not be unlimited".to_string(),
        ));
    }

    build_call(
        selector,
        target,
        vec![
            Token::Address(config.pusd().address()),
            fixed_bytes(H256::zero()),
            fixed_bytes(condition_id),
            uint_array(partition),
            Token::Uint(amount.base_units()),
        ],
    )
}

/// Build a verified `ConditionalTokens.mergePositions` call.
pub fn build_merge_positions_call(
    config: &DepositWalletCalldataConfig,
    condition_id: H256,
    partition: &[U256],
    amount: PusdAmount,
) -> Result<DepositWalletCall> {
    let route = CtfRoute::ConditionalTokensMerge;
    let target = route.target(config, None)?;
    let selector = route.selector();
    validate_index_values(
        partition,
        "CTF merge partition must not be empty",
        "CTF merge partition entries must be non-zero",
        "CTF merge partition entries must be unique",
        "CTF merge partition exceeds the supported length limit",
    )?;
    if amount.base_units().is_zero() {
        return Err(RelayerError::Other(
            "CTF merge amount must be non-zero".to_string(),
        ));
    }
    if amount.is_unlimited() {
        return Err(RelayerError::Other(
            "CTF merge amount must not be unlimited".to_string(),
        ));
    }

    build_call(
        selector,
        target,
        vec![
            Token::Address(config.pusd().address()),
            fixed_bytes(H256::zero()),
            fixed_bytes(condition_id),
            uint_array(partition),
            Token::Uint(amount.base_units()),
        ],
    )
}

/// Build a verified full-balance `ConditionalTokens.redeemPositions` call.
pub fn build_redeem_positions_call(
    config: &DepositWalletCalldataConfig,
    condition_id: H256,
    index_sets: &[U256],
) -> Result<DepositWalletCall> {
    let route = CtfRoute::ConditionalTokensRedeem;
    let target = route.target(config, None)?;
    let selector = route.selector();
    validate_index_values(
        index_sets,
        "CTF redeem index sets must not be empty",
        "CTF redeem index set entries must be non-zero",
        "CTF redeem index set entries must be unique",
        "CTF redeem index sets exceed the supported length limit",
    )?;

    build_call(
        selector,
        target,
        vec![
            Token::Address(config.pusd().address()),
            fixed_bytes(H256::zero()),
            fixed_bytes(condition_id),
            uint_array(index_sets),
        ],
    )
}

/// Build a verified `NegRiskAdapter.redeemPositions` call.
pub fn build_neg_risk_redeem_positions_call(
    config: &DepositWalletCalldataConfig,
    adapter: Address,
    condition_id: H256,
    amounts: &[CtfPositionAmount],
) -> Result<DepositWalletCall> {
    let route = CtfRoute::NegRiskAdapterRedeem;
    let target = route.target(config, Some(adapter))?;
    let selector = route.selector();
    if amounts.is_empty() {
        return Err(RelayerError::Other(
            "NegRisk redeem amounts must not be empty".to_string(),
        ));
    }
    if amounts.len() > MAX_CTF_ARRAY_LENGTH {
        return Err(RelayerError::Other(
            "NegRisk redeem amounts exceed the supported length limit".to_string(),
        ));
    }
    if amounts.iter().any(|amount| amount.base_units().is_zero()) {
        return Err(RelayerError::Other(
            "NegRisk redeem amounts must be non-zero".to_string(),
        ));
    }

    build_call(
        selector,
        target,
        vec![
            fixed_bytes(condition_id),
            Token::Array(
                amounts
                    .iter()
                    .map(|amount| Token::Uint(amount.base_units()))
                    .collect(),
            ),
        ],
    )
}

fn validate_index_values(
    values: &[U256],
    empty_error: &str,
    zero_error: &str,
    duplicate_error: &str,
    length_error: &str,
) -> Result<()> {
    if values.is_empty() {
        return Err(RelayerError::Other(empty_error.to_string()));
    }
    if values.iter().any(U256::is_zero) {
        return Err(RelayerError::Other(zero_error.to_string()));
    }
    for (index, value) in values.iter().enumerate() {
        if values[index + 1..].contains(value) {
            return Err(RelayerError::Other(duplicate_error.to_string()));
        }
    }
    if values.len() > MAX_CTF_ARRAY_LENGTH {
        return Err(RelayerError::Other(length_error.to_string()));
    }
    Ok(())
}

fn build_call(
    selector: [u8; 4],
    target: Address,
    tokens: Vec<Token>,
) -> Result<DepositWalletCall> {
    let mut data = Vec::from(selector);
    data.extend_from_slice(&encode(&tokens));

    Ok(DepositWalletCall {
        target,
        value: U256::zero(),
        data: Bytes::from(data),
    })
}

fn fixed_bytes(value: H256) -> Token {
    Token::FixedBytes(value.as_bytes().to_vec())
}

fn uint_array(values: &[U256]) -> Token {
    Token::Array(values.iter().copied().map(Token::Uint).collect())
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use ethers::utils::to_checksum;
    use serde_json::{json, Value};

    use super::super::{
        polygon_calldata_config, CalldataConfigInput, POLYGON_NEG_RISK_ADAPTER,
        POLYGON_PUSD,
    };
    use super::*;

    const CTF_SPLIT_FIXTURE: &str = include_str!(
        "../../../tests/fixtures/deposit_wallet/calldata_ctf_split_position_call.json"
    );
    const CTF_MERGE_FIXTURE: &str = include_str!(
        "../../../tests/fixtures/deposit_wallet/calldata_ctf_merge_positions_call.json"
    );
    const CTF_REDEEM_FIXTURE: &str = include_str!(
        "../../../tests/fixtures/deposit_wallet/calldata_ctf_redeem_positions_call.json"
    );
    const NEG_RISK_REDEEM_FIXTURE: &str = include_str!(
        "../../../tests/fixtures/deposit_wallet/calldata_neg_risk_redeem_positions_call.json"
    );
    const FIXTURE_CONDITION_ID: &str =
        "0x1111111111111111111111111111111111111111111111111111111111111111";
    const ALTERNATE_CONDITION_ID: &str =
        "0x0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20";
    const ZERO_COLLECTION_ID: &str =
        "0x0000000000000000000000000000000000000000000000000000000000000000";

    #[test]
    fn ctf_split_position_call_matches_fixture() {
        let config = polygon_calldata_config().expect("Polygon config should validate");
        let condition_id = h256(FIXTURE_CONDITION_ID);
        let partition = [U256::one(), U256::from(2u64)];
        let amount = PusdAmount::from_whole_pusd(1).expect("one pUSD should validate");
        let call = build_split_position_call(&config, condition_id, &partition, amount)
            .expect("fixture split should build");
        let expected = fixture(CTF_SPLIT_FIXTURE);

        assert_call_fields_match_fixture(&call, &expected);
        assert_common_fixture_metadata(
            &expected,
            CtfRoute::ConditionalTokensSplit,
            "0x72ce4275",
            condition_id,
        );
        assert_eq!(expected["partition"], json!(["1", "2"]));
        assert_eq!(expected["amountBaseUnits"], amount.base_units().to_string());
        assert_eq!(expected["amountDecimals"], amount.decimals());
        assert_eq!(expected["collateral"], to_checksum(&config.pusd().address(), None));
        assert_eq!(expected["target"], to_checksum(&config.ctf().address(), None));
        assert!(expected["source"]
            .as_str()
            .is_some_and(|source| source.contains("6-decimal base units")));
    }

    #[test]
    fn ctf_merge_positions_call_matches_fixture() {
        let config = polygon_calldata_config().expect("Polygon config should validate");
        let condition_id = h256(FIXTURE_CONDITION_ID);
        let partition = [U256::one(), U256::from(2u64)];
        let amount = PusdAmount::from_whole_pusd(1).expect("one pUSD should validate");
        let call = build_merge_positions_call(&config, condition_id, &partition, amount)
            .expect("fixture merge should build");
        let expected = fixture(CTF_MERGE_FIXTURE);

        assert_call_fields_match_fixture(&call, &expected);
        assert_common_fixture_metadata(
            &expected,
            CtfRoute::ConditionalTokensMerge,
            "0x9e7212ad",
            condition_id,
        );
        assert_eq!(expected["partition"], json!(["1", "2"]));
        assert_eq!(expected["amountBaseUnits"], amount.base_units().to_string());
        assert_eq!(expected["amountDecimals"], amount.decimals());
        assert_eq!(expected["collateral"], to_checksum(&config.pusd().address(), None));
        assert_eq!(expected["target"], to_checksum(&config.ctf().address(), None));
        assert!(expected["source"]
            .as_str()
            .is_some_and(|source| source.contains("6-decimal base units")));
    }

    #[test]
    fn ctf_redeem_positions_call_matches_fixture() {
        let config = polygon_calldata_config().expect("Polygon config should validate");
        let condition_id = h256(FIXTURE_CONDITION_ID);
        let index_sets = [U256::one(), U256::from(2u64)];
        let call = build_redeem_positions_call(&config, condition_id, &index_sets)
            .expect("fixture CTF redeem should build");
        let expected = fixture(CTF_REDEEM_FIXTURE);

        assert_call_fields_match_fixture(&call, &expected);
        assert_common_fixture_metadata(
            &expected,
            CtfRoute::ConditionalTokensRedeem,
            "0x01b7037c",
            condition_id,
        );
        assert_eq!(expected["indexSets"], json!(["1", "2"]));
        assert_eq!(expected["collateral"], to_checksum(&config.pusd().address(), None));
        assert_eq!(expected["target"], to_checksum(&config.ctf().address(), None));
        assert!(expected.get("amountBaseUnits").is_none());
        assert!(expected["source"]
            .as_str()
            .is_some_and(|source| source.contains("no amount argument")));
    }

    #[test]
    fn neg_risk_redeem_positions_call_matches_fixture() {
        let config = polygon_calldata_config().expect("Polygon config should validate");
        let condition_id = h256(FIXTURE_CONDITION_ID);
        let adapter = address(POLYGON_NEG_RISK_ADAPTER);
        let amounts = [position_amount(1_000_000), position_amount(2_000_000)];
        let call = build_neg_risk_redeem_positions_call(
            &config,
            adapter,
            condition_id,
            &amounts,
        )
        .expect("fixture NegRisk redeem should build");
        let expected = fixture(NEG_RISK_REDEEM_FIXTURE);

        assert_call_fields_match_fixture(&call, &expected);
        assert_common_fixture_metadata(
            &expected,
            CtfRoute::NegRiskAdapterRedeem,
            "0xdbeccb23",
            condition_id,
        );
        assert_eq!(expected["amounts"], json!(["1000000", "2000000"]));
        assert_eq!(expected["adapter"], to_checksum(&adapter, None));
        assert_eq!(expected["target"], to_checksum(&adapter, None));
        assert_eq!(expected["collateral"], POLYGON_PUSD);
        assert!(expected["source"]
            .as_str()
            .is_some_and(|source| source.contains("1:1 collateral ratio")));
    }

    #[test]
    fn ctf_calls_encode_supplied_condition_partition_and_amount() {
        let config = polygon_calldata_config().expect("Polygon config should validate");
        let condition_id = h256(ALTERNATE_CONDITION_ID);

        let split = build_split_position_call(
            &config,
            condition_id,
            &[U256::one(), U256::from(2u64), U256::from(4u64)],
            PusdAmount::from_base_units(U256::from(7u64)).expect("seven should validate"),
        )
        .expect("alternate split should build");
        assert_eq!(&split.data[0..4], &CtfRoute::ConditionalTokensSplit.selector());
        assert_address_word(&split.data, 4, config.pusd().address());
        assert_eq!(&split.data[36..68], &[0u8; 32]);
        assert_eq!(&split.data[68..100], condition_id.as_bytes());
        assert_u256_word(&split.data, 100, U256::from(0xa0u64));
        assert_u256_word(&split.data, 132, U256::from(7u64));
        assert_u256_word(&split.data, 164, U256::from(3u64));
        assert_u256_word(&split.data, 196, U256::one());
        assert_u256_word(&split.data, 228, U256::from(2u64));
        assert_u256_word(&split.data, 260, U256::from(4u64));
        assert_ne!(
            serde_json::to_value(&split).expect("split should serialize")["data"],
            fixture(CTF_SPLIT_FIXTURE)["data"]
        );

        let merge = build_merge_positions_call(
            &config,
            condition_id,
            &[U256::from(3u64), U256::from(5u64)],
            PusdAmount::from_base_units(U256::from(9u64)).expect("nine should validate"),
        )
        .expect("alternate merge should build");
        assert_eq!(&merge.data[0..4], &CtfRoute::ConditionalTokensMerge.selector());
        assert_address_word(&merge.data, 4, config.pusd().address());
        assert_eq!(&merge.data[36..68], &[0u8; 32]);
        assert_eq!(&merge.data[68..100], condition_id.as_bytes());
        assert_u256_word(&merge.data, 100, U256::from(0xa0u64));
        assert_u256_word(&merge.data, 132, U256::from(9u64));
        assert_u256_word(&merge.data, 164, U256::from(2u64));
        assert_u256_word(&merge.data, 196, U256::from(3u64));
        assert_u256_word(&merge.data, 228, U256::from(5u64));
        assert_ne!(
            serde_json::to_value(&merge).expect("merge should serialize")["data"],
            fixture(CTF_MERGE_FIXTURE)["data"]
        );
    }

    #[test]
    fn ctf_redeem_and_neg_risk_encode_supplied_index_sets_and_amounts() {
        let config = polygon_calldata_config().expect("Polygon config should validate");
        let condition_id = h256(ALTERNATE_CONDITION_ID);

        let redeem = build_redeem_positions_call(
            &config,
            condition_id,
            &[U256::from(2u64), U256::from(4u64), U256::from(8u64)],
        )
        .expect("alternate CTF redeem should build");
        assert_eq!(&redeem.data[0..4], &CtfRoute::ConditionalTokensRedeem.selector());
        assert_address_word(&redeem.data, 4, config.pusd().address());
        assert_eq!(&redeem.data[36..68], &[0u8; 32]);
        assert_eq!(&redeem.data[68..100], condition_id.as_bytes());
        assert_u256_word(&redeem.data, 100, U256::from(0x80u64));
        assert_u256_word(&redeem.data, 132, U256::from(3u64));
        assert_u256_word(&redeem.data, 164, U256::from(2u64));
        assert_u256_word(&redeem.data, 196, U256::from(4u64));
        assert_u256_word(&redeem.data, 228, U256::from(8u64));
        assert_ne!(
            serde_json::to_value(&redeem).expect("redeem should serialize")["data"],
            fixture(CTF_REDEEM_FIXTURE)["data"]
        );

        let adapter = address(POLYGON_NEG_RISK_ADAPTER);
        let neg_risk = build_neg_risk_redeem_positions_call(
            &config,
            adapter,
            condition_id,
            &[
                position_amount(11),
                position_amount(13),
                position_amount(11),
            ],
        )
        .expect("alternate NegRisk redeem should build");
        assert_eq!(
            &neg_risk.data[0..4],
            &CtfRoute::NegRiskAdapterRedeem.selector()
        );
        assert_eq!(&neg_risk.data[4..36], condition_id.as_bytes());
        assert_u256_word(&neg_risk.data, 36, U256::from(0x40u64));
        assert_u256_word(&neg_risk.data, 68, U256::from(3u64));
        assert_u256_word(&neg_risk.data, 100, U256::from(11u64));
        assert_u256_word(&neg_risk.data, 132, U256::from(13u64));
        assert_u256_word(&neg_risk.data, 164, U256::from(11u64));
        assert_ne!(
            serde_json::to_value(&neg_risk).expect("NegRisk redeem should serialize")["data"],
            fixture(NEG_RISK_REDEEM_FIXTURE)["data"]
        );
    }

    #[test]
    fn ctf_builders_reject_invalid_partitions_and_index_sets() {
        let config = polygon_calldata_config().expect("Polygon config should validate");
        let condition_id = h256(FIXTURE_CONDITION_ID);
        let amount = PusdAmount::from_whole_pusd(1).expect("one pUSD should validate");
        let with_zero = [U256::one(), U256::zero()];
        let duplicate = [U256::one(), U256::one()];
        let at_limit = (1u64..=64).map(U256::from).collect::<Vec<_>>();
        let too_long = (1u64..=65).map(U256::from).collect::<Vec<_>>();

        build_split_position_call(&config, condition_id, &at_limit, amount)
            .expect("64 split partition entries should be supported");
        build_merge_positions_call(&config, condition_id, &at_limit, amount)
            .expect("64 merge partition entries should be supported");
        build_redeem_positions_call(&config, condition_id, &at_limit)
            .expect("64 redeem index sets should be supported");

        assert_error(
            build_split_position_call(&config, condition_id, &[], amount),
            "CTF split partition must not be empty",
        );
        assert_error(
            build_split_position_call(&config, condition_id, &with_zero, amount),
            "CTF split partition entries must be non-zero",
        );
        assert_error(
            build_split_position_call(&config, condition_id, &duplicate, amount),
            "CTF split partition entries must be unique",
        );
        assert_error(
            build_split_position_call(&config, condition_id, &too_long, amount),
            "CTF split partition exceeds the supported length limit",
        );

        assert_error(
            build_merge_positions_call(&config, condition_id, &[], amount),
            "CTF merge partition must not be empty",
        );
        assert_error(
            build_merge_positions_call(&config, condition_id, &with_zero, amount),
            "CTF merge partition entries must be non-zero",
        );
        assert_error(
            build_merge_positions_call(&config, condition_id, &duplicate, amount),
            "CTF merge partition entries must be unique",
        );
        assert_error(
            build_merge_positions_call(&config, condition_id, &too_long, amount),
            "CTF merge partition exceeds the supported length limit",
        );

        assert_error(
            build_redeem_positions_call(&config, condition_id, &[]),
            "CTF redeem index sets must not be empty",
        );
        assert_error(
            build_redeem_positions_call(&config, condition_id, &with_zero),
            "CTF redeem index set entries must be non-zero",
        );
        assert_error(
            build_redeem_positions_call(&config, condition_id, &duplicate),
            "CTF redeem index set entries must be unique",
        );
        assert_error(
            build_redeem_positions_call(&config, condition_id, &too_long),
            "CTF redeem index sets exceed the supported length limit",
        );

        let adapter = address(POLYGON_NEG_RISK_ADAPTER);
        build_neg_risk_redeem_positions_call(
            &config,
            adapter,
            condition_id,
            &vec![position_amount(1); 64],
        )
        .expect("64 NegRisk amounts should be supported");
        assert_error(
            build_neg_risk_redeem_positions_call(&config, adapter, condition_id, &[]),
            "NegRisk redeem amounts must not be empty",
        );
        assert_error(
            build_neg_risk_redeem_positions_call(
                &config,
                adapter,
                condition_id,
                &vec![position_amount(1); 65],
            ),
            "NegRisk redeem amounts exceed the supported length limit",
        );
    }

    #[test]
    fn neg_risk_redeem_rejects_adapter_outside_verified_allowlist() {
        let config = polygon_calldata_config().expect("Polygon config should validate");
        let condition_id = h256(FIXTURE_CONDITION_ID);
        let amount = [position_amount(1_000_000)];

        assert_error(
            build_neg_risk_redeem_positions_call(
                &config,
                address("0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef"),
                condition_id,
                &amount,
            ),
            "CTF adapter is not in the verified allowlist",
        );
        assert_error(
            build_neg_risk_redeem_positions_call(
                &config,
                Address::zero(),
                condition_id,
                &amount,
            ),
            "CTF adapter must not be zero",
        );
    }

    #[test]
    fn neg_risk_redeem_respects_narrowed_adapter_allowlist() {
        let canonical = polygon_calldata_config().expect("Polygon config should validate");
        let mut input = config_input(&canonical);
        input.adapter_allowlist.clear();
        let narrowed = DepositWalletCalldataConfig::try_new(input)
            .expect("empty verified adapter subset should validate");

        assert_error(
            build_neg_risk_redeem_positions_call(
                &narrowed,
                address(POLYGON_NEG_RISK_ADAPTER),
                h256(FIXTURE_CONDITION_ID),
                &[position_amount(1_000_000)],
            ),
            "CTF adapter is not in the verified allowlist",
        );
    }

    #[test]
    fn ctf_route_target_defines_every_adapter_combination() {
        let config = polygon_calldata_config().expect("Polygon config should validate");
        let adapter = address(POLYGON_NEG_RISK_ADAPTER);

        for route in [
            CtfRoute::ConditionalTokensSplit,
            CtfRoute::ConditionalTokensMerge,
            CtfRoute::ConditionalTokensRedeem,
        ] {
            assert_eq!(
                route.target(&config, None).expect("CTF target should resolve"),
                config.ctf().address()
            );
            assert_eq!(
                route
                    .target(&config, Some(adapter))
                    .expect_err("CTF route must reject an adapter")
                    .to_string(),
                "ConditionalTokens routes must not be given an adapter address"
            );
        }

        assert_eq!(
            CtfRoute::NegRiskAdapterRedeem
                .target(&config, None)
                .expect_err("NegRisk route requires an adapter")
                .to_string(),
            "NegRisk redeem requires an adapter address"
        );
        assert_eq!(
            CtfRoute::NegRiskAdapterRedeem
                .target(&config, Some(adapter))
                .expect("verified adapter should resolve"),
            adapter
        );
        assert_eq!(
            CtfRoute::NegRiskAdapterRedeem
                .target(&config, Some(Address::zero()))
                .expect_err("zero adapter must fail")
                .to_string(),
            "CTF adapter must not be zero"
        );
        assert_eq!(
            CtfRoute::NegRiskAdapterRedeem
                .target(
                    &config,
                    Some(address("0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef")),
                )
                .expect_err("unverified adapter must fail")
                .to_string(),
            "CTF adapter is not in the verified allowlist"
        );
    }

    #[test]
    fn ctf_route_selector_matches_verified_wire_truth() {
        for (route, selector, fixture_raw) in [
            (
                CtfRoute::ConditionalTokensSplit,
                [0x72, 0xce, 0x42, 0x75],
                CTF_SPLIT_FIXTURE,
            ),
            (
                CtfRoute::ConditionalTokensMerge,
                [0x9e, 0x72, 0x12, 0xad],
                CTF_MERGE_FIXTURE,
            ),
            (
                CtfRoute::ConditionalTokensRedeem,
                [0x01, 0xb7, 0x03, 0x7c],
                CTF_REDEEM_FIXTURE,
            ),
            (
                CtfRoute::NegRiskAdapterRedeem,
                [0xdb, 0xec, 0xcb, 0x23],
                NEG_RISK_REDEEM_FIXTURE,
            ),
        ] {
            assert_eq!(route.selector(), selector);
            assert_eq!(
                fixture(fixture_raw)["selector"],
                format!("0x{}", hex::encode(selector))
            );
        }
    }

    #[test]
    fn ctf_builders_reject_zero_amount_from_unchecked_construction() {
        let config = polygon_calldata_config().expect("Polygon config should validate");
        let condition_id = h256(FIXTURE_CONDITION_ID);
        let partition = [U256::one(), U256::from(2u64)];
        let zero_pusd = PusdAmount::unchecked_for_test(U256::zero());
        let zero_position = CtfPositionAmount::unchecked_for_test(U256::zero());

        assert_error(
            build_split_position_call(&config, condition_id, &partition, zero_pusd),
            "CTF split amount must be non-zero",
        );
        assert_error(
            build_merge_positions_call(&config, condition_id, &partition, zero_pusd),
            "CTF merge amount must be non-zero",
        );
        assert_error(
            build_neg_risk_redeem_positions_call(
                &config,
                address(POLYGON_NEG_RISK_ADAPTER),
                condition_id,
                &[zero_position],
            ),
            "NegRisk redeem amounts must be non-zero",
        );
    }

    #[test]
    fn ctf_split_and_merge_reject_unlimited_amount() {
        let config = polygon_calldata_config().expect("Polygon config should validate");
        let condition_id = h256(FIXTURE_CONDITION_ID);
        let partition = [U256::one(), U256::from(2u64)];

        assert_error(
            build_split_position_call(
                &config,
                condition_id,
                &partition,
                PusdAmount::unlimited(),
            ),
            "CTF split amount must not be unlimited",
        );
        assert_error(
            build_merge_positions_call(
                &config,
                condition_id,
                &partition,
                PusdAmount::unlimited(),
            ),
            "CTF merge amount must not be unlimited",
        );
    }

    #[test]
    fn neg_risk_redeem_accepts_duplicate_amounts() {
        let config = polygon_calldata_config().expect("Polygon config should validate");
        let duplicate = position_amount(1_000_000);
        let call = build_neg_risk_redeem_positions_call(
            &config,
            address(POLYGON_NEG_RISK_ADAPTER),
            h256(FIXTURE_CONDITION_ID),
            &[duplicate, duplicate],
        )
        .expect("duplicate position quantities are valid");

        assert_u256_word(&call.data, 68, U256::from(2u64));
        assert_u256_word(&call.data, 100, U256::from(1_000_000u64));
        assert_u256_word(&call.data, 132, U256::from(1_000_000u64));
    }

    fn assert_common_fixture_metadata(
        expected: &Value,
        route: CtfRoute,
        selector: &str,
        condition_id: H256,
    ) {
        assert_eq!(
            expected["route"],
            serde_json::to_value(route).expect("route should serialize")
        );
        assert_eq!(expected["selector"], selector);
        assert_eq!(expected["conditionId"], format!("{condition_id:#x}"));
        assert_eq!(expected["parentCollectionId"], ZERO_COLLECTION_ID);
        assert_eq!(expected["value"], "0");
        assert!(expected["source"].as_str().is_some_and(|source| {
            source.contains("3ae1aae5e9ded38f984464c9fc0f307f8a9f41fb")
                && source.contains("keccak256")
        }));
    }

    fn assert_call_fields_match_fixture(call: &DepositWalletCall, expected: &Value) {
        let actual = serde_json::to_value(call).expect("call should serialize");
        assert_eq!(
            actual.as_object().expect("call should serialize as object").len(),
            3
        );
        for field in ["target", "value", "data"] {
            assert_eq!(actual[field], expected[field], "fixture field {field}");
        }
    }

    fn assert_address_word(data: &[u8], start: usize, address: Address) {
        assert_eq!(&data[start..start + 12], &[0u8; 12]);
        assert_eq!(&data[start + 12..start + 32], address.as_bytes());
    }

    fn assert_u256_word(data: &[u8], start: usize, value: U256) {
        let mut expected = [0u8; 32];
        value.to_big_endian(&mut expected);
        assert_eq!(&data[start..start + 32], &expected);
    }

    fn assert_error(result: Result<DepositWalletCall>, expected: &str) {
        assert_eq!(
            result.expect_err("invalid CTF call must fail").to_string(),
            expected
        );
    }

    fn fixture(raw: &str) -> Value {
        serde_json::from_str(raw).expect("fixture should be valid JSON")
    }

    fn address(raw: &str) -> Address {
        Address::from_str(raw).expect("embedded test address should parse")
    }

    fn h256(raw: &str) -> H256 {
        H256::from_str(raw).expect("embedded condition id should parse")
    }

    fn position_amount(base_units: u64) -> CtfPositionAmount {
        CtfPositionAmount::from_base_units(U256::from(base_units))
            .expect("test position amount should validate")
    }

    fn config_input(config: &DepositWalletCalldataConfig) -> CalldataConfigInput {
        CalldataConfigInput {
            chain_id: config.chain_id(),
            pusd: config.pusd().clone(),
            ctf: config.ctf().clone(),
            pusd_decimals: config.pusd_decimals(),
            pusd_decimals_source: config.pusd_decimals_source().clone(),
            pusd_spender_allowlist: config.pusd_spender_allowlist().to_vec(),
            ctf_operator_allowlist: config.ctf_operator_allowlist().to_vec(),
            adapter_allowlist: config.adapter_allowlist().to_vec(),
        }
    }

}
