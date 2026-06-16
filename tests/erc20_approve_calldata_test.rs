use ethers::abi::{decode, ParamType, Token};
use ethers::types::{Address, Bytes, U256};
use polymarket_relayer::build_erc20_approve_call;
use serde_json::Value;

const APPROVE_SELECTOR: [u8; 4] = [0x09, 0x5e, 0xa7, 0xb3];

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

fn parse_bytes(value: &Value, key: &str) -> Bytes {
    let hex_value = value[key].as_str().unwrap();
    Bytes::from(hex::decode(&hex_value[2..]).unwrap())
}

#[test]
fn erc20_approve_calldata_matches_frozen_vectors() {
    let data = fixture("operations/erc20_approve_calldata.json");
    let vectors = data["vectors"].as_array().unwrap();

    for vector in vectors {
        let name = vector["name"].as_str().unwrap();
        let token = parse_address(vector, "token");
        let spender = parse_address(vector, "spender");
        let amount = parse_u256(vector, "amount");
        let expected_calldata = parse_bytes(vector, "expected_calldata");
        let expected_calldata_hex = vector["expected_calldata"].as_str().unwrap();

        let call = match build_erc20_approve_call(token, spender, amount) {
            Ok(call) => call,
            Err(error) => panic!(
                "positive vector {name} should build expected calldata {expected_calldata_hex}; got {error:?}"
            ),
        };

        assert_eq!(call.target, token, "{name}: target should be token");
        assert_eq!(call.value, U256::zero(), "{name}: value should be zero");
        assert_eq!(
            call.data, expected_calldata,
            "{name}: calldata should match frozen fixture"
        );
        assert_eq!(
            &call.data[..4],
            APPROVE_SELECTOR.as_slice(),
            "{name}: selector should be approve(address,uint256)"
        );
        assert_eq!(call.data.len(), 68, "{name}: approve calldata length");

        let decoded = decode(
            &[ParamType::Address, ParamType::Uint(256)],
            &call.data[4..],
        )
        .expect("approve calldata arguments should ABI-decode");
        assert_eq!(
            decoded,
            vec![Token::Address(spender), Token::Uint(amount)],
            "{name}: decoded approve args should recover spender and amount"
        );
    }
}

#[test]
fn erc20_approve_calldata_rejects_zero_addresses() {
    let token = "0x1111111111111111111111111111111111111111"
        .parse::<Address>()
        .unwrap();
    let spender = "0x3333333333333333333333333333333333333333"
        .parse::<Address>()
        .unwrap();
    let amount = U256::from(1u64);

    assert!(build_erc20_approve_call(Address::zero(), spender, amount).is_err());
    assert!(build_erc20_approve_call(token, Address::zero(), amount).is_err());
}
