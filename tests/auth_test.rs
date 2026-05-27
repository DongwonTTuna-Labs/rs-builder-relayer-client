use polymarket_relayer::auth::builder::build_hmac_signature;
use polymarket_relayer::auth::{AuthMethod, BuilderConfig};
use secrecy::{ExposeSecret, SecretString};

#[test]
fn test_hmac_matches_reference_sdk() {
    // Test vector from py-builder-signing-sdk and builder-signing-sdk
    let secret = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
    let timestamp = "1000000";
    let method = "test-sign";
    let path = "/orders";
    let body = r#"{"hash": "0x123"}"#;

    let sig = build_hmac_signature(secret, timestamp, method, path, body).unwrap();
    assert_eq!(sig, "ZwAdJKvoYRlEKDkNMwd5BuwNNtg93kNaR_oU2HrfVvc=");
}

#[test]
fn test_hmac_empty_body() {
    let secret = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
    let sig = build_hmac_signature(secret, "1000000", "GET", "/nonce", "").unwrap();
    assert!(!sig.is_empty());
}

#[test]
fn test_hmac_url_safe_base64() {
    let secret = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
    let sig = build_hmac_signature(secret, "1000000", "test-sign", "/orders", r#"{"hash": "0x123"}"#).unwrap();
    assert!(!sig.contains('+'), "Should not contain '+'");
    assert!(!sig.contains('/'), "Should not contain '/'");
}

#[test]
fn test_builder_auth_generates_all_required_headers() {
    // Use a valid base64 secret
    let config = BuilderConfig::new(
        "test-key",
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
        "test-passphrase",
    );
    let auth = AuthMethod::Builder(config);
    let headers = auth.headers("POST", "/submit", r#"{"data":"test"}"#).unwrap();

    assert!(headers.contains_key("POLY_BUILDER_API_KEY"));
    assert!(headers.contains_key("POLY_BUILDER_TIMESTAMP"));
    assert!(headers.contains_key("POLY_BUILDER_PASSPHRASE"));
    assert!(headers.contains_key("POLY_BUILDER_SIGNATURE"));
    assert_eq!(headers.get("POLY_BUILDER_API_KEY").unwrap(), "test-key");
    assert_eq!(headers.get("POLY_BUILDER_PASSPHRASE").unwrap(), "test-passphrase");
}

#[test]
fn test_relayer_key_auth_generates_headers() {
    let auth = AuthMethod::relayer_key("my-key", "0x1234");
    let headers = auth.headers("GET", "/test", "").unwrap();

    assert_eq!(headers.get("RELAYER_API_KEY").unwrap(), "my-key");
    assert_eq!(headers.get("RELAYER_API_KEY_ADDRESS").unwrap(), "0x1234");
}

#[test]
fn test_relayer_key_auth_identity_can_differ_from_wallet_owner() {
    let auth_address = "0x1111111111111111111111111111111111111111";
    let wallet_owner = "0x2222222222222222222222222222222222222222";
    let auth = AuthMethod::relayer_key("my-key", auth_address);

    let headers = auth.headers("GET", "/nonce?type=WALLET", "").unwrap();

    assert_ne!(auth_address, wallet_owner);
    assert_eq!(headers.get("RELAYER_API_KEY_ADDRESS").unwrap(), auth_address);
    assert_ne!(
        headers
            .get("RELAYER_API_KEY_ADDRESS")
            .unwrap()
            .to_str()
            .unwrap(),
        wallet_owner
    );
}

#[test]
fn test_relayer_key_debug_redacts_malformed_address() {
    let secret_key = "secret-relayer-key";
    let malformed_address = "not-a-valid-address-secret";
    let auth = AuthMethod::RelayerKey {
        api_key: SecretString::from(secret_key.to_string()),
        address: malformed_address.to_string(),
    };

    let rendered = format!("{auth:?}");

    assert!(!rendered.contains(secret_key));
    assert!(!rendered.contains(malformed_address));
    assert!(rendered.contains("<redacted>"));
}

#[test]
fn test_auth_method_builder_convenience() {
    let auth = AuthMethod::builder("k", "s", "p");
    match auth {
        AuthMethod::Builder(config) => {
            assert_eq!(config.key.expose_secret(), "k");
            assert_eq!(config.secret.expose_secret(), "s");
            assert_eq!(config.passphrase.expose_secret(), "p");
        }
        _ => panic!("expected Builder variant"),
    }
}
