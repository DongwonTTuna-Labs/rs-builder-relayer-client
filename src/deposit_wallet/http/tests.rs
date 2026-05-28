use std::time::{Duration, UNIX_EPOCH};

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

use crate::deposit_wallet::{deposit_wallet_contract_config, WALLET_TRANSACTION_TYPE};

use super::response::{parse_transaction_response, validate_transaction_id};
use super::*;

const API_KEY: &str = "unit-test-relayer-api-key";
const API_KEY_ADDRESS: &str = "0xA6Db23622C9EA7584D5c61C3e7497c80E2CE167B";
const WALLET_OWNER: &str = "0x6e0c80c90ea6c15917308F820Eac91Ce2724B5b5";
const OTHER_OWNER: &str = "0x0000000000000000000000000000000000000001";
const TEST_SERVER_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug)]
struct CapturedRequest {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: String,
}

impl CapturedRequest {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

#[derive(Clone)]
struct TestResponse {
    status: &'static str,
    headers: Vec<(String, String)>,
    include_content_length: bool,
    body: String,
}

impl TestResponse {
    fn json(status: &'static str, body: impl Into<String>) -> Self {
        Self {
            status,
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            include_content_length: true,
            body: body.into(),
        }
    }

    fn json_without_content_length(status: &'static str, body: impl Into<String>) -> Self {
        Self {
            status,
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            include_content_length: false,
            body: body.into(),
        }
    }

    fn with_header(mut self, name: &str, value: impl Into<String>) -> Self {
        self.headers.push((name.to_string(), value.into()));
        self
    }
}

fn address(raw: &str) -> Address {
    raw.parse().expect("test address should parse")
}

fn fixture_text(name: &str) -> String {
    let path = format!(
        "{}/tests/fixtures/deposit_wallet/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(path).expect("fixture should be readable")
}

fn fixture_value(name: &str) -> Value {
    serde_json::from_str(&fixture_text(name)).expect("fixture should be valid JSON")
}

fn relayer_auth() -> RelayerKeyAuth {
    RelayerKeyAuth::new(API_KEY, address(API_KEY_ADDRESS)).unwrap()
}

fn reqwest_client(timeout: Duration) -> Client {
    Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(timeout)
        .build()
        .expect("test HTTP client should build")
}

fn test_client(base_url: DepositWalletRelayerUrl) -> DepositWalletRelayerClient {
    DepositWalletRelayerClient::from_parts(
        reqwest_client(Duration::from_secs(2)),
        base_url,
        relayer_auth(),
        deposit_wallet_contract_config(137).unwrap(),
    )
}

async fn spawn_server(
    responses: Vec<TestResponse>,
) -> (DepositWalletRelayerUrl, JoinHandle<Vec<CapturedRequest>>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("test server should bind");
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let mut requests = Vec::with_capacity(responses.len());
        for response in responses {
            let (mut stream, _) = tokio::time::timeout(TEST_SERVER_TIMEOUT, listener.accept())
                .await
                .expect("server accept should not hang")
                .expect("server should accept");
            let request = read_request(&mut stream).await;
            write_response(&mut stream, response).await;
            requests.push(request);
        }
        requests
    });

    (
        DepositWalletRelayerUrl::loopback(&format!("http://{addr}")).unwrap(),
        handle,
    )
}

async fn spawn_reset_server() -> (DepositWalletRelayerUrl, JoinHandle<Vec<CapturedRequest>>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("test server should bind");
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let (mut stream, _) = tokio::time::timeout(TEST_SERVER_TIMEOUT, listener.accept())
            .await
            .expect("server accept should not hang")
            .expect("server should accept");
        let request = read_request(&mut stream).await;
        drop(stream);
        vec![request]
    });

    (
        DepositWalletRelayerUrl::loopback(&format!("http://{addr}")).unwrap(),
        handle,
    )
}

async fn read_request(stream: &mut TcpStream) -> CapturedRequest {
    let mut buffer = Vec::new();
    let headers_end = loop {
        let mut chunk = [0u8; 1024];
        let read = tokio::time::timeout(TEST_SERVER_TIMEOUT, stream.read(&mut chunk))
            .await
            .expect("header read should not hang")
            .expect("request should read");
        assert!(read > 0, "request ended before headers completed");
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(index) = find_headers_end(&buffer) {
            break index;
        }
    };

    let body_start = headers_end + 4;
    let header_text = String::from_utf8(buffer[..headers_end].to_vec()).unwrap();
    let mut lines = header_text.split("\r\n");
    let request_line = lines.next().unwrap();
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts.next().unwrap().to_string();
    let path = request_parts.next().unwrap().to_string();
    let headers = lines
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.trim().to_string(), value.trim().to_string()))
        })
        .collect::<Vec<_>>();
    let content_length = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.parse::<usize>().ok())
        .unwrap_or(0);

    while buffer.len() < body_start + content_length {
        let mut chunk = [0u8; 1024];
        let read = tokio::time::timeout(TEST_SERVER_TIMEOUT, stream.read(&mut chunk))
            .await
            .expect("body read should not hang")
            .expect("body should read");
        assert!(read > 0, "request ended before body completed");
        buffer.extend_from_slice(&chunk[..read]);
    }

    let body = String::from_utf8(buffer[body_start..body_start + content_length].to_vec()).unwrap();

    CapturedRequest {
        method,
        path,
        headers,
        body,
    }
}

fn find_headers_end(buffer: &[u8]) -> Option<usize> {
    buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
}

async fn write_response(stream: &mut TcpStream, response: TestResponse) {
    let mut wire = format!("HTTP/1.1 {}\r\nconnection: close\r\n", response.status);
    if response.include_content_length {
        wire.push_str(&format!("content-length: {}\r\n", response.body.len()));
    }
    for (name, value) in response.headers {
        wire.push_str(&format!("{name}: {value}\r\n"));
    }
    wire.push_str("\r\n");
    wire.push_str(&response.body);
    stream
        .write_all(wire.as_bytes())
        .await
        .expect("response should write");
}

fn transaction_response_value_for_owner(
    transaction_id: &str,
    state: &str,
    owner: &str,
) -> Value {
    json!({
        "transactionID": transaction_id,
        "type": WALLET_TRANSACTION_TYPE,
        "from": owner,
        "to": to_checksum(&deposit_wallet_contract_config(137).unwrap().factory, None),
        "state": state,
        "transactionHash": "0x38cbfbeae8fffa4e2b187ee5978d3ee9cafc53af0363ed90a35b7ea9016535d8",
        "owner": owner
    })
}

fn transaction_response_value(transaction_id: &str, state: &str) -> Value {
    transaction_response_value_for_owner(transaction_id, state, WALLET_OWNER)
}

#[test]
fn relayer_key_auth_validates_redacts_and_marks_headers_sensitive() {
    assert!(RelayerKeyAuth::new("", address(API_KEY_ADDRESS)).is_err());
    assert!(RelayerKeyAuth::new("with whitespace", address(API_KEY_ADDRESS)).is_err());

    let auth = relayer_auth();
    let headers = auth.headers().unwrap();

    assert_eq!(headers.get("RELAYER_API_KEY").unwrap(), API_KEY);
    assert!(headers.get("RELAYER_API_KEY").unwrap().is_sensitive());
    assert!(headers
        .get("RELAYER_API_KEY_ADDRESS")
        .unwrap()
        .is_sensitive());
    assert!(!format!("{auth:?}").contains(API_KEY));
}

#[test]
fn relayer_url_enforces_production_boundary() {
    assert!(DepositWalletRelayerUrl::parse("https://relayer-v2.polymarket.com").is_ok());
    assert!(DepositWalletRelayerUrl::parse("http://relayer-v2.polymarket.com").is_err());
    assert!(DepositWalletRelayerUrl::parse("https://example.com").is_err());
    assert!(DepositWalletRelayerUrl::parse("https://relayer-v2.polymarket.com/path").is_err());
    assert!(DepositWalletRelayerUrl::parse("https://user@relayer-v2.polymarket.com").is_err());
    assert!(DepositWalletRelayerUrl::parse("https://relayer-v2.polymarket.com:8443").is_err());
    assert!(DepositWalletRelayerUrl::parse("https://relayer-v2.polymarket.com?x=1").is_err());
    assert!(DepositWalletRelayerUrl::parse("https://relayer-v2.polymarket.com/#frag").is_err());
    assert!(DepositWalletRelayerUrl::loopback("http://[::1]/").is_ok());

    let production = DepositWalletRelayerUrl::parse("https://relayer-v2.polymarket.com").unwrap();
    let amoy = deposit_wallet_contract_config(80002).unwrap();
    let error = DepositWalletRelayerClient::new(production, relayer_auth(), amoy).unwrap_err();
    assert!(error.to_string().contains("Polygon deposit wallet contract config"));
}

#[tokio::test]
async fn get_wallet_nonce_sends_exact_path_and_parses_decimal_nonce() {
    let expected = fixture_value("wallet_nonce_http_request.json");
    let (url, handle) = spawn_server(vec![TestResponse::json(
        "200 OK",
        expected["response"].to_string(),
    )])
    .await;
    let client = test_client(url);
    let owner: Address = expected["address"].as_str().unwrap().parse().unwrap();

    let nonce = client.get_wallet_nonce(owner).await.unwrap();

    assert_eq!(nonce, U256::from(31u64));
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, expected["method"].as_str().unwrap());
    assert_eq!(requests[0].path, expected["pathAndQuery"].as_str().unwrap());
    assert_eq!(requests[0].header("RELAYER_API_KEY"), Some(API_KEY));
    assert_eq!(
        requests[0].header("RELAYER_API_KEY_ADDRESS"),
        Some(to_checksum(&address(API_KEY_ADDRESS), None).as_str())
    );
    assert!(requests[0].body.is_empty());
}

#[tokio::test]
async fn get_wallet_nonce_rejects_production_before_http() {
    let url = DepositWalletRelayerUrl::parse("https://relayer-v2.polymarket.com").unwrap();
    let client = test_client(url);

    let error = client.get_wallet_nonce(address(WALLET_OWNER)).await.unwrap_err();

    assert!(error.is_deposit_wallet_mutation_blocked());
}

#[test]
fn wallet_nonce_parser_uses_fixture_for_invalid_boundaries() {
    assert_eq!(
        super::read::parse_wallet_nonce_response(br#"{"nonce":31}"#).unwrap(),
        U256::from(31u64)
    );
    assert_eq!(
        super::read::parse_wallet_nonce_response(
            format!(r#"{{"nonce":"{}"}}"#, U256::MAX).as_bytes()
        )
        .unwrap(),
        U256::MAX
    );

    let fixture = fixture_value("wallet_nonce_response_cases.json");
    for case in fixture["rejected"].as_array().unwrap() {
        let raw_nonce = case["raw"].as_str().unwrap();
        assert!(
            super::read::parse_wallet_nonce_response(
                format!(r#"{{"nonce":{raw_nonce}}}"#).as_bytes()
            )
            .is_err(),
            "expected nonce case {} to be rejected",
            case["label"].as_str().unwrap()
        );
    }
}

#[test]
fn transaction_response_fixture_matches_official_owner_field() {
    let transaction_id = "0190b317-a1d3-7bec-9b91-eeb6dcd3a620";
    let fixture = fixture_text("wallet_transaction_response.json");

    let parsed = parse_transaction_response(
        transaction_id,
        deposit_wallet_contract_config(137).unwrap().factory,
        fixture.as_bytes(),
    )
    .unwrap();

    assert_eq!(parsed.receipt.transaction_id, transaction_id);
    assert_eq!(parsed.receipt.state, RelayerTransactionState::Confirmed);
    assert_eq!(parsed.owner, Some(address(WALLET_OWNER)));
    assert_eq!(parsed.receipt.owner, Some(address(WALLET_OWNER)));
    let debug = format!("{:?}", parsed.receipt);
    let owner_checksum = to_checksum(&address(WALLET_OWNER), None);
    assert!(!debug.contains(&owner_checksum));
    assert!(!debug.contains(&owner_checksum.to_ascii_lowercase()));
}

#[tokio::test]
async fn get_transaction_for_owner_accepts_requested_owner() {
    let (url, handle) = spawn_server(vec![TestResponse::json(
        "200 OK",
        transaction_response_value("tx-owner", "STATE_CONFIRMED").to_string(),
    )])
    .await;
    let client = test_client(url);

    let receipt = client
        .get_transaction_for_owner(address(WALLET_OWNER), "tx-owner")
        .await
        .unwrap();

    assert_eq!(receipt.transaction_id, "tx-owner");
    assert_eq!(receipt.state, RelayerTransactionState::Confirmed);
    assert_eq!(receipt.owner, Some(address(WALLET_OWNER)));
    let requests = handle.await.unwrap();
    assert_eq!(requests[0].path, "/transaction?id=tx-owner");
}

#[tokio::test]
async fn get_transaction_for_owner_rejects_confirmed_without_hash() {
    let mut response = transaction_response_value("tx-confirmed-no-hash", "STATE_CONFIRMED");
    response["transactionHash"] = json!("");
    let (url, handle) = spawn_server(vec![TestResponse::json("200 OK", response.to_string())]).await;
    let client = test_client(url);

    let error = client
        .get_transaction_for_owner(address(WALLET_OWNER), "tx-confirmed-no-hash")
        .await
        .unwrap_err();

    assert!(error.is_deposit_wallet_reconciliation_required());
    assert!(error.to_string().contains("did not include transactionHash"));
    let _ = handle.await.unwrap();
}

#[tokio::test]
async fn get_transaction_for_owner_accepts_non_terminal_states_without_hash() {
    for (transaction_id, state, expected_state) in [
        ("tx-new", "STATE_NEW", RelayerTransactionState::New),
        ("tx-executed", "STATE_EXECUTED", RelayerTransactionState::Executed),
        ("tx-mined", "STATE_MINED", RelayerTransactionState::Mined),
    ] {
        let mut response = transaction_response_value(transaction_id, state);
        response.as_object_mut().unwrap().remove("transactionHash");
        let (url, handle) =
            spawn_server(vec![TestResponse::json("200 OK", response.to_string())]).await;
        let client = test_client(url);

        let receipt = client
            .get_transaction_for_owner(address(WALLET_OWNER), transaction_id)
            .await
            .unwrap();

        assert_eq!(receipt.state, expected_state);
        assert_eq!(receipt.transaction_hash, None);
        let _ = handle.await.unwrap();
    }
}

async fn transaction_state_error(transaction_id: &str, state: &str) -> RelayerError {
    let (url, handle) = spawn_server(vec![TestResponse::json(
        "200 OK",
        transaction_response_value(transaction_id, state).to_string(),
    )])
    .await;
    let client = test_client(url);

    let error = client
        .get_transaction_for_owner(address(WALLET_OWNER), transaction_id)
        .await
        .unwrap_err();
    let _ = handle.await.unwrap();
    error
}

#[tokio::test]
async fn get_transaction_for_owner_rejects_terminal_failure_and_unknown_states() {
    let invalid = transaction_state_error("tx-invalid", "STATE_INVALID").await;
    assert!(matches!(invalid, RelayerError::TransactionInvalid(_)));

    let failed = transaction_state_error("tx-failed", "STATE_FAILED").await;
    assert!(matches!(failed, RelayerError::TransactionFailed(_)));

    let unknown = transaction_state_error("tx-unknown", "STATE_FUTURE").await;
    assert!(unknown.is_deposit_wallet_reconciliation_required());
    assert!(!unknown.to_string().contains("STATE_FUTURE"));
}

#[tokio::test]
async fn get_transaction_for_owner_rejects_mismatched_owner() {
    let (url, handle) = spawn_server(vec![TestResponse::json(
        "200 OK",
        transaction_response_value_for_owner("tx-owner", "STATE_CONFIRMED", OTHER_OWNER)
            .to_string(),
    )])
    .await;
    let client = test_client(url);

    let error = client
        .get_transaction_for_owner(address(WALLET_OWNER), "tx-owner")
        .await
        .unwrap_err();

    assert!(error.is_deposit_wallet_reconciliation_required());
    assert!(error.to_string().contains("did not match requested owner"));
    assert!(!error.to_string().contains(OTHER_OWNER));
    let _ = handle.await.unwrap();
}

#[tokio::test]
async fn get_transaction_for_owner_covers_404_missing_array_and_transient_errors() {
    let (url, handle) = spawn_server(vec![TestResponse::json("404 Not Found", "{}")]).await;
    let client = test_client(url);

    let error = client
        .get_transaction_for_owner(address(WALLET_OWNER), "tx-missing-http")
        .await
        .unwrap_err();
    assert!(matches!(error, RelayerError::Api { status: 404, .. }));
    let requests = handle.await.unwrap();
    assert_eq!(requests[0].path, "/transaction?id=tx-missing-http");

    let (url, handle) = spawn_server(vec![TestResponse::json(
        "200 OK",
        json!([transaction_response_value("other-tx", "STATE_CONFIRMED")]).to_string(),
    )])
    .await;
    let client = test_client(url);

    let error = client
        .get_transaction_for_owner(address(WALLET_OWNER), "tx-missing-array")
        .await
        .unwrap_err();
    assert!(error.is_deposit_wallet_reconciliation_required());
    assert!(error.to_string().contains("did not include requested transaction id hash"));
    let _ = handle.await.unwrap();

    let (url, handle) = spawn_reset_server().await;
    let client = test_client(url);
    let error = client
        .get_transaction_for_owner(address(WALLET_OWNER), "tx-reset")
        .await
        .unwrap_err();
    assert!(matches!(error, RelayerError::Http(_)));
    let _ = handle.await.unwrap();
}

#[test]
fn transaction_array_parser_uses_fixture_for_selection_and_negative_cases() {
    let fixture = fixture_value("transaction_array_response_cases.json");
    let target = fixture["target"].as_str().unwrap();
    let item = |transaction_id: String| transaction_response_value(&transaction_id, "STATE_CONFIRMED");

    let body_from_ids = |ids: &Value| {
        json!(
            ids.as_array()
                .unwrap()
                .iter()
                .map(|id| item(id.as_str().unwrap().to_string()))
                .collect::<Vec<_>>()
        )
        .to_string()
    };
    let matching = body_from_ids(&fixture["matchingIds"]);
    let missing = body_from_ids(&fixture["missingIds"]);

    let parsed = parse_transaction_response(
        target,
        deposit_wallet_contract_config(137).unwrap().factory,
        matching.as_bytes(),
    )
    .unwrap();
    assert_eq!(parsed.receipt.transaction_id, target);

    let error = parse_transaction_response(
        target,
        deposit_wallet_contract_config(137).unwrap().factory,
        missing.as_bytes(),
    )
    .unwrap_err()
    .error;
    assert!(error.is_deposit_wallet_reconciliation_required());
    assert!(error.to_string().contains("did not include requested transaction id"));

    let object_mismatch = transaction_response_value("other-object-id", "STATE_CONFIRMED");
    let error = parse_transaction_response(
        target,
        deposit_wallet_contract_config(137).unwrap().factory,
        object_mismatch.to_string().as_bytes(),
    )
    .unwrap_err()
    .error;
    assert!(error.is_deposit_wallet_reconciliation_required());
    assert!(error.to_string().contains("did not match requested id hash"));
    assert!(!error.to_string().contains("other-object-id"));

    for (fixture_key, expected_message) in [
        ("duplicateIds", "duplicate requested transaction id hash"),
        ("invalidIds", "invalid transactionID"),
        ("oversizedIds", "included more than"),
    ] {
        let body = body_from_ids(&fixture[fixture_key]);
        let error = parse_transaction_response(
            target,
            deposit_wallet_contract_config(137).unwrap().factory,
            body.as_bytes(),
        )
        .unwrap_err()
        .error;
        assert!(error.is_deposit_wallet_reconciliation_required());
        assert!(
            error.to_string().contains(expected_message),
            "{fixture_key} produced unexpected error: {error}"
        );
    }
}

#[test]
fn transaction_response_rejects_malformed_transaction_hashes() {
    let expected_factory = deposit_wallet_contract_config(137).unwrap().factory;
    for (label, malformed_hash) in [
        ("too-short", "0x1234"),
        (
            "missing-prefix",
            "38cbfbeae8fffa4e2b187ee5978d3ee9cafc53af0363ed90a35b7ea9016535d8",
        ),
        (
            "non-hex",
            "0x38cbfbeae8fffa4e2b187ee5978d3ee9cafc53af0363ed90a35b7ea9016535dg",
        ),
    ] {
        let transaction_id = format!("tx-bad-hash-{label}");
        let mut response = transaction_response_value(&transaction_id, "STATE_CONFIRMED");
        response["transactionHash"] = json!(malformed_hash);

        let error = parse_transaction_response(
            &transaction_id,
            expected_factory,
            response.to_string().as_bytes(),
        )
        .unwrap_err()
        .error;

        assert!(
            error.is_deposit_wallet_reconciliation_required(),
            "{label}: {error}"
        );
        assert!(
            error.to_string().contains("transactionHash was invalid"),
            "{label}: {error}"
        );
    }
}

#[test]
fn transaction_response_rejects_unproven_wire_evidence_boundaries() {
    let target = "tx-wire-evidence";
    let expected_factory = deposit_wallet_contract_config(137).unwrap().factory;
    let other_address = to_checksum(&address(OTHER_OWNER), None);
    let mut missing_type = transaction_response_value(target, "STATE_CONFIRMED");
    missing_type.as_object_mut().unwrap().remove("type");
    let mut wallet_create_type = transaction_response_value(target, "STATE_CONFIRMED");
    wallet_create_type["type"] = json!(crate::deposit_wallet::WALLET_CREATE_TRANSACTION_TYPE);
    let mut wrong_type = transaction_response_value(target, "STATE_CONFIRMED");
    wrong_type["type"] = json!("SAFE");
    let mut missing_owner = transaction_response_value(target, "STATE_CONFIRMED");
    missing_owner.as_object_mut().unwrap().remove("owner");
    let mut owner_mismatch = transaction_response_value(target, "STATE_CONFIRMED");
    owner_mismatch["owner"] = json!(other_address);
    let mut missing_from = transaction_response_value(target, "STATE_CONFIRMED");
    missing_from.as_object_mut().unwrap().remove("from");
    let mut from_mismatch = transaction_response_value(target, "STATE_CONFIRMED");
    from_mismatch["from"] = json!(other_address);
    let mut missing_to = transaction_response_value(target, "STATE_CONFIRMED");
    missing_to.as_object_mut().unwrap().remove("to");
    let mut to_mismatch = transaction_response_value(target, "STATE_CONFIRMED");
    to_mismatch["to"] = json!(other_address);

    for (label, response, expected_message) in [
        (
            "missing type",
            missing_type,
            "did not include deposit-wallet transaction type",
        ),
        ("WALLET-CREATE type", wallet_create_type, "type was not WALLET"),
        ("wrong type", wrong_type, "type was not WALLET"),
        (
            "missing owner",
            missing_owner,
            "did not include owner evidence",
        ),
        (
            "owner mismatch",
            owner_mismatch,
            "did not match owner evidence",
        ),
        ("missing from", missing_from, "did not include from address"),
        ("from mismatch", from_mismatch, "did not match owner evidence"),
        ("missing to", missing_to, "did not include to address"),
        (
            "to mismatch",
            to_mismatch,
            "did not match expected relayer target",
        ),
    ] {
        let error =
            parse_transaction_response(target, expected_factory, response.to_string().as_bytes())
                .unwrap_err()
                .error;
        assert!(
            error.is_deposit_wallet_reconciliation_required(),
            "{label}: {error}"
        );
        assert!(
            error.to_string().contains(expected_message),
            "{label} produced unexpected error: {error}"
        );
    }
}

#[test]
fn retry_after_parser_accepts_seconds_and_http_date_boundaries() {
    let now = UNIX_EPOCH + Duration::from_secs(10);
    let mut headers = HeaderMap::new();
    headers.insert(RETRY_AFTER, HeaderValue::from_static("7"));
    assert_eq!(
        super::transport::retry_after_duration_at(&headers, now),
        Some(Duration::from_secs(7))
    );

    headers.insert(
        RETRY_AFTER,
        HeaderValue::from_str(&httpdate::fmt_http_date(UNIX_EPOCH + Duration::from_secs(42)))
            .unwrap(),
    );
    assert_eq!(
        super::transport::retry_after_duration_at(&headers, now),
        Some(Duration::from_secs(32))
    );
}

#[tokio::test]
async fn transport_preserves_429_retry_after_and_caps_success_bodies() {
    let (url, handle) = spawn_server(vec![TestResponse::json("429 Too Many Requests", "{}")
        .with_header("retry-after", "7")])
    .await;
    let client = test_client(url);
    let error = client
        .get_wallet_nonce(address(WALLET_OWNER))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        RelayerError::Api { status: 429, ref message }
            if message.contains("retry after 7s")
    ));
    let _ = handle.await.unwrap();

    let content_length_too_large =
        TestResponse::json_without_content_length("200 OK", "").with_header(
            "content-length",
            (MAX_SUCCESS_BODY_BYTES + 1).to_string(),
        );
    let (url, handle) = spawn_server(vec![content_length_too_large]).await;
    let client = test_client(url);
    let error = client
        .get_wallet_nonce(address(WALLET_OWNER))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        RelayerError::Other(ref message) if message.contains("maximum size")
    ));
    let _ = handle.await;

    let oversized_body = "x".repeat(MAX_SUCCESS_BODY_BYTES + 1);
    let (url, handle) = spawn_server(vec![TestResponse::json_without_content_length(
        "200 OK",
        oversized_body,
    )])
    .await;
    let client = test_client(url);
    let error = client
        .get_wallet_nonce(address(WALLET_OWNER))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        RelayerError::Other(ref message) if message.contains("maximum size")
    ));
    let _ = handle.await;
}

#[test]
fn transaction_id_validation_covers_length_and_allowed_characters() {
    let max_len = "a".repeat(MAX_TRANSACTION_ID_LEN);
    assert_eq!(validate_transaction_id(&max_len).unwrap(), max_len);
    assert_eq!(
        validate_transaction_id("tx-abc_123.period").unwrap(),
        "tx-abc_123.period"
    );

    let too_long = "a".repeat(MAX_TRANSACTION_ID_LEN + 1);
    assert!(validate_transaction_id(&too_long).is_err());
    assert!(validate_transaction_id("").is_err());
    assert!(validate_transaction_id("tx abc").is_err());
    assert!(validate_transaction_id("tx\nabc").is_err());
}
