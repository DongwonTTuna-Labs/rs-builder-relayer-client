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
    pub factory: Address,
    pub implementation: Address,
}

pub fn deposit_wallet_contract_config(chain_id: u64) -> Result<DepositWalletContractConfig> {
    match chain_id {
        POLYGON_CHAIN_ID => build_config(
            POLYGON_DEPOSIT_WALLET_FACTORY,
            POLYGON_DEPOSIT_WALLET_IMPLEMENTATION,
        ),
        AMOY_CHAIN_ID => build_config(
            AMOY_DEPOSIT_WALLET_FACTORY,
            AMOY_DEPOSIT_WALLET_IMPLEMENTATION,
        ),
        _ => Err(RelayerError::Other(format!(
            "Deposit wallet contracts are not configured for chain {chain_id}"
        ))),
    }
}

pub(crate) fn deposit_wallet_contract_chain_id(
    config: DepositWalletContractConfig,
) -> Result<u64> {
    if config == deposit_wallet_contract_config(POLYGON_CHAIN_ID)? {
        return Ok(POLYGON_CHAIN_ID);
    }

    if config == deposit_wallet_contract_config(AMOY_CHAIN_ID)? {
        return Ok(AMOY_CHAIN_ID);
    }

    Err(RelayerError::Signing(
        "deposit wallet contract config is not supported for signed batch validation".to_string(),
    ))
}

fn build_config(factory: &str, implementation: &str) -> Result<DepositWalletContractConfig> {
    Ok(DepositWalletContractConfig {
        factory: parse_address(factory)?,
        implementation: parse_address(implementation)?,
    })
}

fn parse_address(value: &str) -> Result<Address> {
    Address::from_str(value).map_err(|e| RelayerError::InvalidAddress(e.to_string()))
}
