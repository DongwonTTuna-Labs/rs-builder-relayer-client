//! Verified deposit-wallet calldata configuration and approval builders.
//!
//! This module provides source-backed Polygon contract addresses, approval
//! allowlists, a unit-safe pUSD amount, and the PBRSDK-18 pUSD/CTF approval
//! encoders. Split/merge/redeem and adapter routing remain later work.

mod amount;
mod approval;
mod config;

pub use amount::PusdAmount;
pub use approval::{build_ctf_approval_for_all_call, build_pusd_approval_call};
pub use config::{
    polygon_calldata_config, CalldataConfigInput, CalldataSourceRef,
    DepositWalletCalldataConfig, SourcedAddress, POLYGON_CTF, POLYGON_NEG_RISK_EXCHANGE,
    POLYGON_PUSD, POLYGON_STANDARD_EXCHANGE, PUSD_DECIMALS,
};
