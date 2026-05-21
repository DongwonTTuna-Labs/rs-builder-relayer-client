use ethers::signers::{LocalWallet, Signer};
use ethers::types::{Address, Bytes, H256, U256};
use polymarket_relayer::{
    build_deposit_wallet_batch_request_from_signed, build_deposit_wallet_batch_typed_data,
    build_wallet_nonce_request, deposit_wallet_contract_config, digest_deposit_wallet_batch,
    recover_deposit_wallet_batch_signer, validate_deposit_wallet_batch_signature,
    DepositWalletBatchToSign, DepositWalletCall, DepositWalletContractConfig, RelayerError,
};
use serde_json::Value;

fn fixture(path: &str) -> Value {
    let full_path = format!("tests/fixtures/{path}");
    let text = std::fs::read_to_string(&full_path).expect("fixture should be readable");
    serde_json::from_str(&text).expect("fixture should be valid JSON")
}

fn parse_address(value: &Value, key: &str) -> Address {
    value[key].as_str().unwrap().parse().unwrap()
}

fn parse_u256(value: &Value, key: &str) -> U256 {
    U256::from_dec_str(value[key].as_str().unwrap()).unwrap()
}

fn call_from_value(value: &Value) -> DepositWalletCall {
    DepositWalletCall {
        target: value["target"].as_str().unwrap().parse().unwrap(),
        value: U256::from_dec_str(value["value"].as_str().unwrap()).unwrap(),
        data: Bytes::from(hex::decode(&value["data"].as_str().unwrap()[2..]).unwrap()),
    }
}

fn batch_from_fixture(data: &Value) -> DepositWalletBatchToSign {
    DepositWalletBatchToSign {
        owner: parse_address(data, "owner"),
        nonce_owner: parse_address(data, "nonceOwner"),
        submit_from: parse_address(data, "submitFrom"),
        deposit_wallet: parse_address(data, "depositWallet"),
        chain_id: data["chainId"].as_u64().unwrap(),
        nonce: parse_u256(data, "nonce"),
        deadline: parse_u256(data, "deadline"),
        calls: data["calls"].as_array().unwrap().iter().map(call_from_value).collect(),
    }
}

fn assert_signing_error_contains<T>(
    result: std::result::Result<T, RelayerError>,
    expected: &str,
) {
    match result {
        Err(RelayerError::Signing(message)) => assert!(
            message.contains(expected),
            "expected signing error containing {expected:?}, got {message:?}"
        ),
        Err(error) => panic!("expected signing error containing {expected:?}, got {error:?}"),
        Ok(_) => panic!("expected signing error containing {expected:?}, got success"),
    }
}

#[test]
fn deposit_wallet_contract_config_keeps_public_literal_compatibility() {
    let config = deposit_wallet_contract_config(137).unwrap();

    let literal = DepositWalletContractConfig {
        factory: config.factory,
        implementation: config.implementation,
    };

    assert_eq!(literal, config);
}

#[test]
fn wallet_batch_typed_data_matches_official_sdk_fixture() {
    let data = fixture("deposit_wallet/wallet_batch_eip712.json");
    let batch = batch_from_fixture(&data);

    let typed_data = build_deposit_wallet_batch_typed_data(&batch);

    assert_eq!(typed_data, data["typedData"]);
}

#[test]
fn wallet_batch_digest_matches_official_sdk_fixture() {
    let data = fixture("deposit_wallet/wallet_batch_eip712.json");
    let batch = batch_from_fixture(&data);
    let expected_digest: H256 = data["expectedDigest"].as_str().unwrap().parse().unwrap();

    let digest = digest_deposit_wallet_batch(&batch).unwrap();

    assert_eq!(digest, expected_digest);
}

#[test]
fn wallet_batch_signature_recovery_accepts_owner() {
    let data = fixture("deposit_wallet/wallet_batch_eip712.json");
    let batch = batch_from_fixture(&data);
    let expected_signer: Address = data["ownerRecoveredSigner"].as_str().unwrap().parse().unwrap();

    let recovered = recover_deposit_wallet_batch_signer(
        &batch,
        data["ownerSignature"].as_str().unwrap(),
    )
    .unwrap();
    let signed = validate_deposit_wallet_batch_signature(
        &batch,
        data["ownerSignature"].as_str().unwrap(),
    )
    .unwrap();

    assert_eq!(recovered, expected_signer);
    assert_eq!(signed.verified_signer(), expected_signer);
}

#[test]
fn wallet_batch_signature_rejects_non_owner_session_signer() {
    let data = fixture("deposit_wallet/wallet_batch_eip712.json");
    let batch = batch_from_fixture(&data);
    let expected_signer: Address = data["approvedSessionRecoveredSigner"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();

    let recovered = recover_deposit_wallet_batch_signer(
        &batch,
        data["approvedSessionSignature"].as_str().unwrap(),
    )
    .unwrap();

    assert_eq!(recovered, expected_signer);
    assert!(validate_deposit_wallet_batch_signature(
        &batch,
        data["approvedSessionSignature"].as_str().unwrap(),
    )
    .is_err());
}

#[test]
fn wallet_batch_signature_rejects_unauthorized_or_self_asserted_session_signer() {
    let data = fixture("deposit_wallet/wallet_batch_eip712.json");
    let batch = batch_from_fixture(&data);
    let recovered = recover_deposit_wallet_batch_signer(
        &batch,
        data["unauthorizedSignature"].as_str().unwrap(),
    )
    .unwrap();
    let self_asserted: Address = data["selfAssertedSessionSigner"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();

    assert_eq!(recovered, self_asserted);
    assert!(validate_deposit_wallet_batch_signature(
        &batch,
        data["unauthorizedSignature"].as_str().unwrap(),
    )
    .is_err());
}

#[test]
fn wallet_batch_signature_rejects_malformed_signature_shapes() {
    let data = fixture("deposit_wallet/wallet_batch_eip712.json");
    let batch = batch_from_fixture(&data);
    let valid = data["ownerSignature"].as_str().unwrap();
    let invalid_signatures = [
        valid.trim_start_matches("0x").to_string(),
        valid[..valid.len() - 2].to_string(),
        format!("{valid}00"),
        format!("{}zz", &valid[..valid.len() - 2]),
    ];

    for signature in invalid_signatures {
        assert_signing_error_contains(
            validate_deposit_wallet_batch_signature(&batch, &signature),
            "0x-prefixed 65-byte hex",
        );
    }
}

#[test]
fn wallet_batch_digest_changes_when_domain_or_message_changes() {
    let data = fixture("deposit_wallet/wallet_batch_eip712.json");
    let batch = batch_from_fixture(&data);
    let baseline = digest_deposit_wallet_batch(&batch).unwrap();

    let mut chain_mutation = batch.clone();
    chain_mutation.chain_id = 80002;
    assert_ne!(digest_deposit_wallet_batch(&chain_mutation).unwrap(), baseline);

    let mut wallet_mutation = batch.clone();
    wallet_mutation.deposit_wallet = "0x000000000000000000000000000000000000dEaD"
        .parse()
        .unwrap();
    assert_ne!(digest_deposit_wallet_batch(&wallet_mutation).unwrap(), baseline);

    let mut nonce_mutation = batch.clone();
    nonce_mutation.nonce += U256::one();
    assert_ne!(digest_deposit_wallet_batch(&nonce_mutation).unwrap(), baseline);

    let mut deadline_mutation = batch.clone();
    deadline_mutation.deadline += U256::one();
    assert_ne!(digest_deposit_wallet_batch(&deadline_mutation).unwrap(), baseline);

    let mut call_mutation = batch.clone();
    call_mutation.calls[0].target = "0x0000000000000000000000000000000000000001"
        .parse()
        .unwrap();
    assert_ne!(digest_deposit_wallet_batch(&call_mutation).unwrap(), baseline);
}

#[test]
fn wallet_batch_validation_rejects_owner_or_submit_identity_mutation() {
    let data = fixture("deposit_wallet/wallet_batch_eip712.json");
    let batch = batch_from_fixture(&data);

    let mut owner_mutation = batch.clone();
    owner_mutation.owner = "0x000000000000000000000000000000000000dEaD"
        .parse()
        .unwrap();
    assert!(validate_deposit_wallet_batch_signature(
        &owner_mutation,
        data["ownerSignature"].as_str().unwrap(),
    )
    .is_err());

    let mut nonce_owner_mutation = batch.clone();
    nonce_owner_mutation.nonce_owner = "0x000000000000000000000000000000000000dEaD"
        .parse()
        .unwrap();
    assert!(validate_deposit_wallet_batch_signature(
        &nonce_owner_mutation,
        data["ownerSignature"].as_str().unwrap(),
    )
    .is_err());

    let mut submit_from_mutation = batch;
    submit_from_mutation.submit_from = "0x000000000000000000000000000000000000dEaD"
        .parse()
        .unwrap();
    assert!(validate_deposit_wallet_batch_signature(
        &submit_from_mutation,
        data["ownerSignature"].as_str().unwrap(),
    )
    .is_err());
}

#[test]
fn signed_batch_submit_request_matches_fixture_and_rejects_config_mismatch() {
    let data = fixture("deposit_wallet/wallet_batch_eip712.json");
    let batch = batch_from_fixture(&data);
    let config = deposit_wallet_contract_config(data["chainId"].as_u64().unwrap()).unwrap();
    let signed = validate_deposit_wallet_batch_signature(
        &batch,
        data["ownerSignature"].as_str().unwrap(),
    )
    .unwrap();

    let request = build_deposit_wallet_batch_request_from_signed(&signed, config).unwrap();
    let debug = format!("{request:?}");

    assert!(debug.contains("calls_count"));
    assert!(!debug.contains("signature"));
    assert!(!debug.contains(data["ownerSignature"].as_str().unwrap()));
    assert!(!debug.contains(data["calls"][0]["data"].as_str().unwrap()));
    assert_eq!(
        serde_json::to_value(request).unwrap(),
        fixture("deposit_wallet/wallet_signed_submit_body.json")
    );

    let mismatched_chain_config = deposit_wallet_contract_config(80002).unwrap();
    assert_signing_error_contains(
        build_deposit_wallet_batch_request_from_signed(&signed, mismatched_chain_config),
        "submit config does not match signed chain id",
    );

    let wallet: LocalWallet = "0000000000000000000000000000000000000000000000000000000000000001"
        .parse()
        .unwrap();
    let owner = wallet.address();
    let mut wrong_wallet_batch = batch;
    wrong_wallet_batch.owner = owner;
    wrong_wallet_batch.nonce_owner = owner;
    wrong_wallet_batch.submit_from = owner;
    wrong_wallet_batch.deposit_wallet = "0x000000000000000000000000000000000000dEaD"
        .parse()
        .unwrap();
    let wrong_wallet_digest = digest_deposit_wallet_batch(&wrong_wallet_batch).unwrap();
    let wrong_wallet_signature = format!("0x{}", wallet.sign_hash(wrong_wallet_digest).unwrap());
    let wrong_wallet_signed =
        validate_deposit_wallet_batch_signature(&wrong_wallet_batch, &wrong_wallet_signature)
            .unwrap();
    assert_signing_error_contains(
        build_deposit_wallet_batch_request_from_signed(&wrong_wallet_signed, config),
        "wallet does not match owner/config derived wallet",
    );
}

#[test]
fn signed_batch_debug_redacts_signature_and_payload_material() {
    let data = fixture("deposit_wallet/wallet_batch_eip712.json");
    let batch = batch_from_fixture(&data);
    let signed = validate_deposit_wallet_batch_signature(
        &batch,
        data["ownerSignature"].as_str().unwrap(),
    )
    .unwrap();

    let debug = format!("{signed:?}");

    assert!(debug.contains("signature: \"<redacted>\""));
    assert!(debug.contains("typed_data: \"<redacted>\""));
    assert!(debug.contains("calls_count"));
    assert!(!debug.contains(data["ownerSignature"].as_str().unwrap()));
    assert!(!debug.contains(data["calls"][0]["data"].as_str().unwrap()));
    assert!(!debug.contains("primaryType"));
}

#[test]
fn signed_batch_accessors_expose_safe_metadata() {
    let data = fixture("deposit_wallet/wallet_batch_eip712.json");
    let batch = batch_from_fixture(&data);
    let signed = validate_deposit_wallet_batch_signature(
        &batch,
        data["ownerSignature"].as_str().unwrap(),
    )
    .unwrap();

    assert_eq!(signed.owner(), batch.owner);
    assert_eq!(signed.nonce_owner(), batch.nonce_owner);
    assert_eq!(signed.submit_from(), batch.submit_from);
    assert_eq!(signed.deposit_wallet(), batch.deposit_wallet);
    assert_eq!(signed.chain_id(), batch.chain_id);
    assert_eq!(signed.nonce(), batch.nonce);
    assert_eq!(signed.deadline(), batch.deadline);
    assert_eq!(signed.calls(), batch.calls.as_slice());
    assert_eq!(signed.digest(), digest_deposit_wallet_batch(&batch).unwrap());
    assert_eq!(
        signed.verified_signer(),
        data["ownerRecoveredSigner"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap()
    );
}

#[test]
fn wallet_nonce_request_matches_fixture() {
    let data = fixture("deposit_wallet/wallet_nonce_request.json");
    let owner: Address = data["address"].as_str().unwrap().parse().unwrap();

    let request = build_wallet_nonce_request(owner);

    assert_eq!(serde_json::to_value(request).unwrap(), data);
}
