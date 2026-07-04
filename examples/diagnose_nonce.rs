use ethers::abi::{encode, Token};
use ethers::types::{Address, H256, U256};
use ethers::utils::keccak256;
use polymarket_relayer::redeem_regular;

fn main() -> anyhow::Result<()> {
    let chain_id = U256::from(137u64);
    let safe_address: Address = "0x2222222222222222222222222222222222222222".parse()?;
    let nonce = U256::from(42u64);
    let redeem = redeem_regular([0x99; 32], &[1, 2]);
    let calldata = hex::decode(redeem.data.trim_start_matches("0x"))?;
    let to: Address = redeem.to.parse()?;

    let domain_separator = safe_domain_separator(chain_id, safe_address);
    let tx_hash = safe_transaction_hash(domain_separator, to, &calldata, nonce);

    println!("Offline Safe nonce diagnostic fixture");
    println!("safe:             {safe_address:?}");
    println!("nonce:            {nonce}");
    println!("domain separator: {domain_separator:?}");
    println!("safe tx hash:     {tx_hash:?}");
    println!("No on-chain nonce or relayer nonce was read.");

    Ok(())
}

fn safe_domain_separator(chain_id: U256, verifying_contract: Address) -> H256 {
    let domain_type_hash = H256::from(keccak256(
        b"EIP712Domain(uint256 chainId,address verifyingContract)",
    ));

    H256::from(keccak256(encode(&[
        Token::FixedBytes(domain_type_hash.as_bytes().to_vec()),
        Token::Uint(chain_id),
        Token::Address(verifying_contract),
    ])))
}

fn safe_transaction_hash(domain_separator: H256, to: Address, data: &[u8], nonce: U256) -> H256 {
    let data_hash = H256::from(keccak256(data));
    let safe_tx_type_hash = H256::from(keccak256(
        b"SafeTx(address to,uint256 value,bytes data,uint8 operation,uint256 safeTxGas,uint256 baseGas,uint256 gasPrice,address gasToken,address refundReceiver,uint256 nonce)",
    ));
    let struct_hash = H256::from(keccak256(encode(&[
        Token::FixedBytes(safe_tx_type_hash.as_bytes().to_vec()),
        Token::Address(to),
        Token::Uint(U256::zero()),
        Token::FixedBytes(data_hash.as_bytes().to_vec()),
        Token::Uint(U256::zero()),
        Token::Uint(U256::zero()),
        Token::Uint(U256::zero()),
        Token::Uint(U256::zero()),
        Token::Address(Address::zero()),
        Token::Address(Address::zero()),
        Token::Uint(nonce),
    ])));

    let mut payload = Vec::with_capacity(66);
    payload.extend_from_slice(&[0x19, 0x01]);
    payload.extend_from_slice(domain_separator.as_bytes());
    payload.extend_from_slice(struct_hash.as_bytes());
    H256::from(keccak256(payload))
}
