use std::time::{Duration, UNIX_EPOCH};

use ethers::types::Bytes;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

use crate::deposit_wallet::{
    deposit_wallet_contract_config, derive_deposit_wallet_address,
    validate_deposit_wallet_batch_signature, DepositWalletBatchToSign, DepositWalletCall,
    SignedDepositWalletBatch, WALLET_TRANSACTION_TYPE,
};

use super::response::{parse_deployed_response, parse_transaction_response, validate_transaction_id};
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

fn fixture_address(data: &Value, key: &str) -> Address {
    data[key]
        .as_str()
        .expect("fixture address should be a string")
        .parse()
        .expect("fixture address should parse")
}

fn fixture_u256(data: &Value, key: &str) -> U256 {
    U256::from_dec_str(data[key].as_str().expect("fixture U256 should be a string"))
        .expect("fixture U256 should parse")
}

fn fixture_call(value: &Value) -> DepositWalletCall {
    DepositWalletCall {
        target: value["target"]
            .as_str()
            .expect("fixture call target should be a string")
            .parse()
            .expect("fixture call target should parse"),
        value: U256::from_dec_str(
            value["value"]
                .as_str()
                .expect("fixture call value should be a string"),
        )
        .expect("fixture call value should parse"),
        data: Bytes::from(
            hex::decode(
                value["data"]
                    .as_str()
                    .expect("fixture call data should be a string")
                    .trim_start_matches("0x"),
            )
            .expect("fixture call data should parse"),
        ),
    }
}

fn signed_batch_fixture(name: &str) -> SignedDepositWalletBatch {
    let data = fixture_value(name);
    let batch = DepositWalletBatchToSign {
        owner: fixture_address(&data, "owner"),
        nonce_owner: fixture_address(&data, "nonceOwner"),
        submit_from: fixture_address(&data, "submitFrom"),
        deposit_wallet: fixture_address(&data, "depositWallet"),
        chain_id: data["chainId"].as_u64().expect("chainId should be u64"),
        nonce: fixture_u256(&data, "nonce"),
        deadline: fixture_u256(&data, "deadline"),
        calls: data["calls"]
            .as_array()
            .expect("calls should be an array")
            .iter()
            .map(fixture_call)
            .collect(),
    };
    validate_deposit_wallet_batch_signature(
        batch,
        data["ownerSignature"]
            .as_str()
            .expect("ownerSignature should be a string"),
    )
    .expect("fixture signature should validate")
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

fn amoy_test_client(base_url: DepositWalletRelayerUrl) -> DepositWalletRelayerClient {
    DepositWalletRelayerClient::from_parts(
        reqwest_client(Duration::from_secs(2)),
        base_url,
        relayer_auth(),
        deposit_wallet_contract_config(80002).unwrap(),
    )
}

fn short_polling_config() -> DepositWalletPollingConfig {
    DepositWalletPollingConfig {
        initial_backoff: Duration::from_millis(1),
        max_backoff: Duration::from_millis(1),
        timeout: Duration::from_secs(1),
    }
}

fn assert_submit_request_headers(request: &CapturedRequest) {
    assert_eq!(request.method, "POST");
    assert_eq!(request.path, "/submit");
    assert_eq!(request.header("content-type"), Some("application/json"));
    assert_eq!(request.header("RELAYER_API_KEY"), Some(API_KEY));
    assert_eq!(
        request.header("RELAYER_API_KEY_ADDRESS"),
        Some(to_checksum(&address(API_KEY_ADDRESS), None).as_str())
    );
}

fn assert_json_body_matches_fixture(body: &str, fixture_name: &str) {
    let posted_body = serde_json::from_str::<Value>(body).expect("posted body should be JSON");
    assert_eq!(posted_body, fixture_value(fixture_name));
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

async fn spawn_optional_redirect_target(
    response: TestResponse,
) -> (String, JoinHandle<Vec<CapturedRequest>>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("test server should bind");
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let Ok(accepted) = tokio::time::timeout(TEST_SERVER_TIMEOUT, listener.accept()).await
        else {
            return Vec::new();
        };
        let (mut stream, _) = accepted.expect("server should accept");
        let request = read_request(&mut stream).await;
        write_response(&mut stream, response).await;
        vec![request]
    });

    (format!("http://{addr}/redirect-target"), handle)
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
    let config = deposit_wallet_contract_config(137).unwrap();
    let deposit_wallet = derive_deposit_wallet_address(address(owner), config).unwrap();
    json!({
        "transactionID": transaction_id,
        "type": WALLET_TRANSACTION_TYPE,
        "from": owner,
        "to": to_checksum(&config.factory, None),
        "proxyAddress": to_checksum(&deposit_wallet, None),
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
    for rejected in ["abc\n", "abc\t", "abc\0"] {
        assert!(
            RelayerKeyAuth::new(rejected, address(API_KEY_ADDRESS)).is_err(),
            "{rejected:?} should be rejected"
        );
    }
    assert!(RelayerKeyAuth::new("a".repeat(4096), address(API_KEY_ADDRESS)).is_ok());
    assert!(RelayerKeyAuth::new("a".repeat(4097), address(API_KEY_ADDRESS)).is_err());

    let auth = relayer_auth();
    let headers = auth.headers().unwrap();

    assert_eq!(headers.get("RELAYER_API_KEY").unwrap(), API_KEY);
    assert!(headers.get("RELAYER_API_KEY").unwrap().is_sensitive());
    assert!(headers
        .get("RELAYER_API_KEY_ADDRESS")
        .unwrap()
        .is_sensitive());
    assert!(!format!("{auth:?}").contains(API_KEY));

    let client = test_client(DepositWalletRelayerUrl::loopback("http://127.0.0.1/").unwrap());
    let client_debug = format!("{client:?}");
    assert!(!client_debug.contains(API_KEY));
    assert!(!client_debug.contains(API_KEY_ADDRESS));
}

#[test]
fn relayer_url_enforces_production_boundary() {
    assert!(DepositWalletRelayerUrl::parse("https://relayer-v2.polymarket.com").is_ok());
    assert!(DepositWalletRelayerUrl::parse("https://relayer-v2-staging.polymarket.dev").is_ok());

    for rejected in [
        "http://relayer-v2.polymarket.com",
        "http://relayer-v2-staging.polymarket.dev",
        "https://example.com",
        "https://relayer-v2-staging.polymarket.com",
        "https://relayer-v2.polymarket.dev",
        "https://relayer-v2.polymarket.com/path",
        "https://relayer-v2-staging.polymarket.dev/path",
        "https://user@relayer-v2.polymarket.com",
        "https://user@relayer-v2-staging.polymarket.dev",
        "https://relayer-v2.polymarket.com:8443",
        "https://relayer-v2-staging.polymarket.dev:8443",
        "https://relayer-v2.polymarket.com?x=1",
        "https://relayer-v2-staging.polymarket.dev?x=1",
        "https://relayer-v2.polymarket.com/#frag",
        "https://relayer-v2-staging.polymarket.dev/#frag",
    ] {
        assert!(
            DepositWalletRelayerUrl::parse(rejected).is_err(),
            "{rejected} should be rejected"
        );
    }

    assert!(DepositWalletRelayerUrl::loopback("http://[::1]/").is_ok());
    assert!(DepositWalletRelayerUrl::loopback("http://user@127.0.0.1/").is_err());
    assert!(DepositWalletRelayerUrl::loopback("http://127.0.0.1/?x=1").is_err());
    assert!(DepositWalletRelayerUrl::loopback("http://127.0.0.1/#frag").is_err());
    assert!(DepositWalletRelayerUrl::loopback("http://127.0.0.1/path").is_err());
    assert!(DepositWalletRelayerUrl::loopback("http://192.0.2.1/").is_err());

    let polygon_url = DepositWalletRelayerUrl::parse("https://relayer-v2.polymarket.com").unwrap();
    let amoy_url =
        DepositWalletRelayerUrl::parse("https://relayer-v2-staging.polymarket.dev").unwrap();
    let polygon_config = deposit_wallet_contract_config(137).unwrap();
    let amoy_config = deposit_wallet_contract_config(80002).unwrap();

    assert!(DepositWalletRelayerClient::new(polygon_url.clone(), relayer_auth(), polygon_config)
        .is_ok());
    assert!(DepositWalletRelayerClient::new(amoy_url.clone(), relayer_auth(), amoy_config).is_ok());

    let polygon_error =
        DepositWalletRelayerClient::new(polygon_url, relayer_auth(), amoy_config).unwrap_err();
    assert!(polygon_error.to_string().contains("chain 137"));

    let amoy_error =
        DepositWalletRelayerClient::new(amoy_url, relayer_auth(), polygon_config).unwrap_err();
    assert!(amoy_error.to_string().contains("chain 80002"));
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
async fn get_wallet_nonce_fetches_fresh_nonce_on_each_call() {
    let expected = fixture_value("wallet_nonce_http_request.json");
    let (url, handle) = spawn_server(vec![
        TestResponse::json("200 OK", expected["response"].to_string()),
        TestResponse::json("200 OK", r#"{"nonce":"32"}"#),
    ])
    .await;
    let client = test_client(url);
    let owner: Address = expected["address"].as_str().unwrap().parse().unwrap();

    let first = client.get_wallet_nonce(owner).await.unwrap();
    let second = client.get_wallet_nonce(owner).await.unwrap();

    assert_eq!(first, U256::from(31u64));
    assert_eq!(second, U256::from(32u64));
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 2);
    for request in requests {
        assert_eq!(request.method, expected["method"].as_str().unwrap());
        assert_eq!(request.path, expected["pathAndQuery"].as_str().unwrap());
        assert!(request.body.is_empty());
    }
}

#[tokio::test]
async fn relayer_client_does_not_follow_redirects_with_auth_headers() {
    let (redirect_target, target_handle) =
        spawn_optional_redirect_target(TestResponse::json("200 OK", r#"{"nonce":31}"#)).await;
    let redirect = TestResponse {
        status: "302 Found",
        headers: vec![("location".to_string(), redirect_target)],
        include_content_length: true,
        body: String::new(),
    };
    let (url, redirect_handle) = spawn_server(vec![redirect]).await;
    let client = DepositWalletRelayerClient::new(
        url,
        relayer_auth(),
        deposit_wallet_contract_config(137).unwrap(),
    )
    .unwrap();

    let error = client.get_wallet_nonce(address(WALLET_OWNER)).await.unwrap_err();

    assert!(matches!(error, RelayerError::Api { status: 302, .. }));
    let redirect_requests = redirect_handle.await.unwrap();
    assert_eq!(redirect_requests.len(), 1);
    assert_eq!(redirect_requests[0].header("RELAYER_API_KEY"), Some(API_KEY));
    assert_eq!(
        redirect_requests[0].header("RELAYER_API_KEY_ADDRESS"),
        Some(to_checksum(&address(API_KEY_ADDRESS), None).as_str())
    );
    let target_requests = target_handle.await.unwrap();
    assert!(target_requests.is_empty());
}

#[test]
fn get_wallet_nonce_production_client_reuses_url_guard_before_http() {
    let url = DepositWalletRelayerUrl::parse("https://relayer-v2.polymarket.com").unwrap();

    let error = DepositWalletRelayerClient::new(
        url,
        relayer_auth(),
        deposit_wallet_contract_config(80002).unwrap(),
    )
    .unwrap_err();

    assert!(error.to_string().contains("chain 137"));
}

#[tokio::test]
async fn discover_deposit_wallet_reads_deployed_address_or_create_needed() {
    let fixture = fixture_value("wallet_deployed_response_cases.json");
    let owner = address(fixture["owner"].as_str().unwrap());
    let expected_wallet = derive_deposit_wallet_address(
        owner,
        deposit_wallet_contract_config(137).unwrap(),
    )
    .unwrap();
    let (url, handle) = spawn_server(vec![
        TestResponse::json("200 OK", fixture["deployed"].to_string()),
        TestResponse::json("200 OK", fixture["notDeployed"].to_string()),
    ])
    .await;
    let client = test_client(url);

    let deployed = client.discover_deposit_wallet(owner).await.unwrap();
    let create_needed = client.discover_deposit_wallet(owner).await.unwrap();

    assert_eq!(deployed.owner(), owner);
    assert_eq!(deployed.deposit_wallet(), expected_wallet);
    assert!(deployed.is_deployed());
    assert!(!deployed.wallet_create_needed());
    assert_eq!(create_needed.owner(), owner);
    assert_eq!(create_needed.deposit_wallet(), expected_wallet);
    assert!(!create_needed.is_deployed());
    assert!(create_needed.wallet_create_needed());
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 2);
    let expected_path = format!(
        "/deployed?address={}&type=WALLET",
        to_checksum(&owner, None)
    );
    for request in requests {
        assert_eq!(request.method, "GET");
        assert_eq!(request.path, expected_path);
        assert_eq!(request.header("RELAYER_API_KEY"), Some(API_KEY));
        assert_eq!(
            request.header("RELAYER_API_KEY_ADDRESS"),
            Some(to_checksum(&address(API_KEY_ADDRESS), None).as_str())
        );
        assert!(request.body.is_empty());
    }
}

#[test]
fn deployed_response_parser_rejects_ambiguous_or_mismatched_wallet_evidence() {
    let owner = address(WALLET_OWNER);
    let expected_wallet =
        derive_deposit_wallet_address(owner, deposit_wallet_contract_config(137).unwrap()).unwrap();
    for (label, response, expected_message) in [
        (
            "deployed true without address",
            json!({"deployed": true}),
            "did not include a wallet address",
        ),
        (
            "not deployed with address",
            json!({"deployed": false, "proxyAddress": to_checksum(&expected_wallet, None)}),
            "not deployed but also included",
        ),
        (
            "mismatched address",
            json!({"deployed": true, "proxyAddress": OTHER_OWNER}),
            "did not match derived deposit wallet",
        ),
        (
            "malformed address",
            json!({"deployed": true, "proxyAddress": "0x1234"}),
            "wallet address was malformed",
        ),
    ] {
        let error = parse_deployed_response(owner, expected_wallet, response.to_string().as_bytes())
            .unwrap_err();
        assert!(error.is_deposit_wallet_reconciliation_required(), "{label}: {error}");
        assert!(
            error.to_string().contains(expected_message),
            "{label} produced unexpected error: {error}"
        );
        assert!(!error.to_string().contains(OTHER_OWNER));
    }
}

#[tokio::test]
async fn submit_wallet_create_posts_exact_fixture_and_accepts_missing_hash() {
    let response = json!({
        "transactionID": "tx-wallet-create-new",
        "state": "STATE_NEW"
    });
    let (url, handle) = spawn_server(vec![TestResponse::json("200 OK", response.to_string())]).await;
    let client = amoy_test_client(url);

    let receipt = client.submit_wallet_create(address(WALLET_OWNER)).await.unwrap();

    assert_eq!(receipt.transaction_id, "tx-wallet-create-new");
    assert_eq!(receipt.state, RelayerTransactionState::New);
    assert_eq!(receipt.transaction_hash, None);
    assert_eq!(receipt.owner, Some(address(WALLET_OWNER)));
    assert_eq!(
        receipt.deposit_wallet,
        Some(
            derive_deposit_wallet_address(
                address(WALLET_OWNER),
                deposit_wallet_contract_config(80002).unwrap()
            )
            .unwrap()
        )
    );
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_submit_request_headers(&requests[0]);
    assert_json_body_matches_fixture(&requests[0].body, "wallet_create_submit_body.json");
}

#[tokio::test]
async fn submit_signed_wallet_batch_posts_exact_fixture_and_accepts_missing_hash() {
    let response = json!({
        "transactionID": "tx-wallet-batch-new",
        "state": "STATE_NEW"
    });
    let (url, handle) = spawn_server(vec![TestResponse::json("200 OK", response.to_string())]).await;
    let client = amoy_test_client(url);
    let signed = signed_batch_fixture("wallet_batch_eip712_amoy.json");
    let expected_owner = signed.submit_from();
    let expected_wallet = signed.deposit_wallet();

    let receipt = client.submit_signed_wallet_batch(signed).await.unwrap();

    assert_eq!(receipt.transaction_id, "tx-wallet-batch-new");
    assert_eq!(receipt.state, RelayerTransactionState::New);
    assert_eq!(receipt.transaction_hash, None);
    assert_eq!(receipt.owner, Some(expected_owner));
    assert_eq!(receipt.deposit_wallet, Some(expected_wallet));
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_submit_request_headers(&requests[0]);
    assert_json_body_matches_fixture(&requests[0].body, "wallet_signed_submit_body_amoy.json");
}

#[test]
fn submit_response_parser_accepts_optional_transaction_hash() {
    let response = br#"{
        "transactionID":"tx-submit-with-hash",
        "state":"STATE_MINED",
        "transactionHash":"0X38CBFBEAE8FFFA4E2B187EE5978D3EE9CAFC53AF0363ED90A35B7EA9016535D8"
    }"#;

    let receipt = super::response::parse_submit_response(response, None, None).unwrap();

    assert_eq!(receipt.transaction_id, "tx-submit-with-hash");
    assert_eq!(receipt.state, RelayerTransactionState::Mined);
    assert_eq!(
        receipt.transaction_hash.as_deref(),
        Some("0x38cbfbeae8fffa4e2b187ee5978d3ee9cafc53af0363ed90a35b7ea9016535d8")
    );
}

#[tokio::test]
async fn submit_refuses_non_amoy_targets_before_network() {
    let (url, handle) = spawn_server(Vec::new()).await;
    let mock_mismatch_client = test_client(url);

    let mock_error = mock_mismatch_client
        .submit_wallet_create(address(WALLET_OWNER))
        .await
        .unwrap_err();

    assert!(mock_error.is_deposit_wallet_mutation_blocked());
    assert!(mock_error.to_string().contains("chain 80002"));
    let requests = handle.await.unwrap();
    assert!(requests.is_empty());

    let polygon_client = DepositWalletRelayerClient::new(
        DepositWalletRelayerUrl::parse("https://relayer-v2.polymarket.com").unwrap(),
        relayer_auth(),
        deposit_wallet_contract_config(137).unwrap(),
    )
    .unwrap();
    let polygon_error = polygon_client
        .submit_wallet_create(address(WALLET_OWNER))
        .await
        .unwrap_err();
    assert!(polygon_error.is_deposit_wallet_mutation_blocked());
    assert!(polygon_error.to_string().contains("Amoy relayer host"));

    let amoy_url = DepositWalletRelayerUrl::parse("https://relayer-v2-staging.polymarket.dev")
        .unwrap();
    let bad_config_error =
        DepositWalletRelayerClient::new(amoy_url, relayer_auth(), deposit_wallet_contract_config(137).unwrap())
            .unwrap_err();
    assert!(bad_config_error.to_string().contains("chain 80002"));
}

#[tokio::test]
async fn submit_429_preserves_retry_after_without_body_or_secret_leak() {
    let response_body = r#"{"error":"leaky-response-body","detail":"depositWalletParams"}"#;
    let (url, handle) = spawn_server(vec![
        TestResponse::json("429 Too Many Requests", response_body).with_header("retry-after", "7"),
    ])
    .await;
    let client = amoy_test_client(url);
    let signed = signed_batch_fixture("wallet_batch_eip712_amoy.json");
    let raw_signature = signed.signature().to_string();

    let error = client.submit_signed_wallet_batch(signed).await.unwrap_err();

    assert!(matches!(
        error,
        RelayerError::Api { status: 429, ref message }
            if message.contains("retry after 7s")
    ));
    let error_display = error.to_string();
    let error_debug = format!("{error:?}");
    for rendered in [&error_display, &error_debug] {
        assert!(!rendered.contains("leaky-response-body"));
        assert!(!rendered.contains("depositWalletParams"));
        assert!(!rendered.contains(API_KEY));
        assert!(!rendered.contains(&raw_signature));
    }
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_submit_request_headers(&requests[0]);
    assert_json_body_matches_fixture(&requests[0].body, "wallet_signed_submit_body_amoy.json");
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
    for raw in [
        "{}",
        r#"{"nonce":null}"#,
        "[]",
        "not-json",
        r#""31""#,
    ] {
        assert!(
            super::read::parse_wallet_nonce_response(raw.as_bytes()).is_err(),
            "expected nonce response shape {raw:?} to be rejected"
        );
    }
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
fn transaction_response_fixture_preserves_proxy_address_evidence() {
    let transaction_id = "0190b317-a1d3-7bec-9b91-eeb6dcd3a620";
    let fixture = fixture_text("wallet_transaction_response.json");

    let parsed = parse_transaction_response(
        transaction_id,
        deposit_wallet_contract_config(137).unwrap(),
        fixture.as_bytes(),
    )
    .unwrap();

    assert_eq!(parsed.receipt.transaction_id, transaction_id);
    assert_eq!(parsed.receipt.state, RelayerTransactionState::Confirmed);
    assert_eq!(parsed.owner, Some(address(WALLET_OWNER)));
    assert_eq!(
        parsed.receipt.deposit_wallet,
        Some("0x069F89dAEfbaDdF5B6639Dc34D73E59cCCBC63De".parse().unwrap())
    );
}

#[test]
fn transaction_response_receipt_debug_redacts_owner_wallet_hash_and_id() {
    let transaction_id = "tx-debug-redaction";
    let response = transaction_response_value(transaction_id, "STATE_CONFIRMED");

    let parsed = parse_transaction_response(
        transaction_id,
        deposit_wallet_contract_config(137).unwrap(),
        response.to_string().as_bytes(),
    )
    .unwrap();

    assert_eq!(parsed.receipt.transaction_id, transaction_id);
    assert_eq!(parsed.receipt.state, RelayerTransactionState::Confirmed);
    assert_eq!(parsed.owner, Some(address(WALLET_OWNER)));
    assert_eq!(parsed.receipt.owner, Some(address(WALLET_OWNER)));
    assert_eq!(
        parsed.receipt.deposit_wallet,
        Some(
            derive_deposit_wallet_address(
                address(WALLET_OWNER),
                deposit_wallet_contract_config(137).unwrap()
            )
            .unwrap()
        )
    );
    let debug = format!("{:?}", parsed.receipt);
    let owner_checksum = to_checksum(&address(WALLET_OWNER), None);
    let wallet_checksum =
        to_checksum(&parsed.receipt.deposit_wallet.expect("wallet evidence"), None);
    let transaction_hash =
        "0x38cbfbeae8fffa4e2b187ee5978d3ee9cafc53af0363ed90a35b7ea9016535d8";
    assert!(!debug.contains(transaction_id));
    assert!(!debug.contains(&owner_checksum));
    assert!(!debug.contains(&owner_checksum.to_ascii_lowercase()));
    assert!(!debug.contains(&wallet_checksum));
    assert!(!debug.contains(&wallet_checksum.to_ascii_lowercase()));
    assert!(!debug.contains(transaction_hash));
    assert!(!debug.contains(&transaction_hash.to_ascii_uppercase()));
}

#[tokio::test]
async fn poll_transaction_for_owner_waits_until_confirmed_before_success() {
    let transaction_id = "tx-poll-confirmed";
    let (url, handle) = spawn_server(vec![
        TestResponse::json(
            "200 OK",
            transaction_response_value(transaction_id, "STATE_NEW").to_string(),
        ),
        TestResponse::json(
            "200 OK",
            transaction_response_value(transaction_id, "STATE_MINED").to_string(),
        ),
        TestResponse::json(
            "200 OK",
            transaction_response_value(transaction_id, "STATE_EXECUTED").to_string(),
        ),
        TestResponse::json(
            "200 OK",
            transaction_response_value(transaction_id, "STATE_CONFIRMED").to_string(),
        ),
    ])
    .await;
    let client = test_client(url);

    let receipt = client
        .poll_transaction_for_owner_with_config(
            address(WALLET_OWNER),
            transaction_id,
            short_polling_config(),
        )
        .await
        .unwrap();

    assert_eq!(receipt.transaction_id, transaction_id);
    assert_eq!(receipt.state, RelayerTransactionState::Confirmed);
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 4);
    for request in requests {
        assert_eq!(request.method, "GET");
        assert_eq!(request.path, "/transaction?id=tx-poll-confirmed");
        assert!(request.body.is_empty());
    }
}

#[tokio::test]
async fn get_transaction_for_owner_rejects_invalid_transaction_id_before_http() {
    let (url, handle) = spawn_server(Vec::new()).await;
    let client = test_client(url);

    for transaction_id in ["", " tx-leading-space", "tx-trailing-space ", "tx\nnewline"] {
        let error = client
            .get_transaction_for_owner(address(WALLET_OWNER), transaction_id)
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("transaction id"),
            "{transaction_id:?}: {error}"
        );
    }

    let requests = handle.await.unwrap();
    assert!(requests.is_empty());
}

#[tokio::test]
async fn transport_sends_json_body_with_auth_headers() {
    let (url, handle) = spawn_server(vec![TestResponse::json("200 OK", "{}")]).await;
    let endpoint = url.endpoint("/transport-body");
    let client = test_client(url);
    let body = r#"{"ping":true}"#.to_string();

    let response = client
        .send(Method::POST, endpoint, Some(body.clone()))
        .await
        .unwrap();

    assert_eq!(response, b"{}");
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "POST");
    assert_eq!(requests[0].path, "/transport-body");
    assert_eq!(requests[0].header("content-type"), Some("application/json"));
    assert_eq!(requests[0].header("RELAYER_API_KEY"), Some(API_KEY));
    assert_eq!(
        requests[0].header("RELAYER_API_KEY_ADDRESS"),
        Some(to_checksum(&address(API_KEY_ADDRESS), None).as_str())
    );
    assert_eq!(requests[0].body, body);
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
    assert_eq!(
        receipt.deposit_wallet,
        Some(
            derive_deposit_wallet_address(
                address(WALLET_OWNER),
                deposit_wallet_contract_config(137).unwrap()
            )
            .unwrap()
        )
    );
    let requests = handle.await.unwrap();
    assert_eq!(requests[0].method, "GET");
    assert_eq!(requests[0].path, "/transaction?id=tx-owner");
    assert_eq!(requests[0].header("RELAYER_API_KEY"), Some(API_KEY));
    assert_eq!(
        requests[0].header("RELAYER_API_KEY_ADDRESS"),
        Some(to_checksum(&address(API_KEY_ADDRESS), None).as_str())
    );
    assert!(requests[0].body.is_empty());
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

async fn poll_transaction_state_error(transaction_id: &str, state: &str) -> RelayerError {
    let (url, handle) = spawn_server(vec![TestResponse::json(
        "200 OK",
        transaction_response_value(transaction_id, state).to_string(),
    )])
    .await;
    let client = test_client(url);

    let error = client
        .poll_transaction_for_owner_with_config(
            address(WALLET_OWNER),
            transaction_id,
            short_polling_config(),
        )
        .await
        .unwrap_err();
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 1);
    error
}

#[tokio::test]
async fn poll_transaction_for_owner_rejects_failed_invalid_and_unknown_states() {
    let invalid = poll_transaction_state_error("tx-poll-invalid", "STATE_INVALID").await;
    assert!(matches!(invalid, RelayerError::TransactionInvalid(_)));

    let failed = poll_transaction_state_error("tx-poll-failed", "STATE_FAILED").await;
    assert!(matches!(failed, RelayerError::TransactionFailed(_)));

    let unknown = poll_transaction_state_error("tx-poll-unknown", "STATE_FUTURE").await;
    assert!(unknown.is_deposit_wallet_reconciliation_required());
    assert!(!unknown.to_string().contains("STATE_FUTURE"));
}

#[tokio::test]
async fn poll_transaction_for_owner_returns_non_success_errors_for_bad_or_pending_responses() {
    let (url, handle) = spawn_server(vec![TestResponse::json("200 OK", "[]")]).await;
    let client = test_client(url);
    let empty = client
        .poll_transaction_for_owner_with_config(
            address(WALLET_OWNER),
            "tx-empty-array",
            short_polling_config(),
        )
        .await
        .unwrap_err();
    assert!(empty.is_deposit_wallet_transaction_absent());
    let _ = handle.await.unwrap();

    let (url, handle) = spawn_server(vec![TestResponse::json(
        "200 OK",
        json!([transaction_response_value("other-tx", "STATE_CONFIRMED")]).to_string(),
    )])
    .await;
    let client = test_client(url);
    let absent = client
        .poll_transaction_for_owner_with_config(
            address(WALLET_OWNER),
            "tx-absent-id",
            short_polling_config(),
        )
        .await
        .unwrap_err();
    assert!(absent.is_deposit_wallet_transaction_absent());
    let _ = handle.await.unwrap();

    let (url, handle) = spawn_server(vec![TestResponse::json("200 OK", "not-json")]).await;
    let client = test_client(url);
    let malformed = client
        .poll_transaction_for_owner_with_config(
            address(WALLET_OWNER),
            "tx-malformed",
            short_polling_config(),
        )
        .await
        .unwrap_err();
    assert!(matches!(malformed, RelayerError::Other(ref message) if message.contains("could not parse transaction response")));
    let _ = handle.await.unwrap();

    let timeout_config = DepositWalletPollingConfig {
        initial_backoff: Duration::from_millis(20),
        max_backoff: Duration::from_millis(20),
        timeout: Duration::from_millis(5),
    };
    let mut pending_response = transaction_response_value("tx-timeout", "STATE_MINED");
    pending_response.as_object_mut().unwrap().remove("transactionHash");
    let (url, handle) = spawn_server(vec![TestResponse::json(
        "200 OK",
        pending_response.to_string(),
    )])
    .await;
    let client = test_client(url);
    let timeout = client
        .poll_transaction_for_owner_with_config(address(WALLET_OWNER), "tx-timeout", timeout_config)
        .await
        .unwrap_err();
    assert!(matches!(timeout, RelayerError::Timeout));
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 1);
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
    assert!(error.is_deposit_wallet_transaction_absent());
    assert!(error.to_string().contains("did not include requested transaction id hash"));
    let _ = handle.await.unwrap();

    let (url, handle) = spawn_reset_server().await;
    let client = test_client(url);
    let error = client
        .get_transaction_for_owner(address(WALLET_OWNER), "tx-reset")
        .await
        .unwrap_err();
    assert!(matches!(error, RelayerError::Http(_)));
    let error_message = error.to_string();
    assert!(!error_message.contains("tx-reset"));
    assert!(!error_message.contains("/transaction"));
    assert!(!error_message.contains("127.0.0.1"));
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
        deposit_wallet_contract_config(137).unwrap(),
        matching.as_bytes(),
    )
    .unwrap();
    assert_eq!(parsed.receipt.transaction_id, target);

    let mut invalid_id_non_target =
        transaction_response_value("other-before-target", "STATE_CONFIRMED");
    invalid_id_non_target["transactionID"] = json!("bad\ntransaction");
    let body = json!([
        invalid_id_non_target,
        transaction_response_value(target, "STATE_CONFIRMED")
    ])
    .to_string();
    let parsed = parse_transaction_response(
        target,
        deposit_wallet_contract_config(137).unwrap(),
        body.as_bytes(),
    )
    .unwrap();
    assert_eq!(parsed.receipt.transaction_id, target);

    let mut malformed_non_target = transaction_response_value("other-before-target", "STATE_CONFIRMED");
    malformed_non_target["from"] = json!(137);
    let body = json!([
        malformed_non_target,
        transaction_response_value(target, "STATE_CONFIRMED")
    ])
    .to_string();
    let parsed = parse_transaction_response(
        target,
        deposit_wallet_contract_config(137).unwrap(),
        body.as_bytes(),
    )
    .unwrap();
    assert_eq!(parsed.receipt.transaction_id, target);

    let error = parse_transaction_response(
        target,
        deposit_wallet_contract_config(137).unwrap(),
        missing.as_bytes(),
    )
    .unwrap_err()
    .error;
    assert!(error.is_deposit_wallet_transaction_absent());
    assert!(error.to_string().contains("did not include requested transaction id"));

    let trailing = format!("{matching} {{}}");
    let error = parse_transaction_response(
        target,
        deposit_wallet_contract_config(137).unwrap(),
        trailing.as_bytes(),
    )
    .unwrap_err()
    .error;
    assert!(matches!(
        error,
        RelayerError::Other(ref message) if message.contains("could not parse transaction response array")
    ));

    let object_mismatch = transaction_response_value("other-object-id", "STATE_CONFIRMED");
    let error = parse_transaction_response(
        target,
        deposit_wallet_contract_config(137).unwrap(),
        object_mismatch.to_string().as_bytes(),
    )
    .unwrap_err()
    .error;
    assert!(error.is_deposit_wallet_reconciliation_required());
    assert!(error.to_string().contains("did not match requested id hash"));
    assert!(!error.to_string().contains("other-object-id"));

    for (fixture_key, expected_message) in [
        ("duplicateIds", "duplicate requested transaction id hash"),
        ("oversizedIds", "included more than"),
    ] {
        let body = body_from_ids(&fixture[fixture_key]);
        let error = parse_transaction_response(
            target,
            deposit_wallet_contract_config(137).unwrap(),
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

    let body = body_from_ids(&fixture["invalidIds"]);
    let error = parse_transaction_response(
        target,
        deposit_wallet_contract_config(137).unwrap(),
        body.as_bytes(),
    )
    .unwrap_err()
    .error;
    assert!(error.is_deposit_wallet_transaction_absent());
    assert!(error.to_string().contains("did not include requested transaction id"));
}

#[test]
fn transaction_response_rejects_malformed_transaction_hashes() {
    let config = deposit_wallet_contract_config(137).unwrap();
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
            config,
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
fn transaction_response_rejects_partial_required_fields() {
    let target = "tx-partial";
    let config = deposit_wallet_contract_config(137).unwrap();

    let mut missing_transaction_id = transaction_response_value(target, "STATE_CONFIRMED");
    missing_transaction_id
        .as_object_mut()
        .unwrap()
        .remove("transactionID");
    let mut non_string_transaction_id = transaction_response_value(target, "STATE_CONFIRMED");
    non_string_transaction_id["transactionID"] = json!(137);
    let mut missing_state = transaction_response_value(target, "STATE_CONFIRMED");
    missing_state.as_object_mut().unwrap().remove("state");
    let mut non_string_state = transaction_response_value(target, "STATE_CONFIRMED");
    non_string_state["state"] = json!(137);

    for (label, response) in [
        ("missing transactionID", missing_transaction_id),
        ("non-string transactionID", non_string_transaction_id),
        ("missing state", missing_state),
        ("non-string state", non_string_state),
    ] {
        let object_error = parse_transaction_response(target, config, response.to_string().as_bytes())
            .unwrap_err()
            .error;
        assert!(
            matches!(object_error, RelayerError::Other(ref message) if message.contains("could not parse transaction response object")),
            "{label}: {object_error}"
        );

        let array_error = parse_transaction_response(
            target,
            config,
            json!([response]).to_string().as_bytes(),
        )
        .unwrap_err()
        .error;
        match label {
            "missing transactionID" | "non-string transactionID" => {
                assert!(array_error.is_deposit_wallet_transaction_absent(), "{label}: {array_error}");
            }
            "missing state" | "non-string state" => {
                assert!(
                    matches!(array_error, RelayerError::Other(ref message) if message.contains("could not parse transaction response array")),
                    "{label}: {array_error}"
                );
            }
            _ => unreachable!("unexpected partial-field case"),
        }
    }
}

#[test]
fn transaction_response_normalizes_valid_transaction_hashes() {
    let transaction_id = "tx-normalized-hash";
    let config = deposit_wallet_contract_config(137).unwrap();
    let mut response = transaction_response_value(transaction_id, "STATE_CONFIRMED");
    response["transactionHash"] =
        json!("0X38CBFBEAE8FFFA4E2B187EE5978D3EE9CAFC53AF0363ED90A35B7EA9016535D8");

    let parsed =
        parse_transaction_response(transaction_id, config, response.to_string().as_bytes())
            .unwrap();

    assert_eq!(
        parsed.receipt.transaction_hash.as_deref(),
        Some("0x38cbfbeae8fffa4e2b187ee5978d3ee9cafc53af0363ed90a35b7ea9016535d8")
    );
}

#[test]
fn transaction_response_accepts_wallet_create_type_for_deploy_polling() {
    let transaction_id = "tx-wallet-create-poll";
    let config = deposit_wallet_contract_config(137).unwrap();
    let mut response = transaction_response_value(transaction_id, "STATE_CONFIRMED");
    response["type"] = json!(crate::deposit_wallet::WALLET_CREATE_TRANSACTION_TYPE);

    let parsed = parse_transaction_response(transaction_id, config, response.to_string().as_bytes())
        .unwrap();

    assert_eq!(parsed.receipt.transaction_id, transaction_id);
    assert_eq!(parsed.receipt.state, RelayerTransactionState::Confirmed);
}

#[test]
fn transaction_response_rejects_malformed_address_evidence() {
    let config = deposit_wallet_contract_config(137).unwrap();
    for (label, field, value) in [
        ("from-empty", "from", json!("")),
        (
            "from-missing-prefix",
            "from",
            json!(WALLET_OWNER.trim_start_matches("0x")),
        ),
        (
            "from-uppercase-prefix",
            "from",
            json!(format!("0X{}", &WALLET_OWNER[2..])),
        ),
        ("from-number", "from", json!(137)),
        ("from-object", "from", json!({"address": WALLET_OWNER})),
        ("to-empty", "to", json!("")),
        ("to-number", "to", json!(137)),
        ("owner-empty", "owner", json!("")),
        ("owner-number", "owner", json!(137)),
        ("proxy-empty", "proxyAddress", json!("")),
        ("proxy-number", "proxyAddress", json!(137)),
    ] {
        let transaction_id = format!("tx-bad-address-{label}");
        let mut response = transaction_response_value(&transaction_id, "STATE_CONFIRMED");
        response[field] = value;

        let object_error = parse_transaction_response(
            &transaction_id,
            config,
            response.to_string().as_bytes(),
        )
        .unwrap_err()
        .error;

        assert!(
            object_error.is_deposit_wallet_reconciliation_required(),
            "{label}: {object_error}"
        );
        assert!(
            object_error.to_string().contains("address evidence"),
            "{label}: {object_error}"
        );

        let array_error = parse_transaction_response(
            &transaction_id,
            config,
            json!([transaction_response_value("other-tx", "STATE_CONFIRMED"), response])
                .to_string()
                .as_bytes(),
        )
        .unwrap_err()
        .error;
        assert!(
            array_error.is_deposit_wallet_reconciliation_required(),
            "{label}: {array_error}"
        );
        assert!(
            array_error.to_string().contains("address evidence"),
            "{label}: {array_error}"
        );
    }
}

#[test]
fn transaction_response_rejects_unproven_wire_evidence_boundaries() {
    let target = "tx-wire-evidence";
    let config = deposit_wallet_contract_config(137).unwrap();
    let other_address = to_checksum(&address(OTHER_OWNER), None);
    let mut missing_type = transaction_response_value(target, "STATE_CONFIRMED");
    missing_type.as_object_mut().unwrap().remove("type");
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
    let mut missing_proxy = transaction_response_value(target, "STATE_CONFIRMED");
    missing_proxy.as_object_mut().unwrap().remove("proxyAddress");
    let mut proxy_mismatch = transaction_response_value(target, "STATE_CONFIRMED");
    proxy_mismatch["proxyAddress"] = json!(other_address);

    for (label, response, expected_message) in [
        (
            "missing type",
            missing_type,
            "did not include deposit-wallet transaction type",
        ),
        (
            "wrong type",
            wrong_type,
            "type was not WALLET or WALLET-CREATE",
        ),
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
        ("to mismatch", to_mismatch, "did not match configured factory"),
        (
            "missing proxyAddress",
            missing_proxy,
            "did not include proxyAddress deposit wallet evidence",
        ),
        (
            "proxyAddress mismatch",
            proxy_mismatch,
            "did not match derived deposit wallet",
        ),
    ] {
        let error =
            parse_transaction_response(target, config, response.to_string().as_bytes())
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

        let array_error = parse_transaction_response(
            target,
            config,
            json!([transaction_response_value("other-tx", "STATE_CONFIRMED"), response])
                .to_string()
                .as_bytes(),
        )
        .unwrap_err()
        .error;
        assert!(
            array_error.is_deposit_wallet_reconciliation_required(),
            "{label} array: {array_error}"
        );
        assert!(
            array_error.to_string().contains(expected_message),
            "{label} array produced unexpected error: {array_error}"
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

    headers.insert(
        RETRY_AFTER,
        HeaderValue::from_str(&httpdate::fmt_http_date(UNIX_EPOCH + Duration::from_secs(1)))
            .unwrap(),
    );
    assert_eq!(
        super::transport::retry_after_duration_at(&headers, now),
        Some(Duration::ZERO)
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
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 1);

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
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 1);
}

#[tokio::test]
async fn transaction_read_uses_transaction_body_limit() {
    let mut large_response =
        transaction_response_value("tx-large-response", "STATE_CONFIRMED");
    large_response["padding"] = json!("x".repeat(MAX_SUCCESS_BODY_BYTES + 1));
    let (url, handle) =
        spawn_server(vec![TestResponse::json("200 OK", large_response.to_string())]).await;
    let client = test_client(url);

    let receipt = client
        .get_transaction_for_owner(address(WALLET_OWNER), "tx-large-response")
        .await
        .unwrap();

    assert_eq!(receipt.transaction_id, "tx-large-response");
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 1);

    let content_length_too_large =
        TestResponse::json_without_content_length("200 OK", "").with_header(
            "content-length",
            (MAX_TRANSACTION_SUCCESS_BODY_BYTES + 1).to_string(),
        );
    let (url, handle) = spawn_server(vec![content_length_too_large]).await;
    let client = test_client(url);
    let error = client
        .get_transaction_for_owner(address(WALLET_OWNER), "tx-too-large-response")
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        RelayerError::Other(ref message) if message.contains("maximum size")
    ));
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 1);
}

#[tokio::test]
async fn error_body_drain_limiter_releases_permits_and_handles_exhaustion() {
    let large_error_body = "x".repeat(MAX_ERROR_BODY_DRAIN_BYTES + 1024);
    let (url, handle) = spawn_server(vec![TestResponse::json_without_content_length(
        "500 Internal Server Error",
        large_error_body.clone(),
    )])
    .await;
    let response = reqwest_client(Duration::from_secs(2))
        .get(url.endpoint("/error"))
        .send()
        .await
        .unwrap();
    let limiter = ErrorBodyDrainLimiter::new(1);

    let drain = limiter
        .try_spawn_error_response_body_drain_for_test(response)
        .unwrap();
    drain.await.unwrap();
    assert_eq!(limiter.available_permits(), 1);
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 1);

    let (url, handle) = spawn_server(vec![TestResponse::json_without_content_length(
        "500 Internal Server Error",
        large_error_body,
    )])
    .await;
    let response = reqwest_client(Duration::from_secs(2))
        .get(url.endpoint("/error"))
        .send()
        .await
        .unwrap();
    let exhausted_limiter = ErrorBodyDrainLimiter::new(0);

    assert!(exhausted_limiter
        .try_spawn_error_response_body_drain_for_test(response)
        .is_none());
    assert_eq!(exhausted_limiter.available_permits(), 0);
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 1);
}

#[test]
fn transaction_id_validation_covers_bounds_and_opaque_ids() {
    let max_len = "a".repeat(MAX_TRANSACTION_ID_LEN);
    assert_eq!(validate_transaction_id(&max_len).unwrap(), max_len);
    assert_eq!(
        validate_transaction_id("tx:/opaque+id=?value").unwrap(),
        "tx:/opaque+id=?value"
    );

    let too_long = "a".repeat(MAX_TRANSACTION_ID_LEN + 1);
    assert!(validate_transaction_id(&too_long).is_err());
    assert!(validate_transaction_id("").is_err());
    assert!(validate_transaction_id(" tx-leading-space").is_err());
    assert!(validate_transaction_id("tx-trailing-space ").is_err());
    assert!(validate_transaction_id("tx\nabc").is_err());
}
