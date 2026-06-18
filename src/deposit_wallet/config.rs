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

pub const POLYGON_PUSD_COLLATERAL_TOKEN: &str = "0xC011a7E12a19f7B1f670d46F03B03f3342E82DFB";
pub const POLYGON_CONDITIONAL_TOKENS: &str = "0x4D97DCd97eC945f40cF65F87097ACe5EA0476045";
pub const POLYGON_CTF_COLLATERAL_ADAPTER: &str = "0xAdA100Db00Ca00073811820692005400218FcE1f";
pub const POLYGON_NEG_RISK_CTF_COLLATERAL_ADAPTER: &str =
    "0xadA2005600Dec949baf300f4C6120000bDB6eAab";

pub const AMOY_PUSD_COLLATERAL_TOKEN: &str = "0xC011a7E12a19f7B1f670d46F03B03f3342E82DFB";
pub const AMOY_CONDITIONAL_TOKENS: &str = "0x69308FB512518e39F9b16112fA8d994F4e2Bf8bB";
pub const AMOY_CTF_COLLATERAL_ADAPTER: &str = "0xADa100874d00e3331D00F2007a9c336a65009718";
pub const AMOY_NEG_RISK_CTF_COLLATERAL_ADAPTER: &str =
    "0xAdA200001000ef00D07553cEE7006808F895c6F1";

pub const PUSD_COLLATERAL_DECIMALS: u8 = 6;
pub const CONDITIONAL_TOKEN_DECIMALS: u8 = 6;

const RETRIEVED_ON: &str = "2026-06-16";

pub const POLYMARKET_CONTRACTS_DOCS_PROVENANCE: DepositWalletConfigProvenance =
    DepositWalletConfigProvenance {
        source: "Polymarket docs: Contracts",
        url: "https://docs.polymarket.com/resources/contracts",
        reference: "retrieved 2026-06-16; page lists Polygon mainnet chain 137 contract addresses",
        retrieved_on: RETRIEVED_ON,
    };

pub const POLYMARKET_CTF_EXCHANGE_V2_AMOY_PROVENANCE: DepositWalletConfigProvenance =
    DepositWalletConfigProvenance {
        source: "Polymarket/ctf-exchange-v2 README",
        url: "https://github.com/Polymarket/ctf-exchange-v2/blob/ccc0596074f4dfd62c944fbca4de252893b82b4b/README.md",
        reference: "commit ccc0596074f4dfd62c944fbca4de252893b82b4b; Amoy deployed contracts table",
        retrieved_on: RETRIEVED_ON,
    };

pub const POLYMARKET_CLOB_CLIENT_V2_CONFIG_PROVENANCE: DepositWalletConfigProvenance =
    DepositWalletConfigProvenance {
        source: "Polymarket/clob-client-v2 config.ts",
        url: "https://github.com/Polymarket/clob-client-v2/blob/d28dacdaed9e6ba29c013de588113fad3a20c4f2/src/config.ts",
        reference: "commit d28dacdaed9e6ba29c013de588113fad3a20c4f2; AMOY/MATIC contracts and token decimals",
        retrieved_on: RETRIEVED_ON,
    };

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DepositWalletContractConfig {
    pub factory: Address,
    pub implementation: Address,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DepositWalletConfigProvenance {
    pub source: &'static str,
    pub url: &'static str,
    pub reference: &'static str,
    pub retrieved_on: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DepositWalletVerifiedAddress {
    pub address: Address,
    pub provenance: DepositWalletConfigProvenance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DepositWalletRequiredAddress {
    pub field: &'static str,
    pub reason: &'static str,
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepositWalletAddressConfig {
    Verified(DepositWalletVerifiedAddress),
    RequiredInput(DepositWalletRequiredAddress),
}

impl DepositWalletAddressConfig {
    pub const fn required_input(field: &'static str, reason: &'static str) -> Self {
        Self::RequiredInput(DepositWalletRequiredAddress { field, reason })
    }

    pub fn address(&self) -> Option<Address> {
        match self {
            Self::Verified(address) => Some(address.address),
            Self::RequiredInput(_) => None,
        }
    }

    pub fn require_address(&self) -> Result<Address> {
        match self {
            Self::Verified(address) => Ok(address.address),
            Self::RequiredInput(required) => Err(RelayerError::Other(format!(
                "Deposit-wallet config field {} requires verified address input: {}",
                required.field, required.reason
            ))),
        }
    }

    pub fn provenance(&self) -> Option<DepositWalletConfigProvenance> {
        match self {
            Self::Verified(address) => Some(address.provenance),
            Self::RequiredInput(_) => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DepositWalletTokenUnitConfig {
    pub decimals: u8,
    pub provenance: DepositWalletConfigProvenance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DepositWalletCalldataConfig {
    pub chain_id: u64,
    pub collateral_token: DepositWalletAddressConfig,
    pub conditional_tokens: DepositWalletAddressConfig,
    pub ctf_collateral_adapter: DepositWalletAddressConfig,
    pub neg_risk_ctf_collateral_adapter: DepositWalletAddressConfig,
    pub collateral_token_unit: DepositWalletTokenUnitConfig,
    pub conditional_token_unit: DepositWalletTokenUnitConfig,
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

pub fn deposit_wallet_calldata_config(chain_id: u64) -> Result<DepositWalletCalldataConfig> {
    match chain_id {
        POLYGON_CHAIN_ID => build_calldata_config(DepositWalletCalldataConfigInput {
            chain_id,
            collateral_token: POLYGON_PUSD_COLLATERAL_TOKEN,
            collateral_token_provenance: POLYMARKET_CONTRACTS_DOCS_PROVENANCE,
            conditional_tokens: POLYGON_CONDITIONAL_TOKENS,
            conditional_tokens_provenance: POLYMARKET_CONTRACTS_DOCS_PROVENANCE,
            ctf_collateral_adapter: POLYGON_CTF_COLLATERAL_ADAPTER,
            ctf_collateral_adapter_provenance: POLYMARKET_CONTRACTS_DOCS_PROVENANCE,
            neg_risk_ctf_collateral_adapter: POLYGON_NEG_RISK_CTF_COLLATERAL_ADAPTER,
            neg_risk_ctf_collateral_adapter_provenance: POLYMARKET_CONTRACTS_DOCS_PROVENANCE,
        }),
        AMOY_CHAIN_ID => build_calldata_config(DepositWalletCalldataConfigInput {
            chain_id,
            collateral_token: AMOY_PUSD_COLLATERAL_TOKEN,
            collateral_token_provenance: POLYMARKET_CTF_EXCHANGE_V2_AMOY_PROVENANCE,
            conditional_tokens: AMOY_CONDITIONAL_TOKENS,
            conditional_tokens_provenance: POLYMARKET_CLOB_CLIENT_V2_CONFIG_PROVENANCE,
            ctf_collateral_adapter: AMOY_CTF_COLLATERAL_ADAPTER,
            ctf_collateral_adapter_provenance: POLYMARKET_CTF_EXCHANGE_V2_AMOY_PROVENANCE,
            neg_risk_ctf_collateral_adapter: AMOY_NEG_RISK_CTF_COLLATERAL_ADAPTER,
            neg_risk_ctf_collateral_adapter_provenance: POLYMARKET_CTF_EXCHANGE_V2_AMOY_PROVENANCE,
        }),
        _ => Err(RelayerError::Other(format!(
            "Deposit wallet calldata contracts are not configured for chain {chain_id}"
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

struct DepositWalletCalldataConfigInput {
    chain_id: u64,
    collateral_token: &'static str,
    collateral_token_provenance: DepositWalletConfigProvenance,
    conditional_tokens: &'static str,
    conditional_tokens_provenance: DepositWalletConfigProvenance,
    ctf_collateral_adapter: &'static str,
    ctf_collateral_adapter_provenance: DepositWalletConfigProvenance,
    neg_risk_ctf_collateral_adapter: &'static str,
    neg_risk_ctf_collateral_adapter_provenance: DepositWalletConfigProvenance,
}

fn build_calldata_config(
    input: DepositWalletCalldataConfigInput,
) -> Result<DepositWalletCalldataConfig> {
    Ok(DepositWalletCalldataConfig {
        chain_id: input.chain_id,
        collateral_token: verified_address(
            input.collateral_token,
            input.collateral_token_provenance,
        )?,
        conditional_tokens: verified_address(
            input.conditional_tokens,
            input.conditional_tokens_provenance,
        )?,
        ctf_collateral_adapter: verified_address(
            input.ctf_collateral_adapter,
            input.ctf_collateral_adapter_provenance,
        )?,
        neg_risk_ctf_collateral_adapter: verified_address(
            input.neg_risk_ctf_collateral_adapter,
            input.neg_risk_ctf_collateral_adapter_provenance,
        )?,
        collateral_token_unit: DepositWalletTokenUnitConfig {
            decimals: PUSD_COLLATERAL_DECIMALS,
            provenance: POLYMARKET_CLOB_CLIENT_V2_CONFIG_PROVENANCE,
        },
        conditional_token_unit: DepositWalletTokenUnitConfig {
            decimals: CONDITIONAL_TOKEN_DECIMALS,
            provenance: POLYMARKET_CLOB_CLIENT_V2_CONFIG_PROVENANCE,
        },
    })
}

fn verified_address(
    value: &str,
    provenance: DepositWalletConfigProvenance,
) -> Result<DepositWalletAddressConfig> {
    Ok(DepositWalletAddressConfig::Verified(
        DepositWalletVerifiedAddress {
            address: parse_address(value)?,
            provenance,
        },
    ))
}

fn parse_address(value: &str) -> Result<Address> {
    Address::from_str(value).map_err(|e| RelayerError::InvalidAddress(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn address(value: &str) -> Address {
        parse_address(value).unwrap()
    }

    fn assert_verified_address(
        address_config: DepositWalletAddressConfig,
        expected: &str,
        expected_provenance: DepositWalletConfigProvenance,
    ) {
        assert_eq!(address_config.address(), Some(address(expected)));
        assert_eq!(address_config.require_address().unwrap(), address(expected));
        assert_eq!(address_config.provenance(), Some(expected_provenance));
    }

    #[test]
    fn polygon_calldata_config_returns_validated_adapter_and_token_addresses() {
        let config = deposit_wallet_calldata_config(POLYGON_CHAIN_ID).unwrap();

        assert_eq!(config.chain_id, POLYGON_CHAIN_ID);
        assert_verified_address(
            config.collateral_token,
            POLYGON_PUSD_COLLATERAL_TOKEN,
            POLYMARKET_CONTRACTS_DOCS_PROVENANCE,
        );
        assert_verified_address(
            config.conditional_tokens,
            POLYGON_CONDITIONAL_TOKENS,
            POLYMARKET_CONTRACTS_DOCS_PROVENANCE,
        );
        assert_verified_address(
            config.ctf_collateral_adapter,
            POLYGON_CTF_COLLATERAL_ADAPTER,
            POLYMARKET_CONTRACTS_DOCS_PROVENANCE,
        );
        assert_verified_address(
            config.neg_risk_ctf_collateral_adapter,
            POLYGON_NEG_RISK_CTF_COLLATERAL_ADAPTER,
            POLYMARKET_CONTRACTS_DOCS_PROVENANCE,
        );
        assert_eq!(config.collateral_token_unit.decimals, PUSD_COLLATERAL_DECIMALS);
        assert_eq!(config.conditional_token_unit.decimals, CONDITIONAL_TOKEN_DECIMALS);
    }

    #[test]
    fn amoy_calldata_config_returns_validated_adapter_and_token_addresses() {
        let config = deposit_wallet_calldata_config(AMOY_CHAIN_ID).unwrap();

        assert_eq!(config.chain_id, AMOY_CHAIN_ID);
        assert_verified_address(
            config.collateral_token,
            AMOY_PUSD_COLLATERAL_TOKEN,
            POLYMARKET_CTF_EXCHANGE_V2_AMOY_PROVENANCE,
        );
        assert_verified_address(
            config.conditional_tokens,
            AMOY_CONDITIONAL_TOKENS,
            POLYMARKET_CLOB_CLIENT_V2_CONFIG_PROVENANCE,
        );
        assert_verified_address(
            config.ctf_collateral_adapter,
            AMOY_CTF_COLLATERAL_ADAPTER,
            POLYMARKET_CTF_EXCHANGE_V2_AMOY_PROVENANCE,
        );
        assert_verified_address(
            config.neg_risk_ctf_collateral_adapter,
            AMOY_NEG_RISK_CTF_COLLATERAL_ADAPTER,
            POLYMARKET_CTF_EXCHANGE_V2_AMOY_PROVENANCE,
        );
        assert_eq!(config.collateral_token_unit.decimals, PUSD_COLLATERAL_DECIMALS);
        assert_eq!(config.conditional_token_unit.decimals, CONDITIONAL_TOKEN_DECIMALS);
    }

    #[test]
    fn required_input_address_cannot_be_mistaken_for_verified_venue_truth() {
        let required = DepositWalletAddressConfig::required_input(
            "ctf_collateral_adapter",
            "official source did not publish this chain address",
        );

        assert_eq!(required.address(), None);
        assert_eq!(required.provenance(), None);
        match required.require_address() {
            Err(RelayerError::Other(message)) => {
                assert!(message.contains("ctf_collateral_adapter"));
                assert!(message.contains("requires verified address input"));
            }
            other => panic!("expected typed required-input error, got {other:?}"),
        }
    }

    #[test]
    fn unsupported_calldata_config_chain_is_not_silent_fallback() {
        match deposit_wallet_calldata_config(1) {
            Err(RelayerError::Other(message)) => {
                assert!(message.contains("not configured for chain 1"));
            }
            other => panic!("expected unsupported chain error, got {other:?}"),
        }
    }
}
