//! Deposit-wallet relayer support.
//!
//! This module contains deposit-wallet-specific building blocks only. It does
//! not reuse the legacy Safe/Proxy execution path for WALLET-CREATE or WALLET
//! request shapes.

pub mod address;
pub mod config;
pub mod requests;
pub mod transaction;
pub mod types;

pub use address::derive_deposit_wallet_address;
pub use config::{
    deposit_wallet_contract_config, DepositWalletContractConfig, AMOY_CHAIN_ID,
    AMOY_DEPOSIT_WALLET_FACTORY, AMOY_DEPOSIT_WALLET_IMPLEMENTATION, POLYGON_CHAIN_ID,
    POLYGON_DEPOSIT_WALLET_FACTORY, POLYGON_DEPOSIT_WALLET_IMPLEMENTATION,
};
pub use requests::{build_wallet_batch_request_with_signature, build_wallet_create_request};
pub use transaction::RelayerTransactionState;
pub use types::{
    DepositWalletBatchRequest, DepositWalletCall, DepositWalletCreateRequest, DepositWalletParams,
    DepositWalletRequestContext, RelayerSubmitResponse, WALLET_CREATE_TRANSACTION_TYPE,
    WALLET_TRANSACTION_TYPE,
};
