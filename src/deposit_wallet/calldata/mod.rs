//! Verified configuration for deposit-wallet calldata builders.
//!
//! This module provides source-backed Polygon contract addresses and approval
//! allowlists. Calldata encoding belongs to later PBRSDK-18/PBRSDK-19 work.

mod config;

pub use config::{
    polygon_calldata_config, CalldataConfigInput, CalldataSourceRef,
    DepositWalletCalldataConfig, SourcedAddress, POLYGON_CTF, POLYGON_NEG_RISK_EXCHANGE,
    POLYGON_PUSD, POLYGON_STANDARD_EXCHANGE, PUSD_DECIMALS,
};
