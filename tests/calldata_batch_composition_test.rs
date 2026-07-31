use ethers::types::{Address, Bytes, H256, U256};
use ethers::utils::to_checksum;
use polymarket_relayer::deposit_wallet::calldata::{
    POLYGON_CTF, POLYGON_NEG_RISK_ADAPTER, POLYGON_STANDARD_EXCHANGE,
};
use polymarket_relayer::{
    build_ctf_approval_for_all_call, build_merge_positions_call,
    build_neg_risk_redeem_positions_call, build_pusd_approval_call,
    build_redeem_positions_call, build_split_position_call, deposit_wallet_contract_config,
    derive_deposit_wallet_address, polygon_calldata_config, summarize_batch_calls,
    try_build_wallet_batch_request_with_signature, CalldataConfigInput, CtfPositionAmount,
    CtfRoute, DepositWalletCall, DepositWalletContractConfig, DepositWalletRequestContext,
    PusdAmount, RelayerError, Result,
};
use serde_json::Value;

const REDACTION_MARKER: &str = "full calldata and signatures are intentionally omitted";
const CONDITION_ID: &str =
    "0x1111111111111111111111111111111111111111111111111111111111111111";
const WALLET_BATCH_FIXTURE: &str = include_str!(
    "fixtures/deposit_wallet/wallet_batch_eip712.json"
);
const CALL_FIXTURES: [&str; 6] = [
    include_str!("fixtures/deposit_wallet/calldata_pusd_approval_call.json"),
    include_str!("fixtures/deposit_wallet/calldata_ctf_approval_for_all_call.json"),
    include_str!("fixtures/deposit_wallet/calldata_ctf_split_position_call.json"),
    include_str!("fixtures/deposit_wallet/calldata_ctf_merge_positions_call.json"),
    include_str!("fixtures/deposit_wallet/calldata_ctf_redeem_positions_call.json"),
    include_str!("fixtures/deposit_wallet/calldata_neg_risk_redeem_positions_call.json"),
];

#[test]
fn every_builder_output_composes_into_an_ordered_batch_summary() {
    let calls = all_builder_calls();
    let summary = summarize_batch_calls(&calls);
    let expected_selectors = [
        "0x095ea7b3".to_string(),
        "0xa22cb465".to_string(),
        selector_hex(CtfRoute::ConditionalTokensSplit),
        selector_hex(CtfRoute::ConditionalTokensMerge),
        selector_hex(CtfRoute::ConditionalTokensRedeem),
        selector_hex(CtfRoute::NegRiskAdapterRedeem),
    ];

    assert_eq!(calls.len(), 6);
    for (call, fixture_raw) in calls.iter().zip(CALL_FIXTURES) {
        assert_call_matches_fixture(call, fixture_raw);
    }
    assert_eq!(summary.call_count(), calls.len());
    assert_eq!(
        summary.total_calldata_bytes(),
        calls.iter().map(|call| call.data.len()).sum::<usize>()
    );
    assert_eq!(summary.calls().len(), calls.len());
    for (index, (call, call_summary)) in calls.iter().zip(summary.calls()).enumerate() {
        assert_eq!(call_summary.target(), redacted_address(call.target));
        assert_eq!(call_summary.value(), call.value.to_string());
        assert_eq!(
            call_summary.selector(),
            Some(expected_selectors[index].as_str())
        );
        assert_eq!(call_summary.data_len(), call.data.len());
    }
}

#[test]
fn batch_summary_omits_full_calldata_full_targets_and_signatures() {
    let calls = all_builder_calls();
    let summary = summarize_batch_calls(&calls);
    let json = serde_json::to_string(&summary).expect("summary should serialize");
    let debug = format!("{summary:?}");

    for (index, call) in calls.iter().enumerate() {
        let full_data = format!("0x{}", hex::encode(call.data.as_ref()));
        let full_data_without_prefix = full_data.trim_start_matches("0x");
        let uppercase_full_data = full_data.to_ascii_uppercase();
        let uppercase_payload = format!("0x{}", full_data_without_prefix.to_ascii_uppercase());
        let full_target = to_checksum(&call.target, None);
        let redacted_target = redacted_address(call.target);

        assert!(
            !json.contains(&full_data)
                && !debug.contains(&full_data)
                && !json.contains(full_data_without_prefix)
                && !debug.contains(full_data_without_prefix)
                && !json.contains(&uppercase_full_data)
                && !debug.contains(&uppercase_full_data)
                && !json.contains(&uppercase_payload)
                && !debug.contains(&uppercase_payload),
            "summary output leaked full calldata for call {index}"
        );
        assert!(
            !json.contains(&full_target) && !debug.contains(&full_target),
            "summary output leaked a full target for call {index}"
        );
        assert!(json.contains(&redacted_target));
        assert!(debug.contains(&redacted_target));
    }
    assert!(json.contains(REDACTION_MARKER));
    assert!(debug.contains(REDACTION_MARKER));
    assert_eq!(summary.redaction(), REDACTION_MARKER);
    let wallet_fixture = fixture(WALLET_BATCH_FIXTURE);
    for signature_key in ["ownerSignature", "nonOwnerSignature"] {
        let signature = wallet_fixture[signature_key]
            .as_str()
            .expect("fixture signature should be a string");
        assert!(!json.contains(signature));
        assert!(!debug.contains(signature));
    }
}

#[test]
fn wallet_batch_preflight_rejects_each_resource_limit_independently() {
    let fixture = fixture(WALLET_BATCH_FIXTURE);
    let config = deposit_wallet_contract_config(137).expect("Polygon config should exist");
    let ctx = fixture_context(&fixture);
    let nonce = fixture_u256(&fixture, "nonce");
    let deadline = fixture_u256(&fixture, "deadline");
    let signature = fixture_string(&fixture, "ownerSignature");
    let builder_call = all_builder_calls().remove(0);

    assert_signing_error_contains(
        try_build_wallet_batch_request_with_signature(
            ctx.clone(),
            config,
            nonce,
            deadline,
            vec![builder_call.clone(); 257],
            signature.clone(),
        ),
        "call count exceeds maximum",
    );

    let oversized_calldata_call = DepositWalletCall {
        target: builder_call.target,
        value: U256::zero(),
        data: Bytes::from(vec![0u8; 1024 * 1024 + 1]),
    };
    assert_signing_error_contains(
        try_build_wallet_batch_request_with_signature(
            ctx,
            config,
            nonce,
            deadline,
            vec![oversized_calldata_call],
            signature,
        ),
        "calldata bytes exceed maximum",
    );
}

#[test]
fn wallet_batch_preflight_rejects_wallet_chain_and_signer_mismatches() {
    let fixture = fixture(WALLET_BATCH_FIXTURE);
    let polygon_config = deposit_wallet_contract_config(137).expect("Polygon config should exist");
    let amoy_config = deposit_wallet_contract_config(80002).expect("Amoy config should exist");
    let ctx = fixture_context(&fixture);
    let nonce = fixture_u256(&fixture, "nonce");
    let deadline = fixture_u256(&fixture, "deadline");
    let signature = fixture_string(&fixture, "ownerSignature");
    let call = all_builder_calls().remove(0);

    let wrong_wallet_ctx = DepositWalletRequestContext {
        owner_address: ctx.owner_address,
        deposit_wallet_address: Address::from_low_u64_be(0xdead),
    };
    assert_signing_error_contains(
        try_build_wallet_batch_request_with_signature(
            wrong_wallet_ctx,
            polygon_config,
            nonce,
            deadline,
            vec![call.clone()],
            signature.clone(),
        ),
        "wallet does not match owner/config derived wallet",
    );

    assert_signing_error_contains(
        try_build_wallet_batch_request_with_signature(
            ctx.clone(),
            amoy_config,
            nonce,
            deadline,
            vec![call.clone()],
            signature.clone(),
        ),
        "wallet does not match owner/config derived wallet",
    );

    assert_signing_error_contains(
        try_build_wallet_batch_request_with_signature(
            ctx.clone(),
            polygon_config,
            nonce,
            deadline,
            vec![call.clone()],
            fixture_string(&fixture, "nonOwnerSignature"),
        ),
        "signer must match owner",
    );

    let unsupported_config = DepositWalletContractConfig {
        factory: Address::from_low_u64_be(1),
        implementation: Address::from_low_u64_be(2),
    };
    assert_signing_error_contains(
        try_build_wallet_batch_request_with_signature(
            ctx,
            unsupported_config,
            nonce,
            deadline,
            vec![call],
            signature,
        ),
        "contract config is not supported",
    );
}

#[test]
fn narrowed_calldata_config_blocks_batch_composition_before_signing() {
    let canonical = polygon_calldata_config().expect("Polygon config should validate");
    let ctf = canonical.ctf().address();
    let narrowed = polymarket_relayer::DepositWalletCalldataConfig::try_new(
        CalldataConfigInput {
            chain_id: canonical.chain_id(),
            pusd: canonical.pusd().clone(),
            ctf: canonical.ctf().clone(),
            pusd_decimals: canonical.pusd_decimals(),
            pusd_decimals_source: canonical.pusd_decimals_source().clone(),
            pusd_spender_allowlist: canonical
                .pusd_spender_allowlist()
                .iter()
                .filter(|entry| entry.address() != ctf)
                .cloned()
                .collect(),
            ctf_operator_allowlist: canonical.ctf_operator_allowlist().to_vec(),
            adapter_allowlist: canonical.adapter_allowlist().to_vec(),
        },
    )
    .expect("reviewed pUSD spender subset should validate");

    let batch = build_pusd_approval_call(&narrowed, ctf, PusdAmount::unlimited())
        .map(|call| vec![call]);
    assert_other_error_contains(
        batch,
        "pUSD approval spender is not in the verified allowlist",
    );
}

#[test]
fn fixture_builder_output_is_compatible_with_wallet_submit_body() {
    let fixture = fixture(WALLET_BATCH_FIXTURE);
    let calldata_config = polygon_calldata_config().expect("Polygon config should validate");
    let call = build_pusd_approval_call(
        &calldata_config,
        address(POLYGON_CTF),
        PusdAmount::unlimited(),
    )
    .expect("fixture approval call should build");
    assert_call_matches_fixture(&call, CALL_FIXTURES[0]);

    let request = try_build_wallet_batch_request_with_signature(
        fixture_context(&fixture),
        deposit_wallet_contract_config(137).expect("Polygon config should exist"),
        fixture_u256(&fixture, "nonce"),
        fixture_u256(&fixture, "deadline"),
        vec![call],
        fixture_string(&fixture, "ownerSignature"),
    )
    .expect("fixture-backed builder output should pass WALLET request preflight");
    let request_json = serde_json::to_value(request).expect("request should serialize");
    let actual_call = &request_json["depositWalletParams"]["calls"][0];
    let expected_call = &fixture["calls"][0];

    assert_eq!(
        actual_call
            .as_object()
            .expect("submit call should be an object")
            .len(),
        3
    );
    for field in ["target", "value", "data"] {
        assert!(
            actual_call[field] == expected_call[field],
            "submit call field {field} must match the reviewed fixture"
        );
    }
}

#[test]
fn summary_selector_boundary_omits_four_bytes_and_keeps_five_byte_routes() {
    let four_byte_call = DepositWalletCall {
        target: Address::from_low_u64_be(1),
        value: U256::zero(),
        data: Bytes::from(vec![0xde, 0xad, 0xbe, 0xef]),
    };
    let four_byte_summary = summarize_batch_calls(&[four_byte_call]);
    let four_byte_json =
        serde_json::to_string(&four_byte_summary).expect("summary should serialize");
    let four_byte_debug = format!("{four_byte_summary:?}");

    assert_eq!(four_byte_summary.calls()[0].selector(), None);
    assert_eq!(four_byte_summary.calls()[0].data_len(), 4);
    assert!(!four_byte_json.contains("deadbeef"));
    assert!(!four_byte_debug.contains("deadbeef"));

    let five_byte_call = DepositWalletCall {
        target: Address::from_low_u64_be(1),
        value: U256::zero(),
        data: Bytes::from(vec![0xde, 0xad, 0xbe, 0xef, 0x01]),
    };
    let five_byte_summary = summarize_batch_calls(&[five_byte_call]);
    let five_byte_json =
        serde_json::to_string(&five_byte_summary).expect("summary should serialize");
    let five_byte_debug = format!("{five_byte_summary:?}");
    assert_eq!(
        five_byte_summary.calls()[0].selector(),
        Some("0xdeadbeef")
    );
    assert_eq!(five_byte_summary.calls()[0].data_len(), 5);
    assert!(!five_byte_json.contains("deadbeef01"));
    assert!(!five_byte_debug.contains("deadbeef01"));
}

#[test]
fn summary_handles_empty_calldata_without_losing_exact_lengths() {
    let empty_call = DepositWalletCall {
        target: Address::from_low_u64_be(1),
        value: U256::from(9u64),
        data: Bytes::new(),
    };
    let single = summarize_batch_calls(std::slice::from_ref(&empty_call));
    assert_eq!(single.call_count(), 1);
    assert_eq!(single.total_calldata_bytes(), 0);
    assert_eq!(single.calls()[0].data_len(), 0);
    assert_eq!(single.calls()[0].selector(), None);
    assert_eq!(single.redaction(), REDACTION_MARKER);

    let builder_call = all_builder_calls().remove(0);
    let mixed = summarize_batch_calls(&[empty_call.clone(), empty_call, builder_call.clone()]);
    assert_eq!(mixed.call_count(), 3);
    assert_eq!(mixed.total_calldata_bytes(), builder_call.data.len());
    for call_summary in &mixed.calls()[..2] {
        assert_eq!(call_summary.data_len(), 0);
        assert_eq!(call_summary.selector(), None);
    }
    assert_eq!(mixed.calls()[2].data_len(), builder_call.data.len());
    assert_eq!(mixed.redaction(), REDACTION_MARKER);
}

#[test]
fn summary_accepts_an_empty_batch_without_validation() {
    let summary = summarize_batch_calls(&[]);

    assert_eq!(summary.call_count(), 0);
    assert_eq!(summary.total_calldata_bytes(), 0);
    assert!(summary.calls().is_empty());
    assert_eq!(summary.redaction(), REDACTION_MARKER);
}

fn all_builder_calls() -> Vec<DepositWalletCall> {
    let config = polygon_calldata_config().expect("Polygon config should validate");
    let condition_id = h256(CONDITION_ID);
    let partition = [U256::one(), U256::from(2u64)];
    let one_pusd = PusdAmount::from_base_units(U256::from(1_000_000u64))
        .expect("fixture pUSD amount should validate");
    let position_amounts = [
        CtfPositionAmount::from_base_units(U256::from(1_000_000u64))
            .expect("fixture position amount should validate"),
        CtfPositionAmount::from_base_units(U256::from(2_000_000u64))
            .expect("fixture position amount should validate"),
    ];

    vec![
        build_pusd_approval_call(
            &config,
            address(POLYGON_CTF),
            PusdAmount::unlimited(),
        )
        .expect("pUSD approval should build"),
        build_ctf_approval_for_all_call(
            &config,
            address(POLYGON_STANDARD_EXCHANGE),
            true,
        )
        .expect("CTF approval should build"),
        build_split_position_call(&config, condition_id, &partition, one_pusd)
            .expect("split should build"),
        build_merge_positions_call(&config, condition_id, &partition, one_pusd)
            .expect("merge should build"),
        build_redeem_positions_call(&config, condition_id, &partition)
            .expect("redeem should build"),
        build_neg_risk_redeem_positions_call(
            &config,
            address(POLYGON_NEG_RISK_ADAPTER),
            condition_id,
            &position_amounts,
        )
        .expect("NegRisk redeem should build"),
    ]
}

fn fixture(raw: &str) -> Value {
    serde_json::from_str(raw).expect("fixture should be valid JSON")
}

fn fixture_context(data: &Value) -> DepositWalletRequestContext {
    let owner = address(data["owner"].as_str().expect("fixture owner should be a string"));
    let deposit_wallet = address(
        data["depositWallet"]
            .as_str()
            .expect("fixture deposit wallet should be a string"),
    );
    let config = deposit_wallet_contract_config(
        data["chainId"]
            .as_u64()
            .expect("fixture chain id should be a number"),
    )
    .expect("fixture chain should be configured");
    let derived =
        derive_deposit_wallet_address(owner, config).expect("fixture wallet should derive");
    assert!(
        derived == deposit_wallet,
        "fixture deposit wallet must match the configured owner derivation"
    );

    DepositWalletRequestContext {
        owner_address: owner,
        deposit_wallet_address: deposit_wallet,
    }
}

fn fixture_u256(data: &Value, key: &str) -> U256 {
    U256::from_dec_str(
        data[key]
            .as_str()
            .unwrap_or_else(|| panic!("fixture {key} should be a decimal string")),
    )
    .unwrap_or_else(|_| panic!("fixture {key} should parse as U256"))
}

fn fixture_string(data: &Value, key: &str) -> String {
    data[key]
        .as_str()
        .unwrap_or_else(|| panic!("fixture {key} should be a string"))
        .to_string()
}

fn assert_call_matches_fixture(call: &DepositWalletCall, fixture_raw: &str) {
    let expected = fixture(fixture_raw);
    let expected_target = address(
        expected["target"]
            .as_str()
            .expect("fixture target should be a string"),
    );
    let expected_value = U256::from_dec_str(
        expected["value"]
            .as_str()
            .expect("fixture value should be a decimal string"),
    )
    .expect("fixture value should parse as U256");
    let expected_data = hex::decode(
        expected["data"]
            .as_str()
            .expect("fixture data should be a string")
            .strip_prefix("0x")
            .expect("fixture data should be 0x-prefixed"),
    )
    .expect("fixture data should be hex");

    assert!(call.target == expected_target, "builder target must match fixture");
    assert!(call.value == expected_value, "builder value must match fixture");
    assert!(
        call.data.as_ref() == expected_data.as_slice(),
        "builder calldata must match fixture"
    );
}

fn assert_signing_error_contains<T>(result: Result<T>, expected: &str) {
    match result {
        Err(RelayerError::Signing(message)) => assert!(
            message.contains(expected),
            "expected signing error containing {expected:?}"
        ),
        Err(_) => panic!("expected signing error containing {expected:?}"),
        Ok(_) => panic!("expected signing error containing {expected:?}"),
    }
}

fn assert_other_error_contains<T>(result: Result<T>, expected: &str) {
    match result {
        Err(RelayerError::Other(message)) => assert!(
            message.contains(expected),
            "expected builder error containing {expected:?}"
        ),
        Err(_) => panic!("expected builder error containing {expected:?}"),
        Ok(_) => panic!("expected builder error containing {expected:?}"),
    }
}

fn selector_hex(route: CtfRoute) -> String {
    format!("0x{}", hex::encode(route.selector()))
}

fn redacted_address(value: Address) -> String {
    let checksum = to_checksum(&value, None);
    format!("{}...{}", &checksum[..6], &checksum[38..])
}

fn address(raw: &str) -> Address {
    raw.parse().expect("embedded address should parse")
}

fn h256(raw: &str) -> H256 {
    raw.parse().expect("embedded condition id should parse")
}
