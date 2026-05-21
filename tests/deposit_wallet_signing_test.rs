use ethers::types::{Address, Bytes, H256, U256};
use polymarket_relayer::{
    build_deposit_wallet_batch_request_from_signed, build_deposit_wallet_batch_typed_data,
    build_wallet_nonce_request, deposit_wallet_contract_config, digest_deposit_wallet_batch,
    recover_deposit_wallet_batch_signer, validate_deposit_wallet_batch_signature,
    ApprovedSessionSigner, DepositWalletBatchToSign, DepositWalletCall,
    SessionSignerScope, SessionSignerSource,
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

fn approved_session_signer(data: &Value) -> ApprovedSessionSigner {
    let session = &data["approvedSessionSigner"];
    ApprovedSessionSigner {
        signer: session["signer"].as_str().unwrap().parse().unwrap(),
        owner: session["owner"].as_str().unwrap().parse().unwrap(),
        scope: SessionSignerScope::DepositWalletBatch,
        expires_at: U256::from_dec_str(session["expiresAt"].as_str().unwrap()).unwrap(),
        source: SessionSignerSource::TrustedConfig(session["source"].as_str().unwrap().to_string()),
    }
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
        &[],
    )
    .unwrap();

    assert_eq!(recovered, expected_signer);
    assert_eq!(signed.verified_signer, expected_signer);
    assert!(signed.signer_authorization.is_owner());
}

#[test]
fn wallet_batch_signature_recovery_accepts_trusted_session_signer() {
    let data = fixture("deposit_wallet/wallet_batch_eip712.json");
    let batch = batch_from_fixture(&data);
    let trusted = approved_session_signer(&data);
    let expected_signer: Address = data["approvedSessionRecoveredSigner"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();

    let signed = validate_deposit_wallet_batch_signature(
        &batch,
        data["approvedSessionSignature"].as_str().unwrap(),
        std::slice::from_ref(&trusted),
    )
    .unwrap();

    assert_eq!(signed.verified_signer, expected_signer);
    assert_eq!(signed.signer_authorization.trusted_session_signer(), Some(&trusted));
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
        &[],
    )
    .is_err());
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
        &[],
    )
    .is_err());

    let mut nonce_owner_mutation = batch.clone();
    nonce_owner_mutation.nonce_owner = "0x000000000000000000000000000000000000dEaD"
        .parse()
        .unwrap();
    assert!(validate_deposit_wallet_batch_signature(
        &nonce_owner_mutation,
        data["ownerSignature"].as_str().unwrap(),
        &[],
    )
    .is_err());

    let mut submit_from_mutation = batch;
    submit_from_mutation.submit_from = "0x000000000000000000000000000000000000dEaD"
        .parse()
        .unwrap();
    assert!(validate_deposit_wallet_batch_signature(
        &submit_from_mutation,
        data["ownerSignature"].as_str().unwrap(),
        &[],
    )
    .is_err());
}

#[test]
fn signed_batch_submit_preflight_rejects_metadata_tampering() {
    let data = fixture("deposit_wallet/wallet_batch_eip712.json");
    let batch = batch_from_fixture(&data);
    let config = deposit_wallet_contract_config(data["chainId"].as_u64().unwrap()).unwrap();
    let signed = validate_deposit_wallet_batch_signature(
        &batch,
        data["ownerSignature"].as_str().unwrap(),
        &[],
    )
    .unwrap();

    let request = serde_json::to_value(
        build_deposit_wallet_batch_request_from_signed(&signed, config).unwrap(),
    )
    .unwrap();
    assert_eq!(request["type"], "WALLET");
    assert_eq!(request["from"], data["submitFrom"]);
    assert_eq!(request["nonce"], data["nonce"]);
    assert_eq!(request["signature"], data["ownerSignature"]);
    assert_eq!(request["depositWalletParams"]["depositWallet"], data["depositWallet"]);

    let mut tampered_owner = signed.clone();
    tampered_owner.owner = "0x000000000000000000000000000000000000dEaD"
        .parse()
        .unwrap();
    assert!(build_deposit_wallet_batch_request_from_signed(&tampered_owner, config).is_err());

    let mut tampered_domain = signed.clone();
    tampered_domain.typed_data["domain"]["verifyingContract"] =
        Value::String("0x000000000000000000000000000000000000dEaD".to_string());
    assert!(build_deposit_wallet_batch_request_from_signed(&tampered_domain, config).is_err());

    let mut tampered_signer = signed.clone();
    tampered_signer.verified_signer = "0x000000000000000000000000000000000000dEaD"
        .parse()
        .unwrap();
    assert!(build_deposit_wallet_batch_request_from_signed(&tampered_signer, config).is_err());

    let mut tampered_digest = signed;
    tampered_digest.digest = H256::zero();
    assert!(build_deposit_wallet_batch_request_from_signed(&tampered_digest, config).is_err());
}

#[test]
fn wallet_nonce_request_matches_fixture() {
    let data = fixture("deposit_wallet/wallet_nonce_request.json");
    let owner: Address = data["address"].as_str().unwrap().parse().unwrap();

    let request = build_wallet_nonce_request(owner);

    assert_eq!(serde_json::to_value(request).unwrap(), data);
}
