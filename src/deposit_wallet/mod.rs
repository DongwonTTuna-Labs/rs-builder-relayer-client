//! Deposit-wallet relayer support.
//!
//! This module contains deposit-wallet-specific building blocks only. It does
//! not reuse the legacy Safe/Proxy execution path for WALLET-CREATE or WALLET
//! request shapes. WALLET submit request DTOs are returned by validated
//! builders, with raw submit fields kept private to the crate.

pub mod address;
pub mod calldata;
pub mod config;
pub mod http;
mod identity;
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
    AmbiguousCandidate, AmbiguousCandidateReport, DepositWalletDeploymentPolicy,
    DepositWalletDeploymentStatus, DepositWalletDryRunEvidence, DepositWalletReadiness,
    DepositWalletRelayerClient, DepositWalletRelayerUrl, DepositWalletSubmitReceipt,
    DepositWalletTransactionReceipt, DryRunCallSummary, InMemoryMutationIntentStore,
    IntentGatedClient, IntentReconcileOutcome, MutationIntentAuditArtifact, MutationIntentLease,
    MutationIntentRecord, MutationIntentStatus, MutationIntentStore, OwnerMutationRegistry,
    ReconciliationDecision, ReconciliationEvidence, ReconciliationSummary, RelayerKeyAuth,
    RelayerMutationMode, RelayerMutationOperation, RelayerMutationPermit, RelayerPollOutcome,
    RelayerPollPolicy, RelayerReadPermit, RelayerSubmitOutcome, TryBeginOutcome,
    MUTATION_AUDIT_ARTIFACT_SCHEMA_VERSION,
};
pub use identity::{
    DepositWalletAddress, DepositWalletIdentityConfig, DepositWalletOwner, IdentityConfigSummary,
    IdentityOverlap, RelayerAuthIdentity,
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
