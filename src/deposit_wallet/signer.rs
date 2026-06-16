use std::{env, fmt, sync::Arc};

use ethers::signers::{LocalWallet, Signer};
use ethers::types::Address;
use ethers::utils::to_checksum;
use secrecy::{ExposeSecret, SecretString};

use crate::error::{RelayerError, Result};

pub const POLYMARKET_OWNER_PRIVATE_KEY_ENV: &str = "POLYMARKET_OWNER_PRIVATE_KEY";

#[derive(Clone)]
pub struct DepositWalletOwnerSigner {
    private_key: Arc<SecretString>,
    owner: DepositWalletOwnerAddress,
}

impl DepositWalletOwnerSigner {
    pub fn new(private_key: impl Into<String>) -> Result<Self> {
        let private_key = private_key.into();
        let wallet = parse_owner_wallet(&private_key)?;
        Ok(Self {
            private_key: Arc::new(SecretString::from(private_key)),
            owner: DepositWalletOwnerAddress::new(wallet.address()),
        })
    }

    pub fn from_env() -> Result<Self> {
        let private_key = env::var(POLYMARKET_OWNER_PRIVATE_KEY_ENV).map_err(|_| {
            RelayerError::Signing(format!(
                "{POLYMARKET_OWNER_PRIVATE_KEY_ENV} must be set to load deposit wallet owner signer"
            ))
        })?;
        Self::new(private_key)
    }

    pub fn owner(&self) -> DepositWalletOwnerAddress {
        self.owner
    }

    pub fn owner_address(&self) -> Address {
        self.owner.address()
    }
}

impl fmt::Debug for DepositWalletOwnerSigner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let private_key = if self.private_key.expose_secret().is_empty() {
            "<invalid>"
        } else {
            "<redacted>"
        };
        f.debug_struct("DepositWalletOwnerSigner")
            .field("private_key", &private_key)
            .field("owner", &self.owner)
            .finish()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct DepositWalletOwnerAddress(Address);

impl DepositWalletOwnerAddress {
    pub fn new(address: Address) -> Self {
        Self(address)
    }

    pub fn address(self) -> Address {
        self.0
    }
}

impl From<Address> for DepositWalletOwnerAddress {
    fn from(address: Address) -> Self {
        Self::new(address)
    }
}

impl fmt::Debug for DepositWalletOwnerAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("DepositWalletOwnerAddress")
            .field(&redacted_address(self.0))
            .finish()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct DepositWalletAddress(Address);

impl DepositWalletAddress {
    pub fn new(address: Address) -> Self {
        Self(address)
    }

    pub fn address(self) -> Address {
        self.0
    }
}

impl From<Address> for DepositWalletAddress {
    fn from(address: Address) -> Self {
        Self::new(address)
    }
}

impl fmt::Debug for DepositWalletAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("DepositWalletAddress")
            .field(&redacted_address(self.0))
            .finish()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct DepositWalletFunderAddress(Address);

impl DepositWalletFunderAddress {
    pub fn new(address: Address) -> Self {
        Self(address)
    }

    pub fn address(self) -> Address {
        self.0
    }
}

impl From<Address> for DepositWalletFunderAddress {
    fn from(address: Address) -> Self {
        Self::new(address)
    }
}

impl fmt::Debug for DepositWalletFunderAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("DepositWalletFunderAddress")
            .field(&redacted_address(self.0))
            .finish()
    }
}

fn parse_owner_wallet(private_key: &str) -> Result<LocalWallet> {
    private_key.parse::<LocalWallet>().map_err(|_| {
        RelayerError::Signing("invalid deposit wallet owner private key".to_string())
    })
}

fn redacted_address(address: Address) -> String {
    let checksum = to_checksum(&address, None);
    format!("{}...{}", &checksum[..6], &checksum[38..])
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, OnceLock};

    use crate::deposit_wallet::RelayerKeyAuth;

    use super::*;

    const DUMMY_OWNER_PRIVATE_KEY: &str =
        "0x59c6995e998f97a5a0044966f094538af2b1810d2ffb53b028b6531dd3639d6d";

    fn parse_address(value: &str) -> Address {
        value.parse().unwrap()
    }

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
    }

    #[test]
    fn owner_signer_derives_owner_address_without_debugging_private_key() {
        let signer = DepositWalletOwnerSigner::new(DUMMY_OWNER_PRIVATE_KEY).unwrap();
        let expected_owner = DUMMY_OWNER_PRIVATE_KEY
            .parse::<LocalWallet>()
            .unwrap()
            .address();

        let debug = format!("{signer:?}");
        let owner_checksum = to_checksum(&expected_owner, None);

        assert_eq!(signer.owner_address(), expected_owner);
        assert_eq!(signer.owner().address(), expected_owner);
        assert!(debug.contains("private_key: \"<redacted>\""));
        assert!(!debug.contains(DUMMY_OWNER_PRIVATE_KEY));
        assert!(!debug.contains(DUMMY_OWNER_PRIVATE_KEY.trim_start_matches("0x")));
        assert!(!debug.contains(&format!("{expected_owner:?}")));
        assert!(!debug.contains(&owner_checksum));
    }

    #[test]
    fn owner_signer_loads_from_polymarket_owner_private_key_env() {
        let _guard = env_lock();
        let previous = env::var_os(POLYMARKET_OWNER_PRIVATE_KEY_ENV);
        env::set_var(POLYMARKET_OWNER_PRIVATE_KEY_ENV, DUMMY_OWNER_PRIVATE_KEY);

        let signer = DepositWalletOwnerSigner::from_env().unwrap();

        match previous {
            Some(value) => env::set_var(POLYMARKET_OWNER_PRIVATE_KEY_ENV, value),
            None => env::remove_var(POLYMARKET_OWNER_PRIVATE_KEY_ENV),
        }

        assert_eq!(
            signer.owner_address(),
            DUMMY_OWNER_PRIVATE_KEY
                .parse::<LocalWallet>()
                .unwrap()
                .address()
        );
    }

    #[test]
    fn owner_private_key_parse_errors_do_not_echo_secret_input() {
        let invalid_private_key = "not-a-private-key-secret";

        let error = DepositWalletOwnerSigner::new(invalid_private_key).unwrap_err();
        let rendered = error.to_string();

        assert!(rendered.contains("invalid deposit wallet owner private key"));
        assert!(!rendered.contains(invalid_private_key));
    }

    #[test]
    fn relayer_owner_deposit_wallet_and_funder_are_distinct_identity_types() {
        let relayer_auth_address = parse_address("0x1111111111111111111111111111111111111111");
        let owner_address = parse_address("0x2222222222222222222222222222222222222222");
        let deposit_wallet_address = parse_address("0x3333333333333333333333333333333333333333");
        let funder_address = parse_address("0x4444444444444444444444444444444444444444");
        let auth = RelayerKeyAuth::new("dummy-relayer-key", relayer_auth_address).unwrap();
        let owner = DepositWalletOwnerAddress::new(owner_address);
        let deposit_wallet = DepositWalletAddress::new(deposit_wallet_address);
        let funder = DepositWalletFunderAddress::new(funder_address);

        assert_ne!(auth.api_key_address(), owner.address());
        assert_ne!(owner.address(), deposit_wallet.address());
        assert_ne!(deposit_wallet.address(), funder.address());
        assert_ne!(auth.api_key_address(), funder.address());
    }
}
