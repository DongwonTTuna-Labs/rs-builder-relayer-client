//! Deposit-wallet relayer support.
//!
//! This module contains deposit-wallet-specific building blocks only. It does
//! not reuse the legacy Safe/Proxy execution path for WALLET-CREATE or WALLET
//! request shapes.

pub mod address;
pub mod config;
pub mod http;
pub mod nonce;
pub mod requests;
pub mod signing;
pub mod transaction;
pub mod types;

pub use address::derive_deposit_wallet_address;
pub use config::{
    deposit_wallet_contract_config, DepositWalletContractConfig, AMOY_CHAIN_ID,
    AMOY_DEPOSIT_WALLET_FACTORY, AMOY_DEPOSIT_WALLET_IMPLEMENTATION, POLYGON_CHAIN_ID,
    POLYGON_DEPOSIT_WALLET_FACTORY, POLYGON_DEPOSIT_WALLET_IMPLEMENTATION,
};
pub use http::{
    DepositWalletMutationGate, DepositWalletMutationPermit,
    DepositWalletOwnerSerializationEvidence, DepositWalletPollPolicy, DepositWalletRelayerClient,
    DepositWalletRelayerUrl, DepositWalletTransactionReceipt, RelayerKeyAuth,
};
pub use nonce::{build_wallet_nonce_request, WalletNonceRequest};
#[allow(deprecated)]
pub use requests::{build_wallet_create_request, try_build_wallet_batch_request_with_signature};
#[allow(deprecated)]
pub use signing::{
    build_deposit_wallet_batch_request_from_signed, digest_deposit_wallet_batch,
    recover_deposit_wallet_batch_signer, try_build_deposit_wallet_batch_typed_data,
    validate_deposit_wallet_batch_signature, DepositWalletBatchToSign, SignedDepositWalletBatch,
};
pub use transaction::RelayerTransactionState;
pub use types::{
    DepositWalletBatchRequest, DepositWalletCall, DepositWalletCreateRequest, DepositWalletParams,
    DepositWalletRequestContext, RelayerSubmitResponse, WALLET_CREATE_TRANSACTION_TYPE,
    WALLET_TRANSACTION_TYPE,
};
