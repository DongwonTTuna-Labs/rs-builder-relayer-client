//! Verified deposit-wallet calldata configuration and call builders.
//!
//! This module provides source-backed Polygon contract addresses, approval
//! allowlists, unit-safe pUSD/CTF amounts, PBRSDK-18 approval encoders, and the
//! four PBRSDK-19 verified split/merge/redeem routes. Unsupported routes remain
//! absent from the typed surface.

mod amount;
mod approval;
mod config;
mod ctf;
mod position;
mod summary;

pub use amount::PusdAmount;
pub use approval::{build_ctf_approval_for_all_call, build_pusd_approval_call};
pub use config::{
    polygon_calldata_config, CalldataConfigInput, CalldataSourceRef,
    DepositWalletCalldataConfig, SourcedAddress, POLYGON_CTF, POLYGON_NEG_RISK_ADAPTER,
    POLYGON_NEG_RISK_EXCHANGE, POLYGON_PUSD, POLYGON_STANDARD_EXCHANGE, PUSD_DECIMALS,
};
pub use ctf::{
    build_merge_positions_call, build_neg_risk_redeem_positions_call,
    build_redeem_positions_call, build_split_position_call, CtfRoute,
};
pub use position::CtfPositionAmount;
pub use summary::{summarize_batch_calls, BatchCallSummary, DepositWalletBatchSummary};
