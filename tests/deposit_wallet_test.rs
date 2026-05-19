use ethers::types::{Address, Bytes, U256};
use polymarket_relayer::{
    build_wallet_batch_request_with_signature, build_wallet_create_request,
    deposit_wallet_contract_config, derive_deposit_wallet_address, DepositWalletCall,
    DepositWalletRequestContext, RelayerTransactionState,
};
use serde_json::Value;

fn fixture(path: &str) -> Value {
    let full_path = format!("tests/fixtures/{path}");
    let text = std::fs::read_to_string(&full_path).expect("fixture should be readable");
    serde_json::from_str(&text).expect("fixture should be valid JSON")
}

#[test]
fn derive_deposit_wallet_address_matches_official_python_reference() {
    let data = fixture("deposit_wallet/derive_address.json");
    let owner: Address = data["owner"].as_str().unwrap().parse().unwrap();
    let expected: Address = data["expectedDepositWallet"].as_str().unwrap().parse().unwrap();
    let config = deposit_wallet_contract_config(data["chainId"].as_u64().unwrap()).unwrap();

    let derived = derive_deposit_wallet_address(owner, config).unwrap();

    assert_eq!(derived, expected);
}

#[test]
fn wallet_create_submit_body_matches_fixture() {
    let owner: Address = "0x6e0c80c90ea6c15917308F820Eac91Ce2724B5b5"
        .parse()
        .unwrap();
    let config = deposit_wallet_contract_config(137).unwrap();

    let request = build_wallet_create_request(owner, config);

    assert_eq!(
        serde_json::to_value(request).unwrap(),
        fixture("deposit_wallet/wallet_create_submit_body.json")
    );
}

#[test]
fn wallet_batch_submit_body_matches_fixture() {
    let owner: Address = "0x6e0c80c90ea6c15917308F820Eac91Ce2724B5b5"
        .parse()
        .unwrap();
    let deposit_wallet: Address = "0x069F89dAEfbaDdF5B6639Dc34D73E59cCCBC63De"
        .parse()
        .unwrap();
    let target: Address = "0xC011a7E12a19f7B1f670d46F03B03f3342E82DFB"
        .parse()
        .unwrap();
    let config = deposit_wallet_contract_config(137).unwrap();
    let ctx = DepositWalletRequestContext {
        owner_address: owner,
        deposit_wallet_address: deposit_wallet,
    };
    let call = DepositWalletCall {
        target,
        value: U256::zero(),
        data: Bytes::from(hex::decode("095ea7b30000000000000000000000004d97dcd97ec945f40cf65f87097ace5ea0476045ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff").unwrap()),
    };
    let signature = "0x111111111111111111111111111111111111111111111111111111111111111122222222222222222222222222222222222222222222222222222222222222221b";

    let request = build_wallet_batch_request_with_signature(
        ctx,
        config,
        U256::from(31u64),
        U256::from(1_760_000_000u64),
        vec![call],
        signature.to_string(),
    );

    assert_eq!(
        serde_json::to_value(request).unwrap(),
        fixture("deposit_wallet/wallet_submit_body.json")
    );
}

#[test]
fn transaction_state_parser_matches_fixture() {
    let data = fixture("relayer/transaction_states.json");
    let rows = data["states"].as_array().unwrap();

    for row in rows {
        let raw = row[0].as_str().unwrap();
        let expected_label = row[1].as_str().unwrap();
        let expected_terminal = row[2].as_bool().unwrap();
        let expected_success = row[3].as_bool().unwrap();

        let parsed = RelayerTransactionState::parse(raw);

        assert_eq!(parsed.label(), expected_label);
        assert_eq!(parsed.is_terminal(), expected_terminal);
        assert_eq!(parsed.is_success(), expected_success);
    }
}
