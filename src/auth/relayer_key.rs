use crate::error::{RelayerError, Result};
use reqwest::header::{HeaderMap, HeaderValue};

pub fn build_headers(api_key: &str, address: &str) -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();

    let mut api_key = HeaderValue::from_str(api_key)
        .map_err(|_| RelayerError::AuthError("Invalid API key header value".to_string()))?;
    api_key.set_sensitive(true);
    headers.insert("RELAYER_API_KEY", api_key);

    let mut address = HeaderValue::from_str(address)
        .map_err(|_| RelayerError::AuthError("Invalid address header value".to_string()))?;
    address.set_sensitive(true);
    headers.insert("RELAYER_API_KEY_ADDRESS", address);

    Ok(headers)
}
