use ethers::types::{Address, U256};

use crate::deposit_wallet::DepositWalletCall;
use crate::error::RelayerError;

pub fn build_erc20_approve_call(
    token: Address,
    spender: Address,
    amount: U256,
) -> Result<DepositWalletCall, RelayerError> {
    let _ = (token, spender, amount);

    Err(RelayerError::Abi(
        "build_erc20_approve_call is not yet implemented".to_string(),
    ))
}
