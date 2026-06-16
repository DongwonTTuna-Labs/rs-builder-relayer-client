use ethers::abi::{encode, Token};
use ethers::types::{Address, Bytes, U256};

use crate::deposit_wallet::DepositWalletCall;
use crate::error::RelayerError;

const ERC20_APPROVE_SELECTOR: [u8; 4] = [0x09, 0x5e, 0xa7, 0xb3];

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
