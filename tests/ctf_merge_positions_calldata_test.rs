use ethers::abi::{decode, ParamType, Token};
use ethers::types::{Address, Bytes, H256, U256};
use polymarket_relayer::deposit_wallet::build_ctf_merge_positions_call;
use polymarket_relayer::RelayerError;
use serde_json::Value;

const MERGE_POSITIONS_SELECTOR: [u8; 4] = [0x9e, 0x72, 0x12, 0xad];
const MERGE_POSITIONS_SIGNATURE: &str = "mergePositions(address,bytes32,bytes32,uint256[],uint256)";

fn fixture(path: &str) -> Value {
    let full_path = format!("tests/fixtures/{path}");
    let text = std::fs::read_to_string(&full_path).expect("fixture should be readable");
    serde_json::from_str(&text).expect("fixture should be valid JSON")
}

fn parse_address(value: &Value, key: &str) -> Address {
    value[key].as_str().unwrap().parse().unwrap()
}

fn parse_h256(value: &Value, key: &str) -> H256 {
    value[key].as_str().unwrap().parse().unwrap()
}

fn parse_u256(value: &Value, key: &str) -> U256 {
    U256::from_dec_str(value[key].as_str().unwrap()).unwrap()
}

fn parse_u256_array(value: &Value, key: &str) -> Vec<U256> {
    value[key]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| U256::from_dec_str(item.as_str().unwrap()).unwrap())
        .collect()
}

fn parse_bytes(value: &Value, key: &str) -> Bytes {
    let hex_value = value[key].as_str().unwrap();
    Bytes::from(hex::decode(&hex_value[2..]).unwrap())
}

fn merge_positions_param_types() -> Vec<ParamType> {
    vec![
        ParamType::Address,
        ParamType::FixedBytes(32),
        ParamType::FixedBytes(32),
        ParamType::Array(Box::new(ParamType::Uint(256))),
        ParamType::Uint(256),
    ]
}

fn merge_positions_tokens(
    collateral: Address,
    parent_collection_id: H256,
    condition_id: H256,
    partition: &[U256],
    amount: U256,
) -> Vec<Token> {
    vec![
        Token::Address(collateral),
        Token::FixedBytes(parent_collection_id.as_bytes().to_vec()),
        Token::FixedBytes(condition_id.as_bytes().to_vec()),
        Token::Array(partition.iter().copied().map(Token::Uint).collect()),
        Token::Uint(amount),
    ]
}

#[test]
fn ctf_merge_positions_calldata_matches_frozen_vectors() {
    let data = fixture("operations/ctf_merge_positions_calldata.json");
    assert_eq!(
        data["provenance"]["signature"].as_str().unwrap(),
        MERGE_POSITIONS_SIGNATURE
    );
    assert_eq!(
        data["provenance"]["selector"].as_str().unwrap(),
        "0x9e7212ad"
    );
    let vectors = data["vectors"].as_array().unwrap();

    for vector in vectors {
        let name = vector["name"].as_str().unwrap();
        let adapter = parse_address(vector, "adapter");
        let collateral = parse_address(vector, "collateral");
        let parent_collection_id = parse_h256(vector, "parent_collection_id");
        let condition_id = parse_h256(vector, "condition_id");
        let partition = parse_u256_array(vector, "partition");
        let amount = parse_u256(vector, "amount");
        let expected_calldata = parse_bytes(vector, "expected_calldata");
        let expected_calldata_hex = vector["expected_calldata"].as_str().unwrap();

        assert_eq!(
            &expected_calldata[..4],
            MERGE_POSITIONS_SELECTOR.as_slice(),
            "{name}: selector should be mergePositions(address,bytes32,bytes32,uint256[],uint256)"
        );
        assert_eq!(
            expected_calldata.len(),
            196 + partition.len() * 32,
            "{name}: mergePositions calldata length should include dynamic partition tail"
        );

        let decoded = decode(&merge_positions_param_types(), &expected_calldata[4..])
            .expect("mergePositions calldata arguments should ABI-decode");
        assert_eq!(
            decoded,
            merge_positions_tokens(
                collateral,
                parent_collection_id,
                condition_id,
                &partition,
                amount,
            ),
            "{name}: decoded mergePositions args should recover fixture inputs"
        );

        let call = match build_ctf_merge_positions_call(
            adapter,
            collateral,
            parent_collection_id,
            condition_id,
            partition,
            amount,
        ) {
            Ok(call) => call,
            Err(error) => panic!(
                "positive vector {name} should build expected calldata {expected_calldata_hex}; got {error:?}"
            ),
        };

        assert_eq!(call.target, adapter, "{name}: target should be adapter");
        assert_eq!(call.value, U256::zero(), "{name}: value should be zero");
        assert_eq!(
            call.data, expected_calldata,
            "{name}: calldata should match frozen fixture"
        );
    }
}

#[test]
fn ctf_merge_positions_calldata_rejects_invalid_inputs() {
    let adapter = "0x4444444444444444444444444444444444444444"
        .parse::<Address>()
        .unwrap();
    let collateral = "0xdddddddddddddddddddddddddddddddddddddddd"
        .parse::<Address>()
        .unwrap();
    let parent_collection_id = "0x0404040404040404040404040404040404040404040404040404040404040404"
        .parse::<H256>()
        .unwrap();
    let condition_id = "0x4040404040404040404040404040404040404040404040404040404040404040"
        .parse::<H256>()
        .unwrap();
    let partition = vec![U256::from(1u64), U256::from(2u64)];
    let amount = U256::from(2_500_000u64);

    assert!(matches!(
        build_ctf_merge_positions_call(
            Address::zero(),
            collateral,
            parent_collection_id,
            condition_id,
            partition.clone(),
            amount,
        ),
        Err(RelayerError::InvalidAddress(_))
    ));
    assert!(matches!(
        build_ctf_merge_positions_call(
            adapter,
            Address::zero(),
            parent_collection_id,
            condition_id,
            partition,
            amount,
        ),
        Err(RelayerError::InvalidAddress(_))
    ));
    assert!(matches!(
        build_ctf_merge_positions_call(
            adapter,
            collateral,
            parent_collection_id,
            condition_id,
            Vec::new(),
            amount,
        ),
        Err(RelayerError::Abi(message)) if message.contains("partition")
    ));
}
