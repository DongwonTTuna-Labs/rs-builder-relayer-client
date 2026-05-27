pub mod builder;
pub mod relayer_key;

use std::fmt;

use ethers::types::Address;
use ethers::utils::to_checksum;
use reqwest::header::HeaderMap;
use secrecy::{ExposeSecret, SecretString};

/// Authentication method for the relayer.
#[derive(Clone)]
pub enum AuthMethod {
    /// Builder Program HMAC-SHA256 authentication.
    Builder(BuilderConfig),
    /// Simple Relayer API key authentication.
    RelayerKey {
        api_key: SecretString,
        address: String,
    },
}

impl fmt::Debug for AuthMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Builder(config) => f.debug_tuple("Builder").field(config).finish(),
            Self::RelayerKey { address, .. } => f
                .debug_struct("RelayerKey")
                .field("api_key", &"<redacted>")
                .field("address", &redacted_address_text(address))
                .finish(),
        }
    }
}

impl AuthMethod {
    /// Create a Builder auth method.
    pub fn builder(key: &str, secret: &str, passphrase: &str) -> Self {
        AuthMethod::Builder(BuilderConfig::new(key, secret, passphrase))
    }

    /// Create a Relayer Key auth method.
    pub fn relayer_key(api_key: &str, address: &str) -> Self {
        AuthMethod::RelayerKey {
            api_key: SecretString::from(api_key.to_string()),
            address: address.to_string(),
        }
    }

    /// Generate auth headers for a request.
    pub fn headers(
        &self,
        method: &str,
        path: &str,
        body: &str,
    ) -> crate::error::Result<HeaderMap> {
        match self {
            AuthMethod::Builder(config) => builder::build_headers(config, method, path, body),
            AuthMethod::RelayerKey { api_key, address } => {
                relayer_key::build_headers(api_key.expose_secret(), address)
            }
        }
    }
}

/// Builder Program API key credentials.
#[derive(Clone)]
pub struct BuilderConfig {
    pub key: SecretString,
    pub secret: SecretString,
    pub passphrase: SecretString,
}

impl BuilderConfig {
    pub fn new(key: &str, secret: &str, passphrase: &str) -> Self {
        Self {
            key: SecretString::from(key.to_string()),
            secret: SecretString::from(secret.to_string()),
            passphrase: SecretString::from(passphrase.to_string()),
        }
    }
}

impl fmt::Debug for BuilderConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BuilderConfig")
            .field("key", &"<redacted>")
            .field("secret", &"<redacted>")
            .field("passphrase", &"<redacted>")
            .finish()
    }
}

fn redacted_address_text(address: &str) -> String {
    match address.parse::<Address>() {
        Ok(address) => {
            let checksum = to_checksum(&address, None);
            format!("{}...{}", &checksum[..6], &checksum[38..])
        }
        Err(_) => "<redacted>".to_string(),
    }
}
