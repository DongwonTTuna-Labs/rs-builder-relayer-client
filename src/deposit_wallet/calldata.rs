use ethers::abi::{encode, Token};
use ethers::types::{Address, Bytes, H256, U256};

use crate::deposit_wallet::DepositWalletCall;
use crate::error::RelayerError;

const ERC20_APPROVE_SELECTOR: [u8; 4] = [0x09, 0x5e, 0xa7, 0xb3];
const CTF_SPLIT_POSITION_SELECTOR: [u8; 4] = [0x72, 0xce, 0x42, 0x75];
const CTF_MERGE_POSITIONS_SELECTOR: [u8; 4] = [0x9e, 0x72, 0x12, 0xad];

pub fn build_erc20_approve_call(
    token: Address,
    spender: Address,
    amount: U256,
) -> Result<DepositWalletCall, RelayerError> {
    if token == Address::zero() {
        return Err(RelayerError::InvalidAddress(
            "ERC20 approve token address cannot be zero".to_string(),
        ));
    }

    if spender == Address::zero() {
        return Err(RelayerError::InvalidAddress(
            "ERC20 approve spender address cannot be zero".to_string(),
        ));
    }

    let encoded_args = encode(&[Token::Address(spender), Token::Uint(amount)]);
    let mut data = Vec::with_capacity(ERC20_APPROVE_SELECTOR.len() + encoded_args.len());
    data.extend_from_slice(&ERC20_APPROVE_SELECTOR);
    data.extend_from_slice(&encoded_args);

    Ok(DepositWalletCall {
        target: token,
        value: U256::zero(),
        data: Bytes::from(data),
    })
}

pub fn build_ctf_split_position_call(
    adapter: Address,
    collateral: Address,
    parent_collection_id: H256,
    condition_id: H256,
    partition: Vec<U256>,
    amount: U256,
) -> Result<DepositWalletCall, RelayerError> {
    if adapter == Address::zero() {
        return Err(RelayerError::InvalidAddress(
            "CTF splitPosition adapter address cannot be zero".to_string(),
        ));
    }

    if collateral == Address::zero() {
        return Err(RelayerError::InvalidAddress(
            "CTF splitPosition collateral address cannot be zero".to_string(),
        ));
    }

    if partition.is_empty() {
        return Err(RelayerError::Abi(
            "CTF splitPosition partition cannot be empty".to_string(),
        ));
    }

    let encoded_args = encode(&[
        Token::Address(collateral),
        Token::FixedBytes(parent_collection_id.as_bytes().to_vec()),
        Token::FixedBytes(condition_id.as_bytes().to_vec()),
        Token::Array(partition.into_iter().map(Token::Uint).collect()),
        Token::Uint(amount),
    ]);
    let mut data = Vec::with_capacity(CTF_SPLIT_POSITION_SELECTOR.len() + encoded_args.len());
    data.extend_from_slice(&CTF_SPLIT_POSITION_SELECTOR);
    data.extend_from_slice(&encoded_args);

    Ok(DepositWalletCall {
        target: adapter,
        value: U256::zero(),
        data: Bytes::from(data),
    })
}

pub fn build_ctf_merge_positions_call(
    adapter: Address,
    collateral: Address,
    parent_collection_id: H256,
    condition_id: H256,
    partition: Vec<U256>,
    amount: U256,
) -> Result<DepositWalletCall, RelayerError> {
    if adapter == Address::zero() {
        return Err(RelayerError::InvalidAddress(
            "CTF mergePositions adapter address cannot be zero".to_string(),
        ));
    }

    if collateral == Address::zero() {
        return Err(RelayerError::InvalidAddress(
            "CTF mergePositions collateral address cannot be zero".to_string(),
        ));
    }

    if partition.is_empty() {
        return Err(RelayerError::Abi(
            "CTF mergePositions partition cannot be empty".to_string(),
        ));
    }

    let encoded_args = encode(&[
        Token::Address(collateral),
        Token::FixedBytes(parent_collection_id.as_bytes().to_vec()),
        Token::FixedBytes(condition_id.as_bytes().to_vec()),
        Token::Array(partition.into_iter().map(Token::Uint).collect()),
        Token::Uint(amount),
    ]);
    let mut data = Vec::with_capacity(CTF_MERGE_POSITIONS_SELECTOR.len() + encoded_args.len());
    data.extend_from_slice(&CTF_MERGE_POSITIONS_SELECTOR);
    data.extend_from_slice(&encoded_args);

    Ok(DepositWalletCall {
        target: adapter,
        value: U256::zero(),
        data: Bytes::from(data),
    })
}
