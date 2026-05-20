use ethers::abi::{encode, Token};
use ethers::types::{Address, H256};
use ethers::utils::keccak256;

use crate::deposit_wallet::DepositWalletContractConfig;
use crate::error::Result;

const ERC1967_CONST1: &str = "cc3735a920a3ca505d382bbc545af43d6000803e6038573d6000fd5b3d6000f3";
const ERC1967_CONST2: &str = "5155f3363d3d373d3d363d7f360894a13ba1a3210667c828492db98dca3e2076";
const ERC1967_PREFIX: u128 = 0x61003d3d8160233d3973;

pub fn derive_deposit_wallet_address(
    owner: Address,
    config: DepositWalletContractConfig,
) -> Result<Address> {
    let mut wallet_id = [0u8; 32];
    wallet_id[12..].copy_from_slice(owner.as_bytes());

    let args = encode(&[
        Token::Address(config.factory),
        Token::FixedBytes(wallet_id.to_vec()),
    ]);
    let salt = H256::from(keccak256(&args));
    let bytecode_hash = init_code_hash_erc1967(config.implementation, &args)?;

    Ok(create2_address(config.factory, salt, bytecode_hash))
}

fn create2_address(deployer: Address, salt: H256, init_code_hash: H256) -> Address {
    let mut bytes = Vec::with_capacity(85);
    bytes.push(0xff);
    bytes.extend_from_slice(deployer.as_bytes());
    bytes.extend_from_slice(salt.as_bytes());
    bytes.extend_from_slice(init_code_hash.as_bytes());

    let hash = keccak256(bytes);
    Address::from_slice(&hash[12..])
}

fn init_code_hash_erc1967(implementation: Address, args: &[u8]) -> Result<H256> {
    let n = args.len() as u128;
    let combined = ERC1967_PREFIX + (n << 56);
    let combined_bytes = combined.to_be_bytes();

    let mut init_code = Vec::with_capacity(10 + 20 + 2 + 32 + 32 + args.len());
    init_code.extend_from_slice(&combined_bytes[6..]);
    init_code.extend_from_slice(implementation.as_bytes());
    init_code.extend_from_slice(&[0x60, 0x09]);
    init_code.extend_from_slice(&hex::decode(ERC1967_CONST2)?);
    init_code.extend_from_slice(&hex::decode(ERC1967_CONST1)?);
    init_code.extend_from_slice(args);

    Ok(H256::from(keccak256(init_code)))
}
