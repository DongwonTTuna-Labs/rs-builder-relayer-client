use ethers::core::rand::{rngs::StdRng, SeedableRng};
use ethers::signers::{LocalWallet, Signer};
use ethers::types::{Address, Bytes, H256, U256};
use ethers::utils::to_checksum;
use polymarket_relayer::auth::AuthMethod;
use polymarket_relayer::{
    build_deposit_wallet_batch_request_from_signed,
    build_wallet_create_request, build_wallet_nonce_request, deposit_wallet_contract_config,
    digest_deposit_wallet_batch, recover_deposit_wallet_batch_signer,
    try_build_deposit_wallet_batch_typed_data, try_build_wallet_batch_request_with_signature,
    validate_deposit_wallet_batch_signature, DepositWalletBatchToSign, DepositWalletCall,
    DepositWalletContractConfig, DepositWalletParams, DepositWalletRequestContext, RelayerError,
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

    let typed_data = try_build_deposit_wallet_batch_typed_data(&batch).unwrap();

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
fn wallet_batch_multicall_matches_official_sdk_fixture() {
    let data = fixture("deposit_wallet/wallet_batch_eip712_multicall.json");
    let batch = batch_from_fixture(&data);
    let config = deposit_wallet_contract_config(data["chainId"].as_u64().unwrap()).unwrap();
    let expected_digest: H256 = data["expectedDigest"].as_str().unwrap().parse().unwrap();
    let expected_signer: Address = data["ownerRecoveredSigner"].as_str().unwrap().parse().unwrap();

    assert_eq!(batch.calls.len(), 2);
    assert!(batch.calls.iter().any(|call| call.value != U256::zero()));
    assert_eq!(
        try_build_deposit_wallet_batch_typed_data(&batch).unwrap(),
        data["typedData"]
    );
    assert_eq!(digest_deposit_wallet_batch(&batch).unwrap(), expected_digest);

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
    let request = build_deposit_wallet_batch_request_from_signed(&signed, config).unwrap();

    assert_eq!(recovered, expected_signer);
    assert_eq!(signed.verified_signer(), expected_signer);
    assert_eq!(
        serde_json::to_value(request).unwrap(),
        fixture("deposit_wallet/wallet_signed_submit_body_multicall.json")
    );
}

#[test]
fn wallet_batch_signature_rejects_non_owner_signer() {
    let data = fixture("deposit_wallet/wallet_batch_eip712.json");
    let batch = batch_from_fixture(&data);
    let expected_signer: Address = data["nonOwnerRecoveredSigner"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();

    let recovered = recover_deposit_wallet_batch_signer(
        &batch,
        data["nonOwnerSignature"].as_str().unwrap(),
    )
    .unwrap();

    assert_eq!(recovered, expected_signer);
    assert_signing_error_contains(
        validate_deposit_wallet_batch_signature(&batch, data["nonOwnerSignature"].as_str().unwrap()),
        "signer must match owner",
    );
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
    assert_signing_error_contains(
        validate_deposit_wallet_batch_signature(
            &batch,
            data["unauthorizedSignature"].as_str().unwrap(),
        ),
        "signer must match owner",
    );
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
    let unrecoverable_signature = format!("0x{}", "00".repeat(65));

    for signature in invalid_signatures {
        assert_signing_error_contains(
            recover_deposit_wallet_batch_signer(&batch, &signature),
            "0x-prefixed 65-byte hex",
        );
        assert_signing_error_contains(
            validate_deposit_wallet_batch_signature(&batch, &signature),
            "0x-prefixed 65-byte hex",
        );
    }

    assert_signing_error_contains(
        recover_deposit_wallet_batch_signer(&batch, &unrecoverable_signature),
        "could not recover deposit wallet signer",
    );
    assert_signing_error_contains(
        validate_deposit_wallet_batch_signature(&batch, &unrecoverable_signature),
        "could not recover deposit wallet signer",
    );
}

#[test]
fn wallet_batch_signature_rejects_resource_abuse_before_digest() {
    let data = fixture("deposit_wallet/wallet_batch_eip712.json");
    let batch = batch_from_fixture(&data);
    let valid_signature_shape = data["ownerSignature"].as_str().unwrap();
    let malformed_signature = valid_signature_shape.trim_start_matches("0x");

    let mut too_many_calls = batch.clone();
    too_many_calls.calls = vec![batch.calls[0].clone(); 257];
    assert_signing_error_contains(
        try_build_deposit_wallet_batch_typed_data(&too_many_calls),
        "call count exceeds maximum",
    );
    assert_signing_error_contains(
        digest_deposit_wallet_batch(&too_many_calls),
        "call count exceeds maximum",
    );
    assert_signing_error_contains(
        recover_deposit_wallet_batch_signer(&too_many_calls, valid_signature_shape),
        "call count exceeds maximum",
    );
    assert_signing_error_contains(
        validate_deposit_wallet_batch_signature(&too_many_calls, valid_signature_shape),
        "call count exceeds maximum",
    );
    assert_signing_error_contains(
        validate_deposit_wallet_batch_signature(&too_many_calls, malformed_signature),
        "0x-prefixed 65-byte hex",
    );

    let mut too_much_calldata = batch.clone();
    too_much_calldata.calls[0].data = Bytes::from(vec![0u8; 1024 * 1024 + 1]);
    assert_signing_error_contains(
        try_build_deposit_wallet_batch_typed_data(&too_much_calldata),
        "calldata bytes exceed maximum",
    );
    assert_signing_error_contains(
        digest_deposit_wallet_batch(&too_much_calldata),
        "calldata bytes exceed maximum",
    );
    assert_signing_error_contains(
        recover_deposit_wallet_batch_signer(&too_much_calldata, valid_signature_shape),
        "calldata bytes exceed maximum",
    );
    assert_signing_error_contains(
        validate_deposit_wallet_batch_signature(&too_much_calldata, valid_signature_shape),
        "calldata bytes exceed maximum",
    );
}

#[test]
fn wallet_batch_resource_limits_accept_exact_boundaries() {
    let data = fixture("deposit_wallet/wallet_batch_eip712.json");
    let batch = batch_from_fixture(&data);

    let mut max_calls = batch.clone();
    max_calls.calls = vec![batch.calls[0].clone(); 256];
    assert!(try_build_deposit_wallet_batch_typed_data(&max_calls).is_ok());
    assert!(digest_deposit_wallet_batch(&max_calls).is_ok());

    let mut max_calldata = batch;
    max_calldata.calls[0].data = Bytes::from(vec![0u8; 1024 * 1024]);
    assert!(try_build_deposit_wallet_batch_typed_data(&max_calldata).is_ok());
    assert!(digest_deposit_wallet_batch(&max_calldata).is_ok());
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
fn wallet_batch_validation_rejects_signed_payload_mutations() {
    let data = fixture("deposit_wallet/wallet_batch_eip712.json");
    let batch = batch_from_fixture(&data);
    let owner_signature = data["ownerSignature"].as_str().unwrap();

    let mut chain_mutation = batch.clone();
    chain_mutation.chain_id = 80002;
    assert_signing_error_contains(
        validate_deposit_wallet_batch_signature(&chain_mutation, owner_signature),
        "signer must match owner",
    );

    let mut wallet_mutation = batch.clone();
    wallet_mutation.deposit_wallet = "0x000000000000000000000000000000000000dEaD"
        .parse()
        .unwrap();
    assert_signing_error_contains(
        validate_deposit_wallet_batch_signature(&wallet_mutation, owner_signature),
        "signer must match owner",
    );

    let mut nonce_mutation = batch.clone();
    nonce_mutation.nonce += U256::one();
    assert_signing_error_contains(
        validate_deposit_wallet_batch_signature(&nonce_mutation, owner_signature),
        "signer must match owner",
    );

    let mut deadline_mutation = batch.clone();
    deadline_mutation.deadline += U256::one();
    assert_signing_error_contains(
        validate_deposit_wallet_batch_signature(&deadline_mutation, owner_signature),
        "signer must match owner",
    );

    let mut target_mutation = batch.clone();
    target_mutation.calls[0].target = "0x0000000000000000000000000000000000000001"
        .parse()
        .unwrap();
    assert_signing_error_contains(
        validate_deposit_wallet_batch_signature(&target_mutation, owner_signature),
        "signer must match owner",
    );

    let mut value_mutation = batch.clone();
    value_mutation.calls[0].value += U256::one();
    assert_signing_error_contains(
        validate_deposit_wallet_batch_signature(&value_mutation, owner_signature),
        "signer must match owner",
    );

    let mut data_mutation = batch;
    let mut call_data = data_mutation.calls[0].data.as_ref().to_vec();
    call_data.push(0);
    data_mutation.calls[0].data = Bytes::from(call_data);
    assert_signing_error_contains(
        validate_deposit_wallet_batch_signature(&data_mutation, owner_signature),
        "signer must match owner",
    );

    let multicall_data = fixture("deposit_wallet/wallet_batch_eip712_multicall.json");
    let mut order_mutation = batch_from_fixture(&multicall_data);
    order_mutation.calls.swap(0, 1);
    assert_signing_error_contains(
        validate_deposit_wallet_batch_signature(
            &order_mutation,
            multicall_data["ownerSignature"].as_str().unwrap(),
        ),
        "signer must match owner",
    );
}

#[test]
fn wallet_batch_validation_rejects_owner_or_submit_identity_mutation() {
    let data = fixture("deposit_wallet/wallet_batch_eip712.json");
    let batch = batch_from_fixture(&data);

    let mut owner_mutation = batch.clone();
    owner_mutation.owner = "0x000000000000000000000000000000000000dEaD"
        .parse()
        .unwrap();
    owner_mutation.nonce_owner = owner_mutation.owner;
    owner_mutation.submit_from = owner_mutation.owner;
    assert_signing_error_contains(
        validate_deposit_wallet_batch_signature(
            &owner_mutation,
            data["ownerSignature"].as_str().unwrap(),
        ),
        "signer must match owner",
    );

    let mut nonce_owner_mutation = batch.clone();
    nonce_owner_mutation.nonce_owner = "0x000000000000000000000000000000000000dEaD"
        .parse()
        .unwrap();
    assert_signing_error_contains(
        validate_deposit_wallet_batch_signature(
            &nonce_owner_mutation,
            data["ownerSignature"].as_str().unwrap(),
        ),
        "nonce owner must match owner signer",
    );

    let mut submit_from_mutation = batch;
    submit_from_mutation.submit_from = "0x000000000000000000000000000000000000dEaD"
        .parse()
        .unwrap();
    assert_signing_error_contains(
        validate_deposit_wallet_batch_signature(
            &submit_from_mutation,
            data["ownerSignature"].as_str().unwrap(),
        ),
        "submit from must match owner signer",
    );
}

#[test]
fn signed_batch_submit_request_matches_fixture_and_rejects_config_mismatch() {
    let data = fixture("deposit_wallet/wallet_batch_eip712.json");
    let batch = batch_from_fixture(&data);
    let config = deposit_wallet_contract_config(data["chainId"].as_u64().unwrap()).unwrap();
    let from_debug = format!("{:?}", batch.submit_from);
    let to_debug = format!("{:?}", config.factory);
    let deposit_wallet_debug = format!("{:?}", batch.deposit_wallet);
    let signed = validate_deposit_wallet_batch_signature(
        &batch,
        data["ownerSignature"].as_str().unwrap(),
    )
    .unwrap();

    let request = build_deposit_wallet_batch_request_from_signed(&signed, config).unwrap();
    let debug = format!("{request:?}");

    assert!(debug.contains("calls_count"));
    assert!(!debug.contains("signature"));
    assert!(!debug.contains(&from_debug));
    assert!(!debug.contains(&to_debug));
    assert!(!debug.contains(&deposit_wallet_debug));
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

    let mut rng = StdRng::seed_from_u64(1);
    let wallet = LocalWallet::new(&mut rng);
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

    let mut rng = StdRng::seed_from_u64(2);
    let unsupported_wallet = LocalWallet::new(&mut rng);
    let unsupported_owner = unsupported_wallet.address();
    let mut unsupported_chain_batch = batch_from_fixture(&data);
    unsupported_chain_batch.owner = unsupported_owner;
    unsupported_chain_batch.nonce_owner = unsupported_owner;
    unsupported_chain_batch.submit_from = unsupported_owner;
    unsupported_chain_batch.chain_id = 999_999;
    let unsupported_digest = digest_deposit_wallet_batch(&unsupported_chain_batch).unwrap();
    let unsupported_signature = format!(
        "0x{}",
        unsupported_wallet.sign_hash(unsupported_digest).unwrap()
    );
    let unsupported_signed = validate_deposit_wallet_batch_signature(
        &unsupported_chain_batch,
        &unsupported_signature,
    )
    .unwrap();
    match build_deposit_wallet_batch_request_from_signed(&unsupported_signed, config) {
        Err(RelayerError::Other(message)) => assert!(
            message.contains("Deposit wallet contracts are not configured for chain 999999"),
            "unexpected unsupported chain error: {message:?}"
        ),
        Err(error) => panic!("expected unsupported chain error, got {error:?}"),
        Ok(_) => panic!("expected unsupported chain error, got success"),
    }
}

#[test]
fn wallet_batch_public_try_builder_matches_signed_submit_fixture() {
    let data = fixture("deposit_wallet/wallet_batch_eip712.json");
    let batch = batch_from_fixture(&data);
    let config = deposit_wallet_contract_config(data["chainId"].as_u64().unwrap()).unwrap();
    let ctx = DepositWalletRequestContext {
        owner_address: batch.submit_from,
        deposit_wallet_address: batch.deposit_wallet,
    };

    let request = try_build_wallet_batch_request_with_signature(
        ctx,
        config,
        batch.nonce,
        batch.deadline,
        batch.calls,
        data["ownerSignature"].as_str().unwrap().to_string(),
    )
    .unwrap();

    assert_eq!(
        serde_json::to_value(request).unwrap(),
        fixture("deposit_wallet/wallet_signed_submit_body.json")
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
    let batch_debug = format!("{batch:?}");
    let owner_debug = format!("{:?}", batch.owner);
    let nonce_owner_debug = format!("{:?}", batch.nonce_owner);
    let submit_from_debug = format!("{:?}", batch.submit_from);
    let deposit_wallet_debug = format!("{:?}", batch.deposit_wallet);
    let owner_checksum = to_checksum(&batch.owner, None);
    let nonce_owner_checksum = to_checksum(&batch.nonce_owner, None);
    let submit_from_checksum = to_checksum(&batch.submit_from, None);
    let deposit_wallet_checksum = to_checksum(&batch.deposit_wallet, None);

    assert!(debug.contains("signature: \"<redacted>\""));
    assert!(!debug.contains("typed_data"));
    assert!(debug.contains("calls_count"));
    for raw_address in [
        owner_debug,
        nonce_owner_debug,
        submit_from_debug,
        deposit_wallet_debug,
        owner_checksum,
        nonce_owner_checksum,
        submit_from_checksum,
        deposit_wallet_checksum,
    ] {
        assert!(!debug.contains(&raw_address));
        assert!(!batch_debug.contains(&raw_address));
    }
    assert!(!debug.contains(data["ownerSignature"].as_str().unwrap()));
    assert!(!debug.contains(data["calls"][0]["data"].as_str().unwrap()));
    assert!(!batch_debug.contains(data["calls"][0]["data"].as_str().unwrap()));
    assert!(!debug.contains("primaryType"));
    assert!(batch_debug.contains("calls_count"));
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
    let debug = format!("{request:?}");

    assert!(!debug.contains(&format!("{owner:?}")));
    assert!(!debug.contains(data["address"].as_str().unwrap()));
    assert!(debug.contains("..."));
    assert_eq!(serde_json::to_value(request).unwrap(), data);
}

#[test]
fn deposit_wallet_public_debug_outputs_redacted_summaries() {
    let data = fixture("deposit_wallet/wallet_batch_eip712.json");
    let batch = batch_from_fixture(&data);
    let config = deposit_wallet_contract_config(data["chainId"].as_u64().unwrap()).unwrap();
    let context = DepositWalletRequestContext {
        owner_address: batch.submit_from,
        deposit_wallet_address: batch.deposit_wallet,
    };
    let create_request = build_wallet_create_request(batch.owner, config);
    let params = DepositWalletParams {
        deposit_wallet: batch.deposit_wallet,
        deadline: batch.deadline,
        calls: batch.calls.clone(),
    };

    let raw_owner_debug = format!("{:?}", batch.owner);
    let raw_submit_from_debug = format!("{:?}", batch.submit_from);
    let raw_deposit_wallet_debug = format!("{:?}", batch.deposit_wallet);
    let raw_factory_debug = format!("{:?}", config.factory);
    let raw_call_target_debug = format!("{:?}", batch.calls[0].target);
    let raw_owner_checksum = to_checksum(&batch.owner, None);
    let raw_submit_from_checksum = to_checksum(&batch.submit_from, None);
    let raw_deposit_wallet_checksum = to_checksum(&batch.deposit_wallet, None);
    let raw_factory_checksum = to_checksum(&config.factory, None);
    let raw_call_target_checksum = to_checksum(&batch.calls[0].target, None);
    let raw_call_data = data["calls"][0]["data"].as_str().unwrap();
    let context_debug = format!("{context:?}");
    let create_debug = format!("{create_request:?}");
    let call_debug = format!("{:?}", batch.calls[0]);
    let params_debug = format!("{params:?}");

    for debug in [&context_debug, &create_debug, &call_debug, &params_debug] {
        for raw in [
            raw_owner_debug.as_str(),
            raw_submit_from_debug.as_str(),
            raw_deposit_wallet_debug.as_str(),
            raw_factory_debug.as_str(),
            raw_call_target_debug.as_str(),
            raw_owner_checksum.as_str(),
            raw_submit_from_checksum.as_str(),
            raw_deposit_wallet_checksum.as_str(),
            raw_factory_checksum.as_str(),
            raw_call_target_checksum.as_str(),
            raw_call_data,
        ] {
            assert!(!debug.contains(raw), "debug leaked raw value {raw}: {debug}");
        }
        assert!(debug.contains("..."), "debug should include redacted address summary: {debug}");
    }
    assert!(call_debug.contains("data: \"<redacted>\""));
    assert!(call_debug.contains("data_len"));
    assert!(params_debug.contains("calls_count"));
}

#[test]
fn wallet_nonce_signing_and_submit_keep_auth_identity_separate_from_owner() {
    let data = fixture("deposit_wallet/wallet_batch_eip712.json");
    let batch = batch_from_fixture(&data);
    let owner = batch.owner;
    let auth_address = "0x1111111111111111111111111111111111111111";
    let auth = AuthMethod::relayer_key("my-key", auth_address);

    let nonce_request = build_wallet_nonce_request(owner);
    let headers = auth
        .headers("GET", nonce_request.path_and_query.as_str(), "")
        .unwrap();
    let signed =
        validate_deposit_wallet_batch_signature(&batch, data["ownerSignature"].as_str().unwrap())
            .unwrap();
    let submit_request = serde_json::to_value(
        build_deposit_wallet_batch_request_from_signed(
            &signed,
            deposit_wallet_contract_config(batch.chain_id).unwrap(),
        )
        .unwrap(),
    )
    .unwrap();

    assert_ne!(headers.get("RELAYER_API_KEY_ADDRESS").unwrap(), data["owner"].as_str().unwrap());
    assert_eq!(headers.get("RELAYER_API_KEY_ADDRESS").unwrap(), auth_address);
    assert_eq!(nonce_request.address, owner);
    assert_eq!(signed.owner(), owner);
    assert_eq!(submit_request["from"], data["submitFrom"]);
    assert_eq!(submit_request["from"], data["owner"]);
}
