use std::fmt;

use ethers::types::Address;
use ethers::utils::to_checksum;
use serde::Serialize;

use super::address::derive_deposit_wallet_address;
use super::config::DepositWalletContractConfig;
use super::types::DepositWalletRequestContext;
use crate::error::{RelayerError, Result};

const IDENTITY_REDACTION_MARKER: &str =
    "full identity addresses are intentionally redacted";

/// The identity authenticated by the relayer API key. This is neither the
/// WALLET signer nor the deposit wallet.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct RelayerAuthIdentity(Address);

impl RelayerAuthIdentity {
    pub fn new(address: Address) -> Self {
        Self(address)
    }

    pub fn address(&self) -> Address {
        self.0
    }
}

impl fmt::Debug for RelayerAuthIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("RelayerAuthIdentity")
            .field(&redacted_address(self.0))
            .finish()
    }
}

/// The EOA that owns the deposit wallet and signs WALLET batches.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct DepositWalletOwner(Address);

impl DepositWalletOwner {
    pub fn new(address: Address) -> Self {
        Self(address)
    }

    pub fn address(&self) -> Address {
        self.0
    }
}

impl fmt::Debug for DepositWalletOwner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("DepositWalletOwner")
            .field(&redacted_address(self.0))
            .finish()
    }
}

/// The deployed deposit-wallet contract address, also used as the CLOB funder.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct DepositWalletAddress(Address);

impl DepositWalletAddress {
    pub fn new(address: Address) -> Self {
        Self(address)
    }

    pub fn address(&self) -> Address {
        self.0
    }
}

impl fmt::Debug for DepositWalletAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("DepositWalletAddress")
            .field(&redacted_address(self.0))
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdentityOverlap {
    AuthEqualsOwner,
    AuthEqualsDepositWallet,
    OwnerEqualsDepositWallet,
}

impl IdentityOverlap {
    pub fn as_key(&self) -> &'static str {
        match self {
            Self::AuthEqualsOwner => "auth_equals_owner",
            Self::AuthEqualsDepositWallet => "auth_equals_deposit_wallet",
            Self::OwnerEqualsDepositWallet => "owner_equals_deposit_wallet",
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct DepositWalletIdentityConfig {
    auth_identity: RelayerAuthIdentity,
    owner: DepositWalletOwner,
    deposit_wallet: DepositWalletAddress,
}

impl DepositWalletIdentityConfig {
    pub fn try_new(
        auth_identity: RelayerAuthIdentity,
        owner: DepositWalletOwner,
        deposit_wallet: DepositWalletAddress,
        contract_config: DepositWalletContractConfig,
    ) -> Result<Self> {
        if auth_identity.address() == Address::zero()
            || owner.address() == Address::zero()
            || deposit_wallet.address() == Address::zero()
        {
            return Err(RelayerError::InvalidAddress(
                "deposit wallet identity addresses must be non-zero".to_string(),
            ));
        }

        let derived_deposit_wallet =
            derive_deposit_wallet_address(owner.address(), contract_config)?;
        if deposit_wallet.address() != derived_deposit_wallet {
            return Err(RelayerError::Signing(
                "deposit wallet address did not match the owner-derived address".to_string(),
            ));
        }

        Ok(Self {
            auth_identity,
            owner,
            deposit_wallet,
        })
    }

    pub fn auth_identity(&self) -> RelayerAuthIdentity {
        self.auth_identity
    }

    pub fn owner(&self) -> DepositWalletOwner {
        self.owner
    }

    pub fn deposit_wallet(&self) -> DepositWalletAddress {
        self.deposit_wallet
    }

    /// Reports every equal identity pair in a stable order. Policy remains the
    /// responsibility of the caller; overlap is not rejected here.
    pub fn overlaps(&self) -> Vec<IdentityOverlap> {
        let mut overlaps = Vec::new();
        if self.auth_identity.address() == self.owner.address() {
            overlaps.push(IdentityOverlap::AuthEqualsOwner);
        }
        if self.auth_identity.address() == self.deposit_wallet.address() {
            overlaps.push(IdentityOverlap::AuthEqualsDepositWallet);
        }
        if self.owner.address() == self.deposit_wallet.address() {
            overlaps.push(IdentityOverlap::OwnerEqualsDepositWallet);
        }
        overlaps
    }

    pub fn summary(&self) -> IdentityConfigSummary {
        IdentityConfigSummary {
            auth_identity: redacted_address(self.auth_identity.address()),
            owner: redacted_address(self.owner.address()),
            deposit_wallet: redacted_address(self.deposit_wallet.address()),
            overlaps: self
                .overlaps()
                .iter()
                .map(|overlap| overlap.as_key().to_string())
                .collect(),
            redaction: IDENTITY_REDACTION_MARKER.to_string(),
        }
    }

    pub fn request_context(&self) -> DepositWalletRequestContext {
        DepositWalletRequestContext {
            owner_address: self.owner.address(),
            deposit_wallet_address: self.deposit_wallet.address(),
        }
    }
}

impl fmt::Debug for DepositWalletIdentityConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DepositWalletIdentityConfig")
            .field(
                "auth_identity",
                &redacted_address(self.auth_identity.address()),
            )
            .field("owner", &redacted_address(self.owner.address()))
            .field(
                "deposit_wallet",
                &redacted_address(self.deposit_wallet.address()),
            )
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct IdentityConfigSummary {
    auth_identity: String,
    owner: String,
    deposit_wallet: String,
    overlaps: Vec<String>,
    redaction: String,
}

/// Pins the summary field shape in every build configuration. Adding a field
/// makes this fail with E0027 even in non-test artifacts.
#[allow(dead_code)]
fn _identity_config_summary_field_shape_is_pinned(summary: IdentityConfigSummary) {
    let IdentityConfigSummary {
        auth_identity: _,
        owner: _,
        deposit_wallet: _,
        overlaps: _,
        redaction: _,
    } = summary;
}

impl IdentityConfigSummary {
    pub fn auth_identity(&self) -> &str {
        &self.auth_identity
    }

    pub fn owner(&self) -> &str {
        &self.owner
    }

    pub fn deposit_wallet(&self) -> &str {
        &self.deposit_wallet
    }

    pub fn overlaps(&self) -> &[String] {
        &self.overlaps
    }

    pub fn redaction(&self) -> &str {
        &self.redaction
    }
}

fn redacted_address(address: Address) -> String {
    let checksum = to_checksum(&address, None);
    format!("{}...{}", &checksum[..6], &checksum[38..])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deposit_wallet::deposit_wallet_contract_config;

    fn address(value: u64) -> Address {
        Address::from_low_u64_be(value)
    }

    fn valid_config(auth_address: Address) -> DepositWalletIdentityConfig {
        let contract_config = deposit_wallet_contract_config(137).unwrap();
        let owner_address = address(1);
        let deposit_wallet_address =
            derive_deposit_wallet_address(owner_address, contract_config).unwrap();

        DepositWalletIdentityConfig::try_new(
            RelayerAuthIdentity::new(auth_address),
            DepositWalletOwner::new(owner_address),
            DepositWalletAddress::new(deposit_wallet_address),
            contract_config,
        )
        .unwrap()
    }

    #[test]
    fn identity_config_validates_derivation_and_builds_legacy_context() {
        let config = valid_config(address(2));
        let context = config.request_context();
        let auth = crate::deposit_wallet::RelayerKeyAuth::from_identity(
            String::from("test-api-key"),
            config.auth_identity(),
        )
        .unwrap();

        assert_eq!(config.auth_identity().address(), address(2));
        assert_eq!(auth.api_key_address(), address(2));
        assert_eq!(config.owner().address(), address(1));
        assert_eq!(config.deposit_wallet().address(), context.deposit_wallet_address);
        assert_eq!(context.owner_address, address(1));
        assert!(config.overlaps().is_empty());
    }

    #[test]
    fn zero_addresses_are_rejected_before_owner_derivation_validation() {
        let contract_config = deposit_wallet_contract_config(137).unwrap();
        let owner_address = address(1);
        let deposit_wallet_address =
            derive_deposit_wallet_address(owner_address, contract_config).unwrap();
        let cases = [
            (
                RelayerAuthIdentity::new(Address::zero()),
                DepositWalletOwner::new(owner_address),
                DepositWalletAddress::new(deposit_wallet_address),
            ),
            (
                RelayerAuthIdentity::new(address(2)),
                DepositWalletOwner::new(Address::zero()),
                DepositWalletAddress::new(deposit_wallet_address),
            ),
            (
                RelayerAuthIdentity::new(address(2)),
                DepositWalletOwner::new(owner_address),
                DepositWalletAddress::new(Address::zero()),
            ),
        ];

        for (auth_identity, owner, deposit_wallet) in cases {
            let error = DepositWalletIdentityConfig::try_new(
                auth_identity,
                owner,
                deposit_wallet,
                contract_config,
            )
            .unwrap_err();
            assert!(matches!(error, RelayerError::InvalidAddress(_)));
        }
    }

    #[test]
    fn owner_derived_wallet_mismatch_is_a_signing_error() {
        let error = DepositWalletIdentityConfig::try_new(
            RelayerAuthIdentity::new(address(2)),
            DepositWalletOwner::new(address(1)),
            DepositWalletAddress::new(address(3)),
            deposit_wallet_contract_config(137).unwrap(),
        )
        .unwrap_err();

        assert!(matches!(error, RelayerError::Signing(_)));
    }

    #[test]
    fn configured_overlap_is_observed_without_policy_rejection() {
        let auth_equals_owner = valid_config(address(1));
        assert_eq!(
            auth_equals_owner.overlaps(),
            vec![IdentityOverlap::AuthEqualsOwner]
        );

        let contract_config = deposit_wallet_contract_config(137).unwrap();
        let owner_address = address(1);
        let deposit_wallet_address =
            derive_deposit_wallet_address(owner_address, contract_config).unwrap();
        let auth_equals_deposit_wallet = valid_config(deposit_wallet_address);
        assert_eq!(
            auth_equals_deposit_wallet.overlaps(),
            vec![IdentityOverlap::AuthEqualsDepositWallet]
        );

        let same_address = address(4);
        let all_equal = DepositWalletIdentityConfig {
            auth_identity: RelayerAuthIdentity::new(same_address),
            owner: DepositWalletOwner::new(same_address),
            deposit_wallet: DepositWalletAddress::new(same_address),
        };
        assert_eq!(
            all_equal.overlaps(),
            vec![
                IdentityOverlap::AuthEqualsOwner,
                IdentityOverlap::AuthEqualsDepositWallet,
                IdentityOverlap::OwnerEqualsDepositWallet,
            ]
        );
        assert_eq!(
            all_equal.summary().overlaps(),
            &[
                "auth_equals_owner".to_string(),
                "auth_equals_deposit_wallet".to_string(),
                "owner_equals_deposit_wallet".to_string(),
            ]
        );
    }

    #[test]
    fn identity_config_summary_field_set_is_exhaustive() {
        let config = valid_config(address(7));

        // Adding a field must fail compilation with E0027; do not add a rest pattern here.
        let IdentityConfigSummary {
            auth_identity: _,
            owner: _,
            deposit_wallet: _,
            overlaps: _,
            redaction: _,
        } = config.summary();
    }

    #[test]
    fn summary_and_debug_output_contain_only_redacted_addresses() {
        let auth_address = Address::from_slice(&[0x11; 20]);
        let owner_address = Address::from_slice(&[0x22; 20]);
        let deposit_wallet_address = Address::from_slice(&[0x33; 20]);
        let config = DepositWalletIdentityConfig {
            auth_identity: RelayerAuthIdentity::new(auth_address),
            owner: DepositWalletOwner::new(owner_address),
            deposit_wallet: DepositWalletAddress::new(deposit_wallet_address),
        };
        let summary = config.summary();

        assert_eq!(summary.auth_identity(), "0x1111...1111");
        assert_eq!(summary.owner(), "0x2222...2222");
        assert_eq!(summary.deposit_wallet(), "0x3333...3333");
        assert!(summary.overlaps().is_empty());
        assert_eq!(summary.redaction(), IDENTITY_REDACTION_MARKER);

        let auth_debug = format!("{:?}", config.auth_identity());
        let auth_debug_alternate = format!("{:#?}", config.auth_identity());
        let owner_debug = format!("{:?}", config.owner());
        let owner_debug_alternate = format!("{:#?}", config.owner());
        let deposit_wallet_debug = format!("{:?}", config.deposit_wallet());
        let deposit_wallet_debug_alternate = format!("{:#?}", config.deposit_wallet());
        let config_debug = format!("{config:?}");
        let config_debug_alternate = format!("{config:#?}");
        assert_eq!(
            auth_debug,
            r#"RelayerAuthIdentity("0x1111...1111")"#
        );
        assert_eq!(
            auth_debug_alternate,
            r#"RelayerAuthIdentity(
    "0x1111...1111",
)"#
        );
        assert_eq!(
            owner_debug,
            r#"DepositWalletOwner("0x2222...2222")"#
        );
        assert_eq!(
            owner_debug_alternate,
            r#"DepositWalletOwner(
    "0x2222...2222",
)"#
        );
        assert_eq!(
            deposit_wallet_debug,
            r#"DepositWalletAddress("0x3333...3333")"#
        );
        assert_eq!(
            deposit_wallet_debug_alternate,
            r#"DepositWalletAddress(
    "0x3333...3333",
)"#
        );
        assert_eq!(
            config_debug,
            r#"DepositWalletIdentityConfig { auth_identity: "0x1111...1111", owner: "0x2222...2222", deposit_wallet: "0x3333...3333" }"#
        );
        assert_eq!(
            config_debug_alternate,
            r#"DepositWalletIdentityConfig {
    auth_identity: "0x1111...1111",
    owner: "0x2222...2222",
    deposit_wallet: "0x3333...3333",
}"#
        );

        let summary_json = serde_json::to_string(&summary).unwrap();
        assert_eq!(
            summary_json,
            r#"{"auth_identity":"0x1111...1111","owner":"0x2222...2222","deposit_wallet":"0x3333...3333","overlaps":[],"redaction":"full identity addresses are intentionally redacted"}"#
        );

        let serialized = serde_json::to_value(&summary).unwrap();
        let summary_debug = format!("{summary:?}");
        let summary_debug_alternate = format!("{summary:#?}");
        assert_eq!(
            summary_debug,
            r#"IdentityConfigSummary { auth_identity: "0x1111...1111", owner: "0x2222...2222", deposit_wallet: "0x3333...3333", overlaps: [], redaction: "full identity addresses are intentionally redacted" }"#
        );
        assert_eq!(
            summary_debug_alternate,
            r#"IdentityConfigSummary {
    auth_identity: "0x1111...1111",
    owner: "0x2222...2222",
    deposit_wallet: "0x3333...3333",
    overlaps: [],
    redaction: "full identity addresses are intentionally redacted",
}"#
        );
        let object = serialized.as_object().unwrap();
        assert_eq!(object.len(), 5);
        assert_eq!(object["auth_identity"], "0x1111...1111");
        assert_eq!(object["owner"], "0x2222...2222");
        assert_eq!(object["deposit_wallet"], "0x3333...3333");
        assert_eq!(object["overlaps"], serde_json::json!([]));
        assert_eq!(object["redaction"], IDENTITY_REDACTION_MARKER);

        let inspected_outputs = [
            summary_json.as_str(),
            summary_debug.as_str(),
            summary_debug_alternate.as_str(),
            auth_debug.as_str(),
            auth_debug_alternate.as_str(),
            owner_debug.as_str(),
            owner_debug_alternate.as_str(),
            deposit_wallet_debug.as_str(),
            deposit_wallet_debug_alternate.as_str(),
            config_debug.as_str(),
            config_debug_alternate.as_str(),
        ];
        for forbidden in [
            "0x1111111111111111111111111111111111111111",
            "0x2222222222222222222222222222222222222222",
            "0x3333333333333333333333333333333333333333",
            "e2c07404b8c1df4c46226425cac68c28d27a766bbddce62309f36724839b22c0",
            "efda2c2822100aaf94fb77c3765831ce37fc3c02cbc11603dd6ffa9c0d25ec55",
            "2ab0a4443bbea3fbe4d0e1503d11ff1367842fb0c8b28a5c8550f27599a40751",
            "3e52d38afc818492f352a9e01ab9de98dcaab3d6e46c6a62367c4e9ce56d0187",
            "37d95e0aa71e34defa88b4c43498bc8b90207e31ad0ef4aa6f5bea78bd25a1ab",
            "fd05d543fd7c68c3811c333778ec2ca116a8e03edfa4deeee24117234f64a12c",
        ] {
            for output in &inspected_outputs {
                assert!(!output.contains(forbidden));
            }
        }
    }

    #[test]
    fn overlap_keys_are_stable() {
        assert_eq!(
            IdentityOverlap::AuthEqualsOwner.as_key(),
            "auth_equals_owner"
        );
        assert_eq!(
            IdentityOverlap::AuthEqualsDepositWallet.as_key(),
            "auth_equals_deposit_wallet"
        );
        assert_eq!(
            IdentityOverlap::OwnerEqualsDepositWallet.as_key(),
            "owner_equals_deposit_wallet"
        );
    }
}
