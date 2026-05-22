pub mod auth;
pub mod builder;
pub mod client;
pub mod contracts;
pub mod deposit_wallet;
pub mod direct;
pub mod error;
pub mod operations;
pub mod types;

// Re-export key types for convenience.
pub use auth::{AuthMethod, BuilderConfig};
pub use client::{RelayClient, TransactionResponseHandle};
pub use direct::{DirectExecutor, DirectTxResult};
#[allow(deprecated)]
pub use deposit_wallet::{
    build_wallet_create_request, build_wallet_nonce_request, deposit_wallet_contract_config,
    derive_deposit_wallet_address, try_build_wallet_batch_request_with_signature,
    DepositWalletCall, DepositWalletContractConfig, DepositWalletCreateRequest,
    DepositWalletRequestContext, RelayerSubmitResponse, RelayerTransactionState,
    WalletNonceRequest,
};
pub use error::{RelayerError, Result};
pub use operations::{
    approve, approve_ctf_for_ctf_exchange, approve_ctf_for_neg_risk_adapter,
    approve_ctf_for_neg_risk_exchange, approve_usdc_for_ctf_exchange,
    approve_usdc_for_neg_risk_exchange, merge_positions, merge_regular, redeem_neg_risk_positions,
    redeem_positions, redeem_regular, set_approval_for_all, split_position, split_regular,
};
pub use types::{RelayerTxType, Transaction, TxResult, TxState};
