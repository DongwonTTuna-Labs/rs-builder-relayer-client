use ethers::types::U256;

use crate::deposit_wallet::{
    DepositWalletBatchRequest, DepositWalletCall, DepositWalletContractConfig,
    DepositWalletCreateRequest, DepositWalletParams, DepositWalletRequestContext,
    WALLET_CREATE_TRANSACTION_TYPE, WALLET_TRANSACTION_TYPE,
};

pub fn build_wallet_create_request(
    owner_address: ethers::types::Address,
    config: DepositWalletContractConfig,
) -> DepositWalletCreateRequest {
    DepositWalletCreateRequest {
        tx_type: WALLET_CREATE_TRANSACTION_TYPE.to_string(),
        from_address: owner_address,
        to: config.factory,
    }
}

pub fn build_wallet_batch_request_with_signature(
    ctx: DepositWalletRequestContext,
    config: DepositWalletContractConfig,
    nonce: U256,
    deadline: U256,
    calls: Vec<DepositWalletCall>,
    signature: String,
) -> DepositWalletBatchRequest {
    DepositWalletBatchRequest {
        tx_type: WALLET_TRANSACTION_TYPE.to_string(),
        from_address: ctx.owner_address,
        to: config.factory,
        nonce,
        signature,
        deposit_wallet_params: DepositWalletParams {
            deposit_wallet: ctx.deposit_wallet_address,
            deadline,
            calls,
        },
    }
}
