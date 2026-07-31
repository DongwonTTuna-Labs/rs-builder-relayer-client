//! Controlled Polymarket builder-relayer fork.
//!
//! The crate root exposes the reviewed `0.2.0` integration surface for
//! deposit-wallet relayer work and keeps legacy Safe/Proxy helpers available as
//! reference compatibility APIs. WALLET submit callers must use the fallible
//! `try_build_wallet_batch_request_with_signature` API or the validated signed
//! batch flow; unchecked submit DTO fields remain crate-private.
//! Production HTTP reads require an owner- and chain-scoped
//! `RelayerReadPermit`; this does not grant submit authority.
//!
//! This crate is not a CLOB order/sign/cancel/post SDK. CLOB trading behavior
//! belongs in the official Polymarket Rust CLOB SDK and the consumer CLOB
//! adapter, not in this relayer crate.
//!
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
pub use deposit_wallet::calldata::{
    polygon_calldata_config, CalldataConfigInput, CalldataSourceRef,
    DepositWalletCalldataConfig, SourcedAddress,
};
#[allow(deprecated)]
pub use deposit_wallet::{
    build_wallet_create_request, build_wallet_nonce_request, deposit_wallet_contract_config,
    derive_deposit_wallet_address, try_build_wallet_batch_request_with_signature,
    AmbiguousCandidate, AmbiguousCandidateReport, DepositWalletCall, DepositWalletContractConfig,
    DepositWalletCreateRequest, DepositWalletDeploymentPolicy, DepositWalletDeploymentStatus,
    DepositWalletDryRunEvidence, DepositWalletReadiness, DepositWalletRelayerClient,
    DepositWalletRelayerUrl, DepositWalletRequestContext, DepositWalletSubmitReceipt,
    DepositWalletTransactionReceipt, DryRunCallSummary, InMemoryMutationIntentStore,
    IntentGatedClient, IntentReconcileOutcome, MutationIntentAuditArtifact, MutationIntentLease,
    MutationIntentRecord, MutationIntentStatus, MutationIntentStore, OwnerMutationRegistry,
    ReconciliationDecision, ReconciliationEvidence, ReconciliationSummary, RelayerKeyAuth,
    RelayerMutationMode, RelayerMutationOperation, RelayerMutationPermit, RelayerPollOutcome,
    RelayerPollPolicy, RelayerReadPermit, RelayerSubmitOutcome, RelayerSubmitResponse,
    RelayerTransactionState, TryBeginOutcome, WalletNonceRequest,
    MUTATION_AUDIT_ARTIFACT_SCHEMA_VERSION,
};
pub use error::{RelayerError, Result};
pub use operations::{
    approve, approve_ctf_for_ctf_exchange, approve_ctf_for_neg_risk_adapter,
    approve_ctf_for_neg_risk_exchange, approve_usdc_for_ctf_exchange,
    approve_usdc_for_neg_risk_exchange, merge_positions, merge_regular, redeem_neg_risk_positions,
    redeem_positions, redeem_regular, set_approval_for_all, split_position, split_regular,
};
pub use types::{RelayerTxType, Transaction, TxResult, TxState};
