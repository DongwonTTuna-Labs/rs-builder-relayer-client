use ethers::abi::{encode, Token};
use ethers::types::{Address, Bytes, U256};

use crate::deposit_wallet::types::DepositWalletCall;
use crate::error::{RelayerError, Result};

use super::{DepositWalletCalldataConfig, PusdAmount};

const PUSD_APPROVE_SELECTOR: [u8; 4] = [0x09, 0x5e, 0xa7, 0xb3];
const CTF_SET_APPROVAL_FOR_ALL_SELECTOR: [u8; 4] = [0xa2, 0x2c, 0xb4, 0x65];

/// Build an ERC-20 `approve(address,uint256)` call for the configured pUSD.
pub fn build_pusd_approval_call(
    config: &DepositWalletCalldataConfig,
    spender: Address,
    amount: PusdAmount,
) -> Result<DepositWalletCall> {
    if spender == Address::zero() {
        return Err(RelayerError::Other(
            "pUSD approval spender must not be zero".to_string(),
        ));
    }
    if !config.is_allowed_pusd_spender(spender) {
        return Err(RelayerError::Other(
            "pUSD approval spender is not in the verified allowlist".to_string(),
        ));
    }
    if amount.base_units().is_zero() {
        return Err(RelayerError::Other(
            "pUSD approval amount must be non-zero".to_string(),
        ));
    }

    let mut calldata_vec = Vec::from(PUSD_APPROVE_SELECTOR);
    calldata_vec.extend_from_slice(&encode(&[
        Token::Address(spender),
        Token::Uint(amount.base_units()),
    ]));

    Ok(DepositWalletCall {
        target: config.pusd().address(),
        value: U256::zero(),
        data: Bytes::from(calldata_vec),
    })
}

/// Build an ERC-1155 `setApprovalForAll(address,bool)` call for configured CTF.
///
/// `approved = false` is intentionally supported for approval revocation, with
/// the same verified operator allowlist check as the approval path.
pub fn build_ctf_approval_for_all_call(
    config: &DepositWalletCalldataConfig,
    operator: Address,
    approved: bool,
) -> Result<DepositWalletCall> {
    if operator == Address::zero() {
        return Err(RelayerError::Other(
            "CTF approval operator must not be zero".to_string(),
        ));
    }
    if !config.is_allowed_ctf_operator(operator) {
        return Err(RelayerError::Other(
            "CTF approval operator is not in the verified allowlist".to_string(),
        ));
    }

    let mut calldata_vec = Vec::from(CTF_SET_APPROVAL_FOR_ALL_SELECTOR);
    calldata_vec.extend_from_slice(&encode(&[
        Token::Address(operator),
        Token::Bool(approved),
    ]));

    Ok(DepositWalletCall {
        target: config.ctf().address(),
        value: U256::zero(),
        data: Bytes::from(calldata_vec),
    })
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use ethers::utils::to_checksum;
    use serde_json::Value;

    use super::super::{
        polygon_calldata_config, CalldataConfigInput, POLYGON_CTF,
        POLYGON_NEG_RISK_EXCHANGE, POLYGON_STANDARD_EXCHANGE,
    };
    use super::*;

    const RECORDED_WALLET_SUBMIT_BODY: &str = include_str!(
        "../../../tests/fixtures/deposit_wallet/wallet_submit_body.json"
    );
    const PUSD_APPROVAL_FIXTURE: &str = include_str!(
        "../../../tests/fixtures/deposit_wallet/calldata_pusd_approval_call.json"
    );
    const CTF_APPROVAL_FIXTURE: &str = include_str!(
        "../../../tests/fixtures/deposit_wallet/calldata_ctf_approval_for_all_call.json"
    );

    #[test]
    fn pusd_approval_call_matches_recorded_batch_call() {
        let config = polygon_calldata_config().expect("Polygon config should validate");
        let call = build_pusd_approval_call(
            &config,
            address(POLYGON_CTF),
            PusdAmount::unlimited(),
        )
        .expect("recorded pUSD approval should build");
        let recorded = fixture(RECORDED_WALLET_SUBMIT_BODY);
        let recorded_call = &recorded["depositWalletParams"]["calls"][0];
        let actual = serde_json::to_value(&call).expect("call should serialize");

        assert_eq!(actual["target"], recorded_call["target"]);
        assert_eq!(actual["value"], recorded_call["value"]);
        assert_eq!(actual["data"], recorded_call["data"]);
    }

    #[test]
    fn approval_calls_encode_supplied_spender_operator_and_amount() {
        let config = polygon_calldata_config().expect("Polygon config should validate");
        let recorded = build_pusd_approval_call(
            &config,
            address(POLYGON_CTF),
            PusdAmount::unlimited(),
        )
        .expect("recorded pUSD approval should build");

        let spender = address(POLYGON_STANDARD_EXCHANGE);
        let finite = build_pusd_approval_call(
            &config,
            spender,
            PusdAmount::from_whole_pusd(1).expect("finite amount should validate"),
        )
        .expect("finite pUSD approval should build");
        let finite_data = finite.data.as_ref();
        assert_eq!(&finite_data[0..4], &PUSD_APPROVE_SELECTOR);
        assert_eq!(&finite_data[4..16], &[0u8; 12]);
        assert_eq!(&finite_data[16..36], spender.as_bytes());
        assert_eq!(&finite_data[36..65], &[0u8; 29]);
        assert_eq!(&finite_data[65..68], &[0x0f, 0x42, 0x40]);
        assert_ne!(finite.data, recorded.data);

        let operator = address(POLYGON_NEG_RISK_EXCHANGE);
        let ctf = build_ctf_approval_for_all_call(&config, operator, true)
            .expect("CTF approval should build");
        let ctf_data = ctf.data.as_ref();
        assert_eq!(&ctf_data[0..4], &CTF_SET_APPROVAL_FOR_ALL_SELECTOR);
        assert_eq!(&ctf_data[4..16], &[0u8; 12]);
        assert_eq!(&ctf_data[16..36], operator.as_bytes());
        assert_eq!(&ctf_data[36..67], &[0u8; 31]);
        assert_eq!(ctf_data[67], 1);
    }

    #[test]
    fn pusd_amount_round_trips_finite_base_units_into_calldata() {
        let config = polygon_calldata_config().expect("Polygon config should validate");
        let amount = PusdAmount::from_base_units(U256::from(123_456u64))
            .expect("finite base units should validate");
        assert_eq!(amount.base_units(), U256::from(123_456u64));

        let call = build_pusd_approval_call(&config, address(POLYGON_CTF), amount)
            .expect("finite pUSD approval should build");
        let mut expected_word = [0u8; 32];
        amount.base_units().to_big_endian(&mut expected_word);
        assert_eq!(&call.data[36..68], &expected_word);
    }

    #[test]
    fn pusd_approval_call_matches_fixture() {
        let config = polygon_calldata_config().expect("Polygon config should validate");
        let spender = address(POLYGON_CTF);
        let amount = PusdAmount::unlimited();
        let call = build_pusd_approval_call(&config, spender, amount)
            .expect("fixture pUSD approval should build");
        let expected = fixture(PUSD_APPROVAL_FIXTURE);

        assert_call_fields_match_fixture(&call, &expected);
        assert_eq!(expected["selector"], "0x095ea7b3");
        assert_eq!(expected["spender"], to_checksum(&spender, None));
        assert_eq!(expected["amountBaseUnits"], amount.base_units().to_string());
        assert_eq!(expected["amountDecimals"], amount.decimals());
        assert_eq!(expected["amountUnit"], "pUSD");
        assert_eq!(&call.data[0..4], &PUSD_APPROVE_SELECTOR);
        assert_eq!(&call.data[4..16], &[0u8; 12]);
        assert_eq!(&call.data[16..36], spender.as_bytes());
        let mut expected_amount = [0u8; 32];
        amount.base_units().to_big_endian(&mut expected_amount);
        assert_eq!(&call.data[36..68], &expected_amount);
        assert!(expected["source"]
            .as_str()
            .is_some_and(|source| !source.is_empty()));
    }

    #[test]
    fn ctf_approval_for_all_call_matches_fixture() {
        let config = polygon_calldata_config().expect("Polygon config should validate");
        let operator = address(POLYGON_STANDARD_EXCHANGE);
        let approved = true;
        let call = build_ctf_approval_for_all_call(&config, operator, approved)
            .expect("fixture CTF approval should build");
        let expected = fixture(CTF_APPROVAL_FIXTURE);

        assert_call_fields_match_fixture(&call, &expected);
        assert_eq!(expected["selector"], "0xa22cb465");
        assert_eq!(expected["operator"], to_checksum(&operator, None));
        assert_eq!(expected["approved"], approved);
        assert_eq!(&call.data[0..4], &CTF_SET_APPROVAL_FOR_ALL_SELECTOR);
        assert_eq!(&call.data[4..16], &[0u8; 12]);
        assert_eq!(&call.data[16..36], operator.as_bytes());
        assert_eq!(&call.data[36..67], &[0u8; 31]);
        assert_eq!(call.data[67], 1);
        assert!(expected["source"]
            .as_str()
            .is_some_and(|source| !source.is_empty()));
    }

    #[test]
    fn approval_builders_reject_addresses_outside_verified_allowlists() {
        let config = polygon_calldata_config().expect("Polygon config should validate");
        let attacker = address("0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef");

        assert_eq!(
            build_pusd_approval_call(&config, attacker, PusdAmount::unlimited())
                .expect_err("outsider pUSD spender must fail")
                .to_string(),
            "pUSD approval spender is not in the verified allowlist"
        );
        assert_eq!(
            build_ctf_approval_for_all_call(&config, attacker, true)
                .expect_err("outsider CTF operator must fail")
                .to_string(),
            "CTF approval operator is not in the verified allowlist"
        );
    }

    #[test]
    fn approval_builders_reject_zero_address_targets() {
        let config = polygon_calldata_config().expect("Polygon config should validate");

        assert_eq!(
            build_pusd_approval_call(&config, Address::zero(), PusdAmount::unlimited())
                .expect_err("zero pUSD spender must fail")
                .to_string(),
            "pUSD approval spender must not be zero"
        );
        assert_eq!(
            build_ctf_approval_for_all_call(&config, Address::zero(), true)
                .expect_err("zero CTF operator must fail")
                .to_string(),
            "CTF approval operator must not be zero"
        );
    }

    #[test]
    fn pusd_approval_respects_narrowed_config_allowlist() {
        let canonical = polygon_calldata_config().expect("Polygon config should validate");
        let mut input = config_input(&canonical);
        input.pusd_spender_allowlist = vec![canonical.pusd_spender_allowlist()[0].clone()];
        let narrowed = DepositWalletCalldataConfig::try_new(input)
            .expect("reviewed pUSD spender subset should validate");

        assert_eq!(
            build_pusd_approval_call(
                &narrowed,
                address(POLYGON_STANDARD_EXCHANGE),
                PusdAmount::unlimited(),
            )
            .expect_err("spender excluded by caller config must fail")
            .to_string(),
            "pUSD approval spender is not in the verified allowlist"
        );
    }

    #[test]
    fn ctf_approval_respects_narrowed_config_allowlist() {
        let canonical = polygon_calldata_config().expect("Polygon config should validate");
        let mut input = config_input(&canonical);
        input.ctf_operator_allowlist =
            vec![canonical.ctf_operator_allowlist()[0].clone()];
        let narrowed = DepositWalletCalldataConfig::try_new(input)
            .expect("reviewed CTF operator subset should validate");

        assert_eq!(
            build_ctf_approval_for_all_call(
                &narrowed,
                address(POLYGON_NEG_RISK_EXCHANGE),
                true,
            )
            .expect_err("operator excluded by caller config must fail")
            .to_string(),
            "CTF approval operator is not in the verified allowlist"
        );
    }

    #[test]
    fn ctf_approval_for_all_encodes_revocation() {
        let config = polygon_calldata_config().expect("Polygon config should validate");
        let call = build_ctf_approval_for_all_call(
            &config,
            address(POLYGON_STANDARD_EXCHANGE),
            false,
        )
        .expect("CTF revocation should build");

        assert_eq!(&call.data[0..4], &CTF_SET_APPROVAL_FOR_ALL_SELECTOR);
        assert_eq!(&call.data[36..68], &[0u8; 32]);
    }

    #[test]
    fn pusd_approval_call_rejects_zero_amount_from_unchecked_construction() {
        let config = polygon_calldata_config().expect("Polygon config should validate");
        let zero = PusdAmount::unchecked_for_test(U256::zero());

        assert_eq!(
            build_pusd_approval_call(&config, address(POLYGON_CTF), zero)
                .expect_err("builder must reject an invariant-bypassing zero amount")
                .to_string(),
            "pUSD approval amount must be non-zero"
        );
    }

    fn fixture(raw: &str) -> Value {
        serde_json::from_str::<Value>(raw).expect("fixture should be valid JSON")
    }

    fn address(raw: &str) -> Address {
        Address::from_str(raw).expect("embedded test address should parse")
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

}
