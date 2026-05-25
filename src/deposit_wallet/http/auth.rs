use super::*;
use super::redaction::redacted_address;

#[derive(Clone)]
pub struct RelayerKeyAuth {
    api_key: Arc<SecretString>,
    api_key_address: Address,
}

impl RelayerKeyAuth {
    pub fn new(api_key: impl Into<String>, api_key_address: Address) -> Self {
        Self {
            api_key: Arc::new(SecretString::from(api_key.into())),
            api_key_address,
        }
    }

    pub fn api_key_address(&self) -> Address {
        self.api_key_address
    }

    pub(super) fn headers(&self) -> Result<HeaderMap> {
        let mut headers = HeaderMap::new();
        let mut api_key = HeaderValue::from_str(self.api_key.expose_secret())
            .map_err(|_| RelayerError::AuthError("invalid relayer API key".to_string()))?;
        api_key.set_sensitive(true);
        headers.insert(relayer_api_key_header()?, api_key);

        let mut api_key_address =
            HeaderValue::from_str(&to_checksum(&self.api_key_address, None)).map_err(|_| {
                RelayerError::AuthError("invalid relayer API key address".to_string())
            })?;
        api_key_address.set_sensitive(true);
        headers.insert(relayer_api_key_address_header()?, api_key_address);
        Ok(headers)
    }
}

impl fmt::Debug for RelayerKeyAuth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RelayerKeyAuth")
            .field("api_key", &"<redacted>")
            .field(
                "api_key_address",
                &redacted_address(self.api_key_address),
            )
            .finish()
    }
}

pub(super) fn relayer_api_key_header() -> Result<HeaderName> {
    HeaderName::from_bytes(b"RELAYER_API_KEY")
        .map_err(|_| RelayerError::AuthError("invalid relayer API key header name".to_string()))
}

pub(super) fn relayer_api_key_address_header() -> Result<HeaderName> {
    HeaderName::from_bytes(b"RELAYER_API_KEY_ADDRESS").map_err(|_| {
        RelayerError::AuthError("invalid relayer API key address header name".to_string())
    })
}
