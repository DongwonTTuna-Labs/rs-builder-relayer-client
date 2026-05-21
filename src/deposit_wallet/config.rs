use std::str::FromStr;

use ethers::types::Address;

use crate::error::{RelayerError, Result};

pub const POLYGON_CHAIN_ID: u64 = 137;
pub const AMOY_CHAIN_ID: u64 = 80002;

pub const POLYGON_DEPOSIT_WALLET_FACTORY: &str = "0x00000000000Fb5C9ADea0298D729A0CB3823Cc07";
pub const POLYGON_DEPOSIT_WALLET_IMPLEMENTATION: &str =
    "0x58CA52ebe0DadfdF531Cde7062e76746de4Db1eB";

pub const AMOY_DEPOSIT_WALLET_FACTORY: &str = "0x00000000000Fb5C9ADea0298D729A0CB3823Cc07";
pub const AMOY_DEPOSIT_WALLET_IMPLEMENTATION: &str =
    "0x50a88fE9a441cB4c9c2aD6A2207CE2795C7D7Fbd";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DepositWalletContractConfig {
    pub chain_id: u64,
    pub factory: Address,
    pub implementation: Address,
}

pub fn deposit_wallet_contract_config(chain_id: u64) -> Result<DepositWalletContractConfig> {
    match chain_id {
        POLYGON_CHAIN_ID => build_config(
            chain_id,
            POLYGON_DEPOSIT_WALLET_FACTORY,
            POLYGON_DEPOSIT_WALLET_IMPLEMENTATION,
        ),
        AMOY_CHAIN_ID => build_config(
            chain_id,
            AMOY_DEPOSIT_WALLET_FACTORY,
            AMOY_DEPOSIT_WALLET_IMPLEMENTATION,
        ),
        _ => Err(RelayerError::Other(format!(
            "Deposit wallet contracts are not configured for chain {chain_id}"
        ))),
    }
}

fn build_config(
    chain_id: u64,
    factory: &str,
    implementation: &str,
) -> Result<DepositWalletContractConfig> {
    Ok(DepositWalletContractConfig {
        chain_id,
        factory: parse_address(factory)?,
        implementation: parse_address(implementation)?,
    })
}

fn parse_address(value: &str) -> Result<Address> {
    Address::from_str(value).map_err(|e| RelayerError::InvalidAddress(e.to_string()))
}
