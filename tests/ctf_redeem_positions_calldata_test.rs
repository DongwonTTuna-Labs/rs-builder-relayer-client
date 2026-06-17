use ethers::abi::{decode, ParamType, Token};
use ethers::types::{Address, Bytes, H256, U256};
use polymarket_relayer::deposit_wallet::build_ctf_redeem_positions_call;
use polymarket_relayer::RelayerError;
use serde_json::Value;

const REDEEM_POSITIONS_SELECTOR: [u8; 4] = [0x01, 0xb7, 0x03, 0x7c];
const REDEEM_POSITIONS_SIGNATURE: &str = "redeemPositions(address,bytes32,bytes32,uint256[])";

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

fn redeem_positions_param_types() -> Vec<ParamType> {
    vec![
        ParamType::Address,
        ParamType::FixedBytes(32),
        ParamType::FixedBytes(32),
        ParamType::Array(Box::new(ParamType::Uint(256))),
    ]
}

fn redeem_positions_tokens(
    collateral: Address,
    parent_collection_id: H256,
    condition_id: H256,
    index_sets: &[U256],
) -> Vec<Token> {
    vec![
        Token::Address(collateral),
        Token::FixedBytes(parent_collection_id.as_bytes().to_vec()),
        Token::FixedBytes(condition_id.as_bytes().to_vec()),
        Token::Array(index_sets.iter().copied().map(Token::Uint).collect()),
    ]
}

fn word_at(data: &[u8], argument_index: usize) -> U256 {
    let start = 4 + argument_index * 32;
    U256::from_big_endian(&data[start..start + 32])
}

#[test]
fn ctf_redeem_positions_calldata_matches_frozen_vectors() {
    let data = fixture("operations/ctf_redeem_positions_calldata.json");
    assert_eq!(
        data["provenance"]["signature"].as_str().unwrap(),
        REDEEM_POSITIONS_SIGNATURE
    );
    assert_eq!(
        data["provenance"]["selector"].as_str().unwrap(),
        "0x01b7037c"
    );
    assert!(
        data["provenance"]["encoding_rule"]
            .as_str()
            .unwrap()
            .contains("no trailing amount word")
    );
    let vectors = data["vectors"].as_array().unwrap();

    for vector in vectors {
        let name = vector["name"].as_str().unwrap();
        let adapter = parse_address(vector, "adapter");
        let collateral = parse_address(vector, "collateral");
        let parent_collection_id = parse_h256(vector, "parent_collection_id");
        let condition_id = parse_h256(vector, "condition_id");
        let index_sets = parse_u256_array(vector, "index_sets");
        let expected_calldata = parse_bytes(vector, "expected_calldata");
        let expected_calldata_hex = vector["expected_calldata"].as_str().unwrap();

        assert!(
            vector.get("amount").is_none(),
            "{name}: redeemPositions fixture must not contain an amount field"
        );
        assert_eq!(
            &expected_calldata[..4],
            REDEEM_POSITIONS_SELECTOR.as_slice(),
            "{name}: selector should be redeemPositions(address,bytes32,bytes32,uint256[])"
        );
        assert_eq!(
            expected_calldata.len(),
            164 + index_sets.len() * 32,
            "{name}: redeemPositions calldata length should include only four head slots and the dynamic indexSets tail"
        );
        assert_ne!(
            expected_calldata.len(),
            196 + index_sets.len() * 32,
            "{name}: redeemPositions calldata must not include split/merge trailing amount"
        );
        assert_eq!(
            word_at(&expected_calldata, 3),
            U256::from(0x80u64),
            "{name}: indexSets dynamic-array offset should be 0x80 for four head slots"
        );

        let decoded = decode(&redeem_positions_param_types(), &expected_calldata[4..])
            .expect("redeemPositions calldata arguments should ABI-decode");
        assert_eq!(
            decoded,
            redeem_positions_tokens(collateral, parent_collection_id, condition_id, &index_sets),
            "{name}: decoded redeemPositions args should recover fixture inputs"
        );

        let call = match build_ctf_redeem_positions_call(
            adapter,
            collateral,
            parent_collection_id,
            condition_id,
            index_sets,
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
fn ctf_redeem_positions_calldata_rejects_invalid_inputs() {
    let adapter = "0x7777777777777777777777777777777777777777"
        .parse::<Address>()
        .unwrap();
    let collateral = "0x1212121212121212121212121212121212121212"
        .parse::<Address>()
        .unwrap();
    let parent_collection_id = "0x0707070707070707070707070707070707070707070707070707070707070707"
        .parse::<H256>()
        .unwrap();
    let condition_id = "0x7070707070707070707070707070707070707070707070707070707070707070"
        .parse::<H256>()
        .unwrap();
    let index_sets = vec![U256::from(1u64), U256::from(2u64)];

    assert!(matches!(
        build_ctf_redeem_positions_call(
            Address::zero(),
            collateral,
            parent_collection_id,
            condition_id,
            index_sets.clone(),
        ),
        Err(RelayerError::InvalidAddress(_))
    ));
    assert!(matches!(
        build_ctf_redeem_positions_call(
            adapter,
            Address::zero(),
            parent_collection_id,
            condition_id,
            index_sets,
        ),
        Err(RelayerError::InvalidAddress(_))
    ));
    assert!(matches!(
        build_ctf_redeem_positions_call(
            adapter,
            collateral,
            parent_collection_id,
            condition_id,
            Vec::new(),
        ),
        Err(RelayerError::Abi(message)) if message.contains("index_sets")
    ));
}
