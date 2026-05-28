use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use ethers::types::Bytes;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::deposit_wallet::{
    deposit_wallet_contract_config, derive_deposit_wallet_address,
    validate_deposit_wallet_batch_signature, DepositWalletBatchToSign, DepositWalletCall,
    WALLET_CREATE_TRANSACTION_TYPE, WALLET_TRANSACTION_TYPE,
};

use super::response::{
    extract_submit_transaction_id, parse_submit_response, parse_transaction_response,
    validate_transaction_id,
};
use super::state::{
    ensure_owner_mutation_capacity, OwnerMutationBlock, OwnerMutationState, OwnerTransactionRecord,
};
use super::*;

const API_KEY: &str = "unit-test-relayer-api-key";
const API_KEY_ADDRESS: &str = "0xA6Db23622C9EA7584D5c61C3e7497c80E2CE167B";
const WALLET_OWNER: &str = "0x6e0c80c90ea6c15917308F820Eac91Ce2724B5b5";
const OTHER_OWNER: &str = "0x0000000000000000000000000000000000000001";
const TEST_SERVER_TIMEOUT: Duration = Duration::from_secs(2);
const REDIRECT_TARGET_NEGATIVE_TIMEOUT: Duration = Duration::from_millis(250);

#[derive(Clone)]
struct FixedClock {
    now: Arc<AtomicU64>,
}

impl DepositWalletClock for FixedClock {
    fn now_unix_seconds(&self) -> Result<u64> {
        Ok(self.now.load(Ordering::SeqCst))
    }
}

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

fn assert_url_error_contains(result: Result<DepositWalletRelayerUrl>, expected: &str) {
    let error = result.expect_err("URL should be rejected");
    match error {
        RelayerError::Other(message) => {
            assert!(
                message.starts_with("Invalid relayer URL:"),
                "expected invalid relayer URL error, got {message}"
            );
            assert!(
                message.contains(expected),
                "expected {expected:?} in {message}"
            );
        }
        other => panic!("expected invalid relayer URL error, got {other}"),
    }
}

fn relayer_auth() -> RelayerKeyAuth {
    RelayerKeyAuth::new(API_KEY, address(API_KEY_ADDRESS)).unwrap()
}

fn mutation_scope(action: DepositWalletMutationAction) -> DepositWalletMutationScope {
    let config = deposit_wallet_contract_config(137).unwrap();
    DepositWalletMutationScope::new(
        137,
        config.factory,
        config.implementation,
        DepositWalletMutationEnvironment::TestLoopback,
        action,
    )
}

fn owner_serialization_evidence_for_action(
    owner: Address,
    action: DepositWalletMutationAction,
) -> DepositWalletOwnerSerializationEvidence {
    DepositWalletOwnerSerializationEvidence::new(
        owner,
        mutation_scope(action),
        "unit-test owner serialization guard",
        format!("unit-test-owner-lease-{owner:?}"),
        1_699_999_900,
        1_700_000_200,
    )
    .unwrap()
}

fn mutation_permit_token_for_action(
    owner: Address,
    action: DepositWalletMutationAction,
) -> DepositWalletMutationPermit {
    DepositWalletMutationPermit::from_owner_serialization_evidence(
        "mocked unit-test relayer call",
        owner_serialization_evidence_for_action(owner, action),
    )
    .unwrap()
}

fn mutation_permit_for_action(
    owner: Address,
    action: DepositWalletMutationAction,
) -> DepositWalletMutationGate {
    DepositWalletMutationGate::Permit(mutation_permit_token_for_action(owner, action))
}

fn mutation_permit_for_action_at(
    owner: Address,
    action: DepositWalletMutationAction,
    acquired_at_unix_seconds: u64,
    expires_at_unix_seconds: u64,
) -> DepositWalletMutationGate {
    let evidence = DepositWalletOwnerSerializationEvidence::new(
        owner,
        mutation_scope(action),
        "unit-test owner serialization guard",
        format!("unit-test-owner-lease-{owner:?}-{acquired_at_unix_seconds}"),
        acquired_at_unix_seconds,
        expires_at_unix_seconds,
    )
    .unwrap();
    DepositWalletMutationGate::Permit(
        DepositWalletMutationPermit::from_owner_serialization_evidence(
            "mocked unit-test relayer call",
            evidence,
        )
        .unwrap(),
    )
}

fn wallet_nonce_read_permit_for(owner: Address) -> DepositWalletMutationGate {
    mutation_permit_for_action(owner, DepositWalletMutationAction::WalletNonceRead)
}

fn wallet_create_permit_for(owner: Address) -> DepositWalletMutationGate {
    mutation_permit_for_action(owner, DepositWalletMutationAction::WalletCreate)
}

fn wallet_batch_permit_for(owner: Address) -> DepositWalletMutationGate {
    mutation_permit_for_action(owner, DepositWalletMutationAction::WalletBatch)
}

fn manual_reconciliation_permit_token_for(owner: Address) -> DepositWalletMutationPermit {
    mutation_permit_token_for_action(owner, DepositWalletMutationAction::ManualReconciliation)
}

fn reqwest_client(timeout: Duration) -> Client {
    Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(timeout)
        .build()
        .expect("test HTTP client should build")
}

fn test_client_with_clock(
    base_url: DepositWalletRelayerUrl,
    now: u64,
) -> (DepositWalletRelayerClient, Arc<AtomicU64>) {
    let clock = Arc::new(AtomicU64::new(now));
    let client = DepositWalletRelayerClient::from_parts(
        reqwest_client(Duration::from_secs(2)),
        base_url,
        relayer_auth(),
        deposit_wallet_contract_config(137).unwrap(),
        Arc::new(FixedClock {
            now: Arc::clone(&clock),
        }),
    );
    (client, clock)
}

fn test_client(base_url: DepositWalletRelayerUrl) -> DepositWalletRelayerClient {
    test_client_with_clock(base_url, 1_700_000_000).0
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

async fn spawn_delayed_response_server(
    response: TestResponse,
) -> (
    DepositWalletRelayerUrl,
    JoinHandle<Vec<CapturedRequest>>,
    oneshot::Receiver<()>,
    oneshot::Sender<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("test server should bind");
    let addr = listener.local_addr().unwrap();
    let (accepted_tx, accepted_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let handle = tokio::spawn(async move {
        let (mut stream, _) = tokio::time::timeout(TEST_SERVER_TIMEOUT, listener.accept())
            .await
            .expect("server accept should not hang")
            .expect("server should accept");
        let request = read_request(&mut stream).await;
        let _ = accepted_tx.send(());
        release_rx.await.expect("test should release response");
        write_response(&mut stream, response).await;
        vec![request]
    });

    (
        DepositWalletRelayerUrl::loopback(&format!("http://{addr}")).unwrap(),
        handle,
        accepted_rx,
        release_tx,
    )
}

async fn spawn_redirect_target_probe() -> (String, JoinHandle<CapturedRequest>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("test server should bind");
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("redirect target should accept if followed");
        read_request(&mut stream).await
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

fn wallet_create_transaction_response_value(transaction_id: &str, state: &str) -> Value {
    let mut value = transaction_response_value(transaction_id, state);
    value["type"] = json!(WALLET_CREATE_TRANSACTION_TYPE);
    value
}

fn parse_u256(value: &Value, key: &str) -> U256 {
    U256::from_dec_str(value[key].as_str().unwrap()).unwrap()
}

fn call_from_value(value: &Value) -> DepositWalletCall {
    DepositWalletCall {
        target: value["target"].as_str().unwrap().parse().unwrap(),
        value: U256::from_dec_str(value["value"].as_str().unwrap()).unwrap(),
        data: Bytes::from(hex::decode(&value["data"].as_str().unwrap()[2..]).unwrap()),
    }
}

fn signed_wallet_batch_fixture() -> SignedDepositWalletBatch {
    let data = fixture_value("wallet_batch_eip712.json");
    let batch = DepositWalletBatchToSign {
        owner: data["owner"].as_str().unwrap().parse().unwrap(),
        nonce_owner: data["nonceOwner"].as_str().unwrap().parse().unwrap(),
        submit_from: data["submitFrom"].as_str().unwrap().parse().unwrap(),
        deposit_wallet: data["depositWallet"].as_str().unwrap().parse().unwrap(),
        chain_id: data["chainId"].as_u64().unwrap(),
        nonce: parse_u256(&data, "nonce"),
        deadline: parse_u256(&data, "deadline"),
        calls: data["calls"]
            .as_array()
            .unwrap()
            .iter()
            .map(call_from_value)
            .collect(),
    };
    validate_deposit_wallet_batch_signature(batch, data["ownerSignature"].as_str().unwrap())
        .unwrap()
}

fn manual_reconciliation_evidence(
    owner: Address,
    payload_hash: impl Into<String>,
    transaction_id: &str,
    observed_state: RelayerTransactionState,
    transaction_hash: Option<&str>,
) -> DepositWalletSubmitReconciliationEvidence {
    manual_reconciliation_evidence_at(
        owner,
        payload_hash,
        transaction_id,
        observed_state,
        transaction_hash,
        1_700_000_001,
    )
}

fn manual_reconciliation_evidence_at(
    owner: Address,
    payload_hash: impl Into<String>,
    transaction_id: &str,
    observed_state: RelayerTransactionState,
    transaction_hash: Option<&str>,
    checked_at_unix_seconds: u64,
) -> DepositWalletSubmitReconciliationEvidence {
    let observation = DepositWalletSubmitReconciliationObservation::new(
        transaction_id,
        observed_state,
        transaction_hash,
        "unit-test manual submit reconciliation",
        checked_at_unix_seconds,
    )
    .unwrap();
    DepositWalletSubmitReconciliationEvidence::new(
        owner,
        mutation_scope(DepositWalletMutationAction::ManualReconciliation),
        "unit-test owner serialization guard",
        payload_hash,
        observation,
    )
    .unwrap()
}

fn idless_reconciliation_evidence(
    owner: Address,
    payload_hash: impl Into<String>,
    checked_at_unix_seconds: u64,
) -> DepositWalletIdlessSubmitReconciliationEvidence {
    DepositWalletIdlessSubmitReconciliationEvidence::new(
        owner,
        mutation_scope(DepositWalletMutationAction::ManualReconciliation),
        "unit-test owner serialization guard",
        payload_hash,
        "unit-test relayer audit found no accepted transaction for the ambiguous payload",
        checked_at_unix_seconds,
    )
    .unwrap()
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
    assert_url_error_contains(
        DepositWalletRelayerUrl::parse("http://relayer-v2.polymarket.com"),
        "must use https",
    );
    assert_url_error_contains(
        DepositWalletRelayerUrl::parse("https://example.com"),
        "host is not allowlisted",
    );
    assert_url_error_contains(
        DepositWalletRelayerUrl::parse("https://relayer-v2.polymarket.com.evil.test"),
        "host is not allowlisted",
    );
    assert_url_error_contains(
        DepositWalletRelayerUrl::parse("https://evil-relayer-v2.polymarket.com"),
        "host is not allowlisted",
    );
    assert_url_error_contains(
        DepositWalletRelayerUrl::parse("https://relayer-v2.polymarket.com/path"),
        "must not include a path",
    );
    assert_url_error_contains(
        DepositWalletRelayerUrl::parse("https://user@relayer-v2.polymarket.com"),
        "must not include userinfo",
    );
    assert_url_error_contains(
        DepositWalletRelayerUrl::parse("https://relayer-v2.polymarket.com:8443"),
        "default HTTPS port",
    );
    assert_url_error_contains(
        DepositWalletRelayerUrl::parse("https://relayer-v2.polymarket.com?x=1"),
        "must not include query or fragment",
    );
    assert_url_error_contains(
        DepositWalletRelayerUrl::parse("https://relayer-v2.polymarket.com/#frag"),
        "must not include query or fragment",
    );
    assert!(DepositWalletRelayerUrl::loopback("http://[::1]/").is_ok());
    assert_url_error_contains(
        DepositWalletRelayerUrl::loopback("http://user@127.0.0.1/"),
        "must not include userinfo",
    );
    assert_url_error_contains(
        DepositWalletRelayerUrl::loopback("http://127.0.0.1/?x=1"),
        "must not include query or fragment",
    );
    assert_url_error_contains(
        DepositWalletRelayerUrl::loopback("http://127.0.0.1/#frag"),
        "must not include query or fragment",
    );
    assert_url_error_contains(
        DepositWalletRelayerUrl::loopback("http://127.0.0.1/path"),
        "must not include a path",
    );
    assert_url_error_contains(
        DepositWalletRelayerUrl::loopback("http://192.0.2.1/"),
        "loopback-only",
    );

    let production = DepositWalletRelayerUrl::parse("https://relayer-v2.polymarket.com").unwrap();
    assert!(DepositWalletRelayerClient::new(
        production.clone(),
        relayer_auth(),
        deposit_wallet_contract_config(137).unwrap()
    )
    .is_ok());

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

    let nonce = client
        .get_wallet_nonce(owner, wallet_nonce_read_permit_for(owner))
        .await
        .unwrap();

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
async fn relayer_client_does_not_follow_redirects_with_auth_headers() {
    let (redirect_target, mut target_handle) = spawn_redirect_target_probe().await;
    let redirect = TestResponse {
        status: "302 Found",
        headers: vec![("location".to_string(), redirect_target)],
        include_content_length: true,
        body: String::new(),
    };
    let (url, redirect_handle) = spawn_server(vec![redirect]).await;
    let client = test_client(url);

    let owner = address(WALLET_OWNER);
    let error = client
        .get_wallet_nonce(owner, wallet_nonce_read_permit_for(owner))
        .await
        .unwrap_err();

    assert!(matches!(error, RelayerError::Api { status: 302, .. }));
    let redirect_requests = redirect_handle.await.unwrap();
    assert_eq!(redirect_requests.len(), 1);
    assert_eq!(redirect_requests[0].header("RELAYER_API_KEY"), Some(API_KEY));
    assert_eq!(
        redirect_requests[0].header("RELAYER_API_KEY_ADDRESS"),
        Some(to_checksum(&address(API_KEY_ADDRESS), None).as_str())
    );
    match tokio::time::timeout(REDIRECT_TARGET_NEGATIVE_TIMEOUT, &mut target_handle).await {
        Ok(Ok(request)) => panic!(
            "redirect target unexpectedly accepted request: {} {}",
            request.method, request.path
        ),
        Ok(Err(error)) => panic!("redirect target task failed: {error}"),
        Err(_) => target_handle.abort(),
    }
}

#[tokio::test]
async fn get_wallet_nonce_rejects_production_before_http() {
    let url = DepositWalletRelayerUrl::parse("https://relayer-v2.polymarket.com").unwrap();
    let client = test_client(url);

    let error = client
        .get_wallet_nonce(address(WALLET_OWNER), DepositWalletMutationGate::Deny)
        .await
        .unwrap_err();

    assert!(error.is_deposit_wallet_mutation_blocked());
}

#[tokio::test]
async fn public_mutation_apis_default_deny_before_http() {
    let owner = signed_wallet_batch_fixture().owner();

    let (url, handle) = spawn_server(Vec::new()).await;
    let client = test_client(url);
    let error = client
        .get_wallet_nonce_with_lease(owner, DepositWalletMutationGate::Deny)
        .await
        .unwrap_err();
    assert!(error.is_deposit_wallet_mutation_blocked());
    assert!(
        error
            .to_string()
            .contains("explicit deposit-wallet mutation permit required")
    );
    assert!(handle.await.unwrap().is_empty());

    let (url, handle) = spawn_server(Vec::new()).await;
    let client = test_client(url);
    let error = client
        .submit_wallet_create(owner, DepositWalletMutationGate::Deny)
        .await
        .unwrap_err();
    assert!(error.is_deposit_wallet_mutation_blocked());
    assert!(
        error
            .to_string()
            .contains("explicit deposit-wallet mutation permit required")
    );
    assert!(handle.await.unwrap().is_empty());

    let (url, handle) = spawn_server(vec![TestResponse::json(
        "200 OK",
        json!({"nonce": "32"}).to_string(),
    )])
    .await;
    let client = test_client(url);
    let lease = client
        .get_wallet_nonce_with_lease(owner, wallet_nonce_read_permit_for(owner))
        .await
        .unwrap();
    let error = client
        .submit_signed_wallet_batch_with_nonce_lease(
            signed_wallet_batch_fixture(),
            DepositWalletMutationGate::Deny,
            lease,
        )
        .await
        .unwrap_err();
    assert!(error.is_deposit_wallet_mutation_blocked());
    assert!(
        error
            .to_string()
            .contains("explicit deposit-wallet mutation permit required")
    );
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "GET");
}

#[test]
fn production_mutation_permits_are_not_publicly_constructible() {
    let owner = address(WALLET_OWNER);
    let production = DepositWalletRelayerUrl::parse("https://relayer-v2.polymarket.com").unwrap();
    let client = DepositWalletRelayerClient::new(
        production,
        relayer_auth(),
        deposit_wallet_contract_config(137).unwrap(),
    )
    .unwrap();
    for action in [
        DepositWalletMutationAction::WalletNonceRead,
        DepositWalletMutationAction::WalletCreate,
        DepositWalletMutationAction::WalletBatch,
        DepositWalletMutationAction::OwnerRecoveryPoll,
        DepositWalletMutationAction::ManualReconciliation,
    ] {
        let evidence = DepositWalletOwnerSerializationEvidence::new(
            owner,
            client.mutation_scope(action).unwrap(),
            "unit-test owner serialization guard",
            format!("production-owner-lease-{action:?}"),
            1_699_999_900,
            1_700_000_200,
        )
        .unwrap();

        let error = DepositWalletMutationPermit::from_owner_serialization_evidence(
            "production submit",
            evidence,
        )
        .unwrap_err();

        assert!(error.is_deposit_wallet_mutation_blocked(), "{action:?}");
    }
}

#[test]
fn reconciliation_payload_hash_rejects_non_canonical_inputs() {
    let owner = address(WALLET_OWNER);
    let observation = || {
        DepositWalletSubmitReconciliationObservation::new(
            "tx-reconcile",
            RelayerTransactionState::Failed,
            None::<&str>,
            "unit-test manual submit reconciliation",
            1_700_000_001,
        )
        .unwrap()
    };
    let bad_hashes = [
        "sha256:0x0000000000000000000000000000000000000000000000000000000000000000",
        "0X0000000000000000000000000000000000000000000000000000000000000000",
        "signed-digest:0X0000000000000000000000000000000000000000000000000000000000000000",
        "0x000000000000000000000000000000000000000000000000000000000000000",
        "0x00000000000000000000000000000000000000000000000000000000000000000",
        "0x000000000000000000000000000000000000000000000000000000000000000g",
    ];

    for payload_hash in bad_hashes {
        let error = DepositWalletSubmitReconciliationEvidence::new(
            owner,
            mutation_scope(DepositWalletMutationAction::ManualReconciliation),
            "unit-test owner serialization guard",
            payload_hash,
            observation(),
        )
        .unwrap_err();
        assert!(
            error.is_deposit_wallet_mutation_blocked(),
            "{payload_hash}: {error}"
        );
    }
}

#[tokio::test]
async fn submit_wallet_create_sends_fixture_body_and_records_owner_state() {
    let expected = fixture_value("wallet_create_http_submit_request.json");
    let owner = address(expected["body"]["from"].as_str().unwrap());
    let (url, handle) = spawn_server(vec![TestResponse::json(
        "200 OK",
        json!({"transactionID": "tx-create", "state": "STATE_NEW"}).to_string(),
    )])
    .await;
    let client = test_client(url);

    let receipt = client
        .submit_wallet_create(owner, wallet_create_permit_for(owner))
        .await
        .unwrap();

    assert_eq!(receipt.transaction_id, "tx-create");
    assert_eq!(receipt.owner, Some(owner));
    let blocked = client
        .submit_wallet_create(owner, wallet_create_permit_for(owner))
        .await
        .unwrap_err();
    assert!(blocked.is_deposit_wallet_reconciliation_required());
    let requests = handle.await.unwrap();
    assert_eq!(requests[0].method, expected["method"].as_str().unwrap());
    assert_eq!(requests[0].path, expected["path"].as_str().unwrap());
    assert_eq!(
        serde_json::from_str::<Value>(&requests[0].body).unwrap(),
        expected["body"]
    );
}

#[tokio::test]
async fn owner_scoped_submit_state_does_not_block_other_owner() {
    let owner = address(WALLET_OWNER);
    let other_owner = address(OTHER_OWNER);
    let (url, handle) = spawn_server(vec![
        TestResponse::json(
            "200 OK",
            json!({"transactionID": "tx-owner", "state": "STATE_NEW"}).to_string(),
        ),
        TestResponse::json(
            "200 OK",
            json!({"transactionID": "tx-other-owner", "state": "STATE_NEW"}).to_string(),
        ),
    ])
    .await;
    let client = test_client(url);

    let first = client
        .submit_wallet_create(owner, wallet_create_permit_for(owner))
        .await
        .unwrap();
    assert_eq!(first.transaction_id, "tx-owner");
    let same_owner_blocked = client
        .submit_wallet_create(owner, wallet_create_permit_for(owner))
        .await
        .unwrap_err();
    assert!(same_owner_blocked.is_deposit_wallet_reconciliation_required());

    let other = client
        .submit_wallet_create(other_owner, wallet_create_permit_for(other_owner))
        .await
        .unwrap();
    assert_eq!(other.transaction_id, "tx-other-owner");
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 2);
}

#[tokio::test]
async fn inflight_submit_without_response_blocks_same_owner_before_http() {
    let owner = address(WALLET_OWNER);
    let (url, handle, accepted_rx, release_tx) = spawn_delayed_response_server(
        TestResponse::json(
            "200 OK",
            json!({"transactionID": "tx-delayed", "state": "STATE_NEW"}).to_string(),
        ),
    )
    .await;
    let client = test_client(url);
    let first_client = client.clone();
    let first_submit = tokio::spawn(async move {
        first_client
            .submit_wallet_create(owner, wallet_create_permit_for(owner))
            .await
    });

    accepted_rx.await.expect("first request should reach server");
    let second_submit = client
        .submit_wallet_create(owner, wallet_create_permit_for(owner))
        .await
        .unwrap_err();
    assert!(second_submit.is_deposit_wallet_reconciliation_required());
    assert!(second_submit.to_string().contains("wait for the submit response"));
    let second_nonce = client
        .get_wallet_nonce_with_lease(owner, wallet_nonce_read_permit_for(owner))
        .await
        .unwrap_err();
    assert!(second_nonce.is_deposit_wallet_reconciliation_required());
    assert!(second_nonce.to_string().contains("wait for the submit response"));

    release_tx.send(()).expect("delayed response should release");
    let first_receipt = first_submit.await.unwrap().unwrap();
    assert_eq!(first_receipt.transaction_id, "tx-delayed");
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "POST");
}

#[tokio::test]
async fn aborted_submit_after_post_boundary_leaves_ambiguous_owner_block() {
    let owner = address(WALLET_OWNER);
    let (url, handle, accepted_rx, _release_tx) = spawn_delayed_response_server(
        TestResponse::json(
            "200 OK",
            json!({"transactionID": "tx-aborted", "state": "STATE_NEW"}).to_string(),
        ),
    )
    .await;
    let client = test_client(url);
    let submit_client = client.clone();
    let submit = tokio::spawn(async move {
        submit_client
            .submit_wallet_create(owner, wallet_create_permit_for(owner))
            .await
    });

    accepted_rx.await.expect("submit should reach POST boundary");
    submit.abort();
    assert!(submit.await.unwrap_err().is_cancelled());

    assert!(client.ambiguous_submit_block(owner).is_some());
    let blocked_submit = client
        .submit_wallet_create(owner, wallet_create_permit_for(owner))
        .await
        .unwrap_err();
    assert!(blocked_submit.is_deposit_wallet_reconciliation_required());
    let blocked_nonce = client
        .get_wallet_nonce_with_lease(owner, wallet_nonce_read_permit_for(owner))
        .await
        .unwrap_err();
    assert!(blocked_nonce.is_deposit_wallet_reconciliation_required());
    handle.abort();
}

#[tokio::test]
async fn submit_signed_wallet_batch_with_nonce_lease_sends_fixture_body() {
    let expected = fixture_value("wallet_signed_http_submit_request.json");
    let signed = signed_wallet_batch_fixture();
    let owner = signed.owner();
    let (url, handle) = spawn_server(vec![
        TestResponse::json("200 OK", json!({"nonce": "31"}).to_string()),
        TestResponse::json(
            "200 OK",
            json!({"transactionID": "tx-wallet", "state": "STATE_NEW"}).to_string(),
        ),
    ])
    .await;
    let client = test_client(url);

    let lease = client
        .get_wallet_nonce_with_lease(owner, wallet_nonce_read_permit_for(owner))
        .await
        .unwrap();
    let receipt = client
        .submit_signed_wallet_batch_with_nonce_lease(
            signed,
            wallet_batch_permit_for(owner),
            lease,
        )
        .await
        .unwrap();

    assert_eq!(receipt.transaction_id, "tx-wallet");
    assert_eq!(receipt.owner, Some(owner));
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].method, "GET");
    assert_eq!(
        requests[0].path,
        format!("/nonce?address={}&type=WALLET", to_checksum(&owner, None))
    );
    assert_eq!(requests[1].method, expected["method"].as_str().unwrap());
    assert_eq!(requests[1].path, expected["path"].as_str().unwrap());
    assert_eq!(
        serde_json::from_str::<Value>(&requests[1].body).unwrap(),
        expected["body"]
    );
}

#[tokio::test]
async fn nonce_lease_blocks_same_owner_work_and_releases_after_fetch_failure() {
    let owner = signed_wallet_batch_fixture().owner();
    let (url, handle) = spawn_server(vec![TestResponse::json(
        "200 OK",
        json!({"nonce": "31"}).to_string(),
    )])
    .await;
    let client = test_client(url);

    let lease = client
        .get_wallet_nonce_with_lease(owner, wallet_nonce_read_permit_for(owner))
        .await
        .unwrap();
    let second_read = client
        .get_wallet_nonce_with_lease(owner, wallet_nonce_read_permit_for(owner))
        .await
        .unwrap_err();
    assert!(second_read.is_deposit_wallet_mutation_blocked());
    let submit = client
        .submit_wallet_create(owner, wallet_create_permit_for(owner))
        .await
        .unwrap_err();
    assert!(submit.is_deposit_wallet_mutation_blocked());
    drop(lease);
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 1);

    let (url, handle) = spawn_server(vec![
        TestResponse::json("500 Internal Server Error", "{}"),
        TestResponse::json("200 OK", json!({"nonce": "31"}).to_string()),
    ])
    .await;
    let client = test_client(url);
    let first = client
        .get_wallet_nonce_with_lease(owner, wallet_nonce_read_permit_for(owner))
        .await
        .unwrap_err();
    assert!(matches!(first, RelayerError::Api { status: 500, .. }));

    let lease = client
        .get_wallet_nonce_with_lease(owner, wallet_nonce_read_permit_for(owner))
        .await
        .unwrap();

    assert_eq!(lease.owner(), owner);
    assert_eq!(lease.nonce(), U256::from(31u64));
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 2);
}

#[tokio::test]
async fn nonce_lease_rejects_expired_owner_and_nonce_mismatch() {
    let signed = signed_wallet_batch_fixture();
    let owner = signed.owner();

    let (url, handle) = spawn_server(vec![TestResponse::json(
        "200 OK",
        json!({"nonce": "31"}).to_string(),
    )])
    .await;
    let (client, clock) = test_client_with_clock(url, 1_700_000_000);
    let lease = client
        .get_wallet_nonce_with_lease(owner, wallet_nonce_read_permit_for(owner))
        .await
        .unwrap();
    clock.store(1_700_000_201, Ordering::SeqCst);
    let error = client
        .submit_signed_wallet_batch_with_nonce_lease(
            signed.clone(),
            wallet_batch_permit_for(owner),
            lease,
        )
        .await
        .unwrap_err();
    assert!(error.is_deposit_wallet_mutation_blocked());
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 1);

    let (url, handle) = spawn_server(vec![TestResponse::json(
        "200 OK",
        json!({"nonce": "31"}).to_string(),
    )])
    .await;
    let client = test_client(url);
    let other_owner = address(OTHER_OWNER);
    let other_owner_lease = client
        .get_wallet_nonce_with_lease(other_owner, wallet_nonce_read_permit_for(other_owner))
        .await
        .unwrap();
    let error = client
        .submit_signed_wallet_batch_with_nonce_lease(
            signed.clone(),
            wallet_batch_permit_for(owner),
            other_owner_lease,
        )
        .await
        .unwrap_err();
    assert!(error.is_deposit_wallet_mutation_blocked());
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 1);

    let (url, handle) = spawn_server(vec![TestResponse::json(
        "200 OK",
        json!({"nonce": "32"}).to_string(),
    )])
    .await;
    let client = test_client(url);
    let wrong_nonce_lease = client
        .get_wallet_nonce_with_lease(owner, wallet_nonce_read_permit_for(owner))
        .await
        .unwrap();
    let error = client
        .submit_signed_wallet_batch_with_nonce_lease(
            signed,
            wallet_batch_permit_for(owner),
            wrong_nonce_lease,
        )
        .await
        .unwrap_err();
    assert!(matches!(error, RelayerError::Signing(_)));
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 1);
}

#[tokio::test]
async fn nonce_lease_rejects_cross_client_promotion() {
    let signed = signed_wallet_batch_fixture();
    let owner = signed.owner();
    let (url, handle) = spawn_server(vec![TestResponse::json(
        "200 OK",
        json!({"nonce": "31"}).to_string(),
    )])
    .await;
    let lease_client = test_client(url.clone());
    let submit_client = test_client(url);

    let lease = lease_client
        .get_wallet_nonce_with_lease(owner, wallet_nonce_read_permit_for(owner))
        .await
        .unwrap();
    let error = submit_client
        .submit_signed_wallet_batch_with_nonce_lease(signed, wallet_batch_permit_for(owner), lease)
        .await
        .unwrap_err();

    assert!(error.is_deposit_wallet_mutation_blocked());
    assert!(error.to_string().contains("different deposit-wallet client"));
    lease_client.ensure_owner_unblocked(owner).unwrap();
    submit_client.ensure_owner_unblocked(owner).unwrap();
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "GET");
}

#[tokio::test]
async fn signed_wallet_batch_deadline_expiry_blocks_submit_before_post() {
    let signed = signed_wallet_batch_fixture();
    let owner = signed.owner();
    let signed_nonce = signed.nonce();
    let (url, handle) = spawn_server(vec![
        TestResponse::json("200 OK", json!({"nonce": "31"}).to_string()),
        TestResponse::json("200 OK", json!({"nonce": "31"}).to_string()),
    ])
    .await;
    let (client, _clock) = test_client_with_clock(url, 1_760_000_001);
    let nonce_permit = mutation_permit_for_action_at(
        owner,
        DepositWalletMutationAction::WalletNonceRead,
        1_760_000_000,
        1_760_000_200,
    );
    let batch_permit = mutation_permit_for_action_at(
        owner,
        DepositWalletMutationAction::WalletBatch,
        1_760_000_000,
        1_760_000_200,
    );

    let lease = client
        .get_wallet_nonce_with_lease(owner, nonce_permit)
        .await
        .unwrap();
    let error = client
        .submit_signed_wallet_batch_with_nonce_lease(signed, batch_permit, lease)
        .await
        .unwrap_err();

    assert!(matches!(error, RelayerError::Signing(_)));
    assert!(error.to_string().contains("deadline is expired"));
    assert!(client.ambiguous_submit_block(owner).is_none());
    client.ensure_owner_unblocked(owner).unwrap();
    let next_lease = client
        .get_wallet_nonce_with_lease(
            owner,
            mutation_permit_for_action_at(
                owner,
                DepositWalletMutationAction::WalletNonceRead,
                1_760_000_000,
                1_760_000_200,
            ),
        )
        .await
        .unwrap();
    assert_eq!(next_lease.nonce(), signed_nonce);
    drop(next_lease);
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests.iter().all(|request| request.method == "GET"));
}

#[test]
fn manual_reconciliation_observation_normalizes_non_confirmed_hashes() {
    let owner = address(WALLET_OWNER);
    let payload_hash =
        "0x5123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let transaction_hash =
        "0x38CBFBEAE8FFFA4E2B187EE5978D3EE9CAFC53AF0363ED90A35B7EA9016535D8";

    for state in [
        RelayerTransactionState::Failed,
        RelayerTransactionState::Invalid,
    ] {
        let observation = DepositWalletSubmitReconciliationObservation::new(
            format!("tx-{state:?}"),
            state,
            Some(transaction_hash),
            "unit-test manual submit reconciliation",
            1_700_000_001,
        )
        .unwrap();
        let evidence = DepositWalletSubmitReconciliationEvidence::new(
            owner,
            mutation_scope(DepositWalletMutationAction::ManualReconciliation),
            "unit-test owner serialization guard",
            payload_hash,
            observation,
        )
        .unwrap();
        assert_eq!(evidence.transaction_hash(), None);
    }
}

#[tokio::test]
async fn manual_reconciliation_records_terminal_poll_and_clears_owner_block() {
    let owner = address(WALLET_OWNER);
    let (url, handle) = spawn_server(vec![
        TestResponse::json(
            "200 OK",
            json!({"transactionID": "tx-ambiguous"}).to_string(),
        ),
        TestResponse::json(
            "200 OK",
            wallet_create_transaction_response_value("tx-ambiguous", "STATE_FAILED").to_string(),
        ),
    ])
    .await;
    let client = test_client(url);

    let submit = client
        .submit_wallet_create(owner, wallet_create_permit_for(owner))
        .await
        .unwrap_err();
    assert!(submit.is_deposit_wallet_ambiguous_submit());
    assert_eq!(
        client.ambiguous_submit_transaction_ids(owner),
        vec!["tx-ambiguous".to_string()]
    );
    let poll = client
        .get_transaction_for_owner(owner, "tx-ambiguous")
        .await
        .unwrap_err();
    assert!(matches!(poll, RelayerError::TransactionFailed(_)));
    let payload_hash = client
        .ambiguous_submit_block(owner)
        .expect("ambiguous submit should remain blocked until manual clear");
    let evidence = manual_reconciliation_evidence(
        owner,
        payload_hash,
        "tx-ambiguous",
        RelayerTransactionState::Failed,
        None,
    );

    client
        .clear_ambiguous_submit_after_manual_reconciliation(
            evidence,
            manual_reconciliation_permit_token_for(owner),
        )
        .unwrap();

    assert!(client.ambiguous_submit_block(owner).is_none());
    assert!(client.ambiguous_submit_transaction_ids(owner).is_empty());
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 2);
}

#[tokio::test]
async fn manual_reconciliation_rejects_mismatched_terminal_observation_without_partial_delete() {
    let owner = address(WALLET_OWNER);
    let (url, handle) = spawn_server(vec![
        TestResponse::json(
            "200 OK",
            json!({"transactionID": "tx-ambiguous"}).to_string(),
        ),
        TestResponse::json(
            "200 OK",
            wallet_create_transaction_response_value("tx-ambiguous", "STATE_FAILED").to_string(),
        ),
    ])
    .await;
    let client = test_client(url);

    let _ = client
        .submit_wallet_create(owner, wallet_create_permit_for(owner))
        .await
        .unwrap_err();
    let payload_hash = client.ambiguous_submit_block(owner).unwrap();
    let _ = client
        .get_transaction_for_owner(owner, "tx-ambiguous")
        .await
        .unwrap_err();
    let evidence = manual_reconciliation_evidence(
        owner,
        payload_hash,
        "tx-ambiguous",
        RelayerTransactionState::Invalid,
        None,
    );

    let error = client
        .clear_ambiguous_submit_after_manual_reconciliation(
            evidence,
            manual_reconciliation_permit_token_for(owner),
        )
        .unwrap_err();

    assert!(error.is_deposit_wallet_reconciliation_required());
    assert!(client.ambiguous_submit_block(owner).is_some());
    assert_eq!(
        client.ambiguous_submit_transaction_ids(owner),
        vec!["tx-ambiguous".to_string()]
    );
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 2);
}

#[test]
fn manual_reconciliation_rejects_untrusted_evidence_without_clearing() {
    let owner = address(WALLET_OWNER);
    let payload_hash =
        "0x9123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string();
    let client = test_client(DepositWalletRelayerUrl::loopback("http://127.0.0.1/").unwrap());
    client.record_ambiguous(owner, payload_hash.clone()).unwrap();

    let observation = DepositWalletSubmitReconciliationObservation::new(
        "tx-issuer-mismatch",
        RelayerTransactionState::Failed,
        None::<&str>,
        "unit-test manual submit reconciliation",
        1_700_000_001,
    )
    .unwrap();
    let issuer_mismatch = DepositWalletSubmitReconciliationEvidence::new(
        owner,
        mutation_scope(DepositWalletMutationAction::ManualReconciliation),
        "different unit-test issuer",
        payload_hash.clone(),
        observation,
    )
    .unwrap();
    let error = client
        .clear_ambiguous_submit_after_manual_reconciliation(
            issuer_mismatch,
            manual_reconciliation_permit_token_for(owner),
        )
        .unwrap_err();
    assert!(error.is_deposit_wallet_mutation_blocked());
    assert!(error.to_string().contains("issuer"));
    assert!(client.ambiguous_submit_block(owner).is_some());

    let missing_record = manual_reconciliation_evidence(
        owner,
        payload_hash,
        "tx-missing-record",
        RelayerTransactionState::Failed,
        None,
    );
    let error = client
        .clear_ambiguous_submit_after_manual_reconciliation(
            missing_record,
            manual_reconciliation_permit_token_for(owner),
        )
        .unwrap_err();
    assert!(error.is_deposit_wallet_reconciliation_required());
    assert!(error.to_string().contains("no local owner payload record"));
    assert!(client.ambiguous_submit_block(owner).is_some());
}

#[tokio::test]
async fn terminal_owner_poll_requires_manual_reconciliation_before_resubmit() {
    let owner = address(WALLET_OWNER);
    let wallet_create_confirmed =
        wallet_create_transaction_response_value("tx-inflight", "STATE_CONFIRMED");
    let (url, handle) = spawn_server(vec![
        TestResponse::json(
            "200 OK",
            json!({"transactionID": "tx-inflight", "state": "STATE_NEW"}).to_string(),
        ),
        TestResponse::json("200 OK", wallet_create_confirmed.to_string()),
        TestResponse::json(
            "200 OK",
            json!({"transactionID": "tx-next", "state": "STATE_NEW"}).to_string(),
        ),
    ])
    .await;
    let client = test_client(url);

    let first = client
        .submit_wallet_create(owner, wallet_create_permit_for(owner))
        .await
        .unwrap();
    assert_eq!(first.transaction_id, "tx-inflight");
    let blocked = client
        .submit_wallet_create(owner, wallet_create_permit_for(owner))
        .await
        .unwrap_err();
    assert!(blocked.is_deposit_wallet_reconciliation_required());

    let terminal = client
        .get_transaction_for_owner(owner, "tx-inflight")
        .await
        .unwrap();
    assert_eq!(terminal.state, RelayerTransactionState::Confirmed);
    let confirmed_hash = terminal.transaction_hash.clone();

    let blocked = client
        .submit_wallet_create(owner, wallet_create_permit_for(owner))
        .await
        .unwrap_err();
    assert!(blocked.is_deposit_wallet_reconciliation_required());
    let payload_hash = client
        .ambiguous_submit_block(owner)
        .expect("confirmed terminal poll should require manual owner reconciliation");
    let evidence = manual_reconciliation_evidence(
        owner,
        payload_hash,
        "tx-inflight",
        RelayerTransactionState::Confirmed,
        confirmed_hash.as_deref(),
    );
    client
        .clear_ambiguous_submit_after_manual_reconciliation(
            evidence,
            manual_reconciliation_permit_token_for(owner),
        )
        .unwrap();

    let next = client
        .submit_wallet_create(owner, wallet_create_permit_for(owner))
        .await
        .unwrap();
    assert_eq!(next.transaction_id, "tx-next");
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 3);
}

#[tokio::test]
async fn terminal_owner_poll_requires_matching_transaction_type() {
    let owner = address(WALLET_OWNER);
    let (url, handle) = spawn_server(vec![
        TestResponse::json(
            "200 OK",
            json!({"transactionID": "tx-type-mismatch", "state": "STATE_NEW"}).to_string(),
        ),
        TestResponse::json(
            "200 OK",
            transaction_response_value("tx-type-mismatch", "STATE_CONFIRMED").to_string(),
        ),
    ])
    .await;
    let client = test_client(url);

    let first = client
        .submit_wallet_create(owner, wallet_create_permit_for(owner))
        .await
        .unwrap();
    assert_eq!(first.transaction_id, "tx-type-mismatch");
    let error = client
        .get_transaction_for_owner(owner, "tx-type-mismatch")
        .await
        .unwrap_err();

    assert!(error.is_deposit_wallet_reconciliation_required());
    assert!(error.to_string().contains("did not match submitted type"));
    assert!(client.ambiguous_submit_block(owner).is_some());
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 2);
}

#[tokio::test]
async fn terminal_failure_poll_requires_manual_reconciliation_before_resubmit() {
    let owner = address(WALLET_OWNER);
    let (url, handle) = spawn_server(vec![
        TestResponse::json(
            "200 OK",
            json!({"transactionID": "tx-failed-inflight", "state": "STATE_NEW"}).to_string(),
        ),
        TestResponse::json(
            "200 OK",
            wallet_create_transaction_response_value("tx-failed-inflight", "STATE_FAILED")
                .to_string(),
        ),
        TestResponse::json(
            "200 OK",
            json!({"transactionID": "tx-after-failed-clear", "state": "STATE_NEW"}).to_string(),
        ),
    ])
    .await;
    let client = test_client(url);

    let first = client
        .submit_wallet_create(owner, wallet_create_permit_for(owner))
        .await
        .unwrap();
    assert_eq!(first.transaction_id, "tx-failed-inflight");
    let failed = client
        .get_transaction_for_owner(owner, "tx-failed-inflight")
        .await
        .unwrap_err();
    assert!(matches!(failed, RelayerError::TransactionFailed(_)));
    let blocked = client
        .submit_wallet_create(owner, wallet_create_permit_for(owner))
        .await
        .unwrap_err();
    assert!(blocked.is_deposit_wallet_reconciliation_required());
    let payload_hash = client
        .ambiguous_submit_block(owner)
        .expect("failed terminal poll should require manual owner reconciliation");
    let evidence = manual_reconciliation_evidence(
        owner,
        payload_hash,
        "tx-failed-inflight",
        RelayerTransactionState::Failed,
        None,
    );
    client
        .clear_ambiguous_submit_after_manual_reconciliation(
            evidence,
            manual_reconciliation_permit_token_for(owner),
        )
        .unwrap();

    let next = client
        .submit_wallet_create(owner, wallet_create_permit_for(owner))
        .await
        .unwrap();
    assert_eq!(next.transaction_id, "tx-after-failed-clear");
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 3);
}

#[tokio::test]
async fn reconciliation_required_poll_transitions_inflight_to_ambiguous_block() {
    let owner = address(WALLET_OWNER);
    let mut confirmed_without_hash =
        wallet_create_transaction_response_value("tx-reconcile-poll-confirmed-no-hash", "STATE_CONFIRMED");
    confirmed_without_hash
        .as_object_mut()
        .unwrap()
        .remove("transactionHash");
    let mut missing_owner_evidence =
        wallet_create_transaction_response_value("tx-reconcile-poll-missing-owner", "STATE_CONFIRMED");
    missing_owner_evidence
        .as_object_mut()
        .unwrap()
        .remove("owner");
    for (transaction_id, transaction_response) in [
        (
            "tx-reconcile-poll-unknown",
            wallet_create_transaction_response_value("tx-reconcile-poll-unknown", "STATE_FUTURE"),
        ),
        (
            "tx-reconcile-poll-confirmed-no-hash",
            confirmed_without_hash,
        ),
        ("tx-reconcile-poll-missing-owner", missing_owner_evidence),
    ] {
        let (url, handle) = spawn_server(vec![
            TestResponse::json(
                "200 OK",
                json!({"transactionID": transaction_id, "state": "STATE_NEW"}).to_string(),
            ),
            TestResponse::json("200 OK", transaction_response.to_string()),
        ])
        .await;
        let client = test_client(url);

        let first = client
            .submit_wallet_create(owner, wallet_create_permit_for(owner))
            .await
            .unwrap();
        assert_eq!(first.transaction_id, transaction_id);
        let poll = client
            .get_transaction_for_owner(owner, transaction_id)
            .await
            .unwrap_err();
        assert!(poll.is_deposit_wallet_reconciliation_required());
        assert!(client.ambiguous_submit_block(owner).is_some());
        let blocked = client
            .submit_wallet_create(owner, wallet_create_permit_for(owner))
            .await
            .unwrap_err();
        assert!(blocked.is_deposit_wallet_reconciliation_required());
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 2);
    }
}

#[test]
fn manual_reconciliation_rejects_additional_records_without_partial_delete() {
    let owner = address(WALLET_OWNER);
    let payload_hash =
        "0x0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string();
    let client = test_client(DepositWalletRelayerUrl::loopback("http://127.0.0.1/").unwrap());

    client.record_ambiguous(owner, payload_hash.clone()).unwrap();
    client
        .record_transaction_owner(
            "tx-a",
            owner,
            payload_hash.clone(),
            WALLET_CREATE_TRANSACTION_TYPE,
        )
        .unwrap();
    client
        .record_transaction_owner(
            "tx-b",
            owner,
            payload_hash.clone(),
            WALLET_CREATE_TRANSACTION_TYPE,
        )
        .unwrap();
    let receipt = DepositWalletTransactionReceipt {
        transaction_id: "tx-a".to_string(),
        state: RelayerTransactionState::Failed,
        transaction_hash: None,
        owner: Some(owner),
        deposit_wallet: None,
    };
    client
        .record_terminal_observation_from_receipt(owner, &receipt, WALLET_CREATE_TRANSACTION_TYPE)
        .unwrap();
    let evidence = manual_reconciliation_evidence(
        owner,
        payload_hash,
        "tx-a",
        RelayerTransactionState::Failed,
        None,
    );

    let error = client
        .clear_ambiguous_submit_after_manual_reconciliation(
            evidence,
            manual_reconciliation_permit_token_for(owner),
        )
        .unwrap_err();

    assert!(error.is_deposit_wallet_reconciliation_required());
    assert_eq!(
        client.ambiguous_submit_transaction_ids(owner),
        vec!["tx-a".to_string(), "tx-b".to_string()]
    );
}

#[test]
fn transaction_owner_mapping_rejects_payload_conflicts() {
    let owner = address(WALLET_OWNER);
    let payload_hash =
        "0x5123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string();
    let other_payload_hash =
        "0x6123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string();
    let client = test_client(DepositWalletRelayerUrl::loopback("http://127.0.0.1/").unwrap());

    client
        .record_transaction_owner(
            "tx-conflict",
            owner,
            payload_hash,
            WALLET_CREATE_TRANSACTION_TYPE,
        )
        .unwrap();
    let error = client
        .record_transaction_owner(
            "tx-conflict",
            owner,
            other_payload_hash,
            WALLET_CREATE_TRANSACTION_TYPE,
        )
        .unwrap_err();

    assert!(error.is_deposit_wallet_reconciliation_required());
    assert!(error.to_string().contains("already associated"));
}

#[test]
fn owner_mutation_capacity_rejects_new_records_but_allows_existing_records() {
    let owner = address(WALLET_OWNER);
    let payload_hash =
        "0x8123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string();
    let mut state = OwnerMutationState::default();

    for index in 0..MAX_OWNER_MUTATION_RECORDS {
        state.owner_blocks.insert(
            Address::from_low_u64_be(index as u64),
            OwnerMutationBlock::Ambiguous {
                payload_hash: payload_hash.clone(),
                created_at_unix_seconds: 1_700_000_000,
                unrecorded_transaction_id_observed: false,
            },
        );
    }
    let new_owner = Address::from_low_u64_be(MAX_OWNER_MUTATION_RECORDS as u64);
    let owner_error = ensure_owner_mutation_capacity(&state, new_owner, None).unwrap_err();
    assert!(owner_error.is_deposit_wallet_mutation_blocked());
    assert!(owner_error.to_string().contains("tracks 1024 owners"));
    ensure_owner_mutation_capacity(&state, Address::from_low_u64_be(0), None).unwrap();

    state.owner_blocks.clear();
    for index in 0..MAX_OWNER_MUTATION_RECORDS {
        state.transaction_owners.insert(
            format!("tx-{index}"),
            OwnerTransactionRecord {
                owner,
                payload_hash: payload_hash.clone(),
                transaction_type: WALLET_CREATE_TRANSACTION_TYPE,
            },
        );
    }
    let transaction_error =
        ensure_owner_mutation_capacity(&state, owner, Some("tx-new")).unwrap_err();
    assert!(transaction_error.is_deposit_wallet_mutation_blocked());
    assert!(
        transaction_error
            .to_string()
            .contains("tracks 1024 transactions")
    );
    ensure_owner_mutation_capacity(&state, owner, Some("tx-0")).unwrap();
}

#[test]
fn mutation_permit_rejects_owner_scope_and_freshness_failures() {
    let owner = address(WALLET_OWNER);
    let other_owner = address(OTHER_OWNER);
    let client = test_client(DepositWalletRelayerUrl::loopback("http://127.0.0.1/").unwrap());

    let owner_mismatch = client
        .ensure_permitted_for_action(
            &wallet_create_permit_for(other_owner),
            owner,
            DepositWalletMutationAction::WalletCreate,
        )
        .unwrap_err();
    assert!(owner_mismatch.is_deposit_wallet_mutation_blocked());

    let scope_mismatch = client
        .ensure_permitted_for_action(
            &wallet_nonce_read_permit_for(owner),
            owner,
            DepositWalletMutationAction::WalletCreate,
        )
        .unwrap_err();
    assert!(scope_mismatch.is_deposit_wallet_mutation_blocked());

    let expired_evidence = DepositWalletOwnerSerializationEvidence::new(
        owner,
        mutation_scope(DepositWalletMutationAction::WalletCreate),
        "unit-test owner serialization guard",
        "expired-lease",
        1_699_999_800,
        1_699_999_900,
    )
    .unwrap();
    let expired_permit =
        DepositWalletMutationPermit::from_owner_serialization_evidence("expired", expired_evidence)
            .unwrap();
    let expired = client
        .ensure_permit_token_for_action(
            &expired_permit,
            owner,
            DepositWalletMutationAction::WalletCreate,
        )
        .unwrap_err();
    assert!(expired.is_deposit_wallet_mutation_blocked());
    assert!(expired.to_string().contains("expired"));

    let future_evidence = DepositWalletOwnerSerializationEvidence::new(
        owner,
        mutation_scope(DepositWalletMutationAction::WalletCreate),
        "unit-test owner serialization guard",
        "future-lease",
        1_700_000_031,
        1_700_000_100,
    )
    .unwrap();
    let future_permit =
        DepositWalletMutationPermit::from_owner_serialization_evidence("future", future_evidence)
            .unwrap();
    let future = client
        .ensure_permit_token_for_action(
            &future_permit,
            owner,
            DepositWalletMutationAction::WalletCreate,
        )
        .unwrap_err();
    assert!(future.is_deposit_wallet_mutation_blocked());
    assert!(future.to_string().contains("future"));

    let stale_evidence = DepositWalletOwnerSerializationEvidence::new(
        owner,
        mutation_scope(DepositWalletMutationAction::WalletCreate),
        "unit-test owner serialization guard",
        "stale-lease",
        1_699_999_699,
        1_699_999_999,
    )
    .unwrap();
    let stale_permit =
        DepositWalletMutationPermit::from_owner_serialization_evidence("stale", stale_evidence)
            .unwrap();
    let stale = client
        .ensure_permit_token_for_action(
            &stale_permit,
            owner,
            DepositWalletMutationAction::WalletCreate,
        )
        .unwrap_err();
    assert!(stale.is_deposit_wallet_mutation_blocked());
    assert!(stale.to_string().contains("stale"));

    assert!(DepositWalletOwnerSerializationEvidence::new(
        owner,
        mutation_scope(DepositWalletMutationAction::WalletCreate),
        "",
        "lease",
        1_699_999_900,
        1_700_000_200,
    )
    .is_err());
    assert!(DepositWalletOwnerSerializationEvidence::new(
        owner,
        mutation_scope(DepositWalletMutationAction::WalletCreate),
        "unit-test owner serialization guard",
        "",
        1_699_999_900,
        1_700_000_200,
    )
    .is_err());
    assert!(DepositWalletOwnerSerializationEvidence::new(
        owner,
        mutation_scope(DepositWalletMutationAction::WalletCreate),
        "unit-test owner serialization guard",
        "too-long-lease",
        1_699_999_900,
        1_700_000_201,
    )
    .is_err());
    let evidence = owner_serialization_evidence_for_action(
        owner,
        DepositWalletMutationAction::WalletCreate,
    );
    assert!(DepositWalletMutationPermit::from_owner_serialization_evidence("", evidence).is_err());
}

#[tokio::test]
async fn post_boundary_errors_leave_ambiguous_owner_block() {
    let owner = address(WALLET_OWNER);
    for (status, expected_status) in [
        ("500 Internal Server Error", 500u16),
        ("429 Too Many Requests", 429u16),
    ] {
        let (url, handle) = spawn_server(vec![TestResponse::json(status, "{}")]).await;
        let client = test_client(url);

        let error = client
            .submit_wallet_create(owner, wallet_create_permit_for(owner))
            .await
            .unwrap_err();

        assert!(error.is_deposit_wallet_ambiguous_submit());
        assert!(error.to_string().contains(&expected_status.to_string()));
        assert!(client.ambiguous_submit_block(owner).is_some());
        let blocked = client
            .submit_wallet_create(owner, wallet_create_permit_for(owner))
            .await
            .unwrap_err();
        assert!(blocked.is_deposit_wallet_reconciliation_required());
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
    }

    let (url, handle) = spawn_reset_server().await;
    let client = test_client(url);
    let error = client
        .submit_wallet_create(owner, wallet_create_permit_for(owner))
        .await
        .unwrap_err();
    assert!(error.is_deposit_wallet_ambiguous_submit());
    assert!(error.to_string().contains("transport category"));
    assert!(client.ambiguous_submit_block(owner).is_some());
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 1);

    let (url, handle) = spawn_server(vec![TestResponse::json_without_content_length(
        "200 OK",
        "x".repeat(MAX_SUCCESS_BODY_BYTES + 1),
    )])
    .await;
    let client = test_client(url);
    let error = client
        .submit_wallet_create(owner, wallet_create_permit_for(owner))
        .await
        .unwrap_err();
    assert!(error.is_deposit_wallet_ambiguous_submit());
    assert!(error.to_string().contains("exceeded maximum size"));
    assert!(client.ambiguous_submit_block(owner).is_some());
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 1);
}

#[tokio::test]
async fn connect_failure_keeps_owner_reservation_ambiguous() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("test listener should bind");
    let addr = listener.local_addr().unwrap();
    drop(listener);
    let url = DepositWalletRelayerUrl::loopback(&format!("http://{addr}")).unwrap();
    let owner = address(WALLET_OWNER);
    let client = test_client(url);

    let error = client
        .submit_wallet_create(owner, wallet_create_permit_for(owner))
        .await
        .unwrap_err();

    assert!(error.is_deposit_wallet_ambiguous_submit());
    assert!(error.to_string().contains("transport category: connect"));
    assert!(client.ambiguous_submit_block(owner).is_some());
    let blocked = client
        .submit_wallet_create(owner, wallet_create_permit_for(owner))
        .await
        .unwrap_err();
    assert!(blocked.is_deposit_wallet_reconciliation_required());
}

#[tokio::test]
async fn idless_manual_reconciliation_rejects_when_local_transaction_record_exists() {
    let owner = address(WALLET_OWNER);
    let (url, handle) = spawn_server(vec![TestResponse::json(
        "200 OK",
        json!({"transactionID": "tx-ambiguous"}).to_string(),
    )])
    .await;
    let client = test_client(url);

    let submit_error = client
        .submit_wallet_create(owner, wallet_create_permit_for(owner))
        .await
        .unwrap_err();
    assert!(submit_error.is_deposit_wallet_ambiguous_submit());
    let payload_hash = client
        .ambiguous_submit_block(owner)
        .expect("ambiguous submit should block owner");
    assert_eq!(
        client.ambiguous_submit_transaction_ids(owner),
        vec!["tx-ambiguous".to_string()]
    );
    let evidence = DepositWalletIdlessSubmitReconciliationEvidence::new(
        owner,
        mutation_scope(DepositWalletMutationAction::ManualReconciliation),
        "unit-test owner serialization guard",
        payload_hash,
        "unit-test relayer audit found no accepted transaction for the ambiguous payload",
        1_700_000_001,
    )
    .unwrap();

    let error = client
        .clear_idless_ambiguous_submit_after_manual_reconciliation(
            evidence,
            manual_reconciliation_permit_token_for(owner),
        )
        .unwrap_err();

    assert!(error.is_deposit_wallet_reconciliation_required());
    assert!(error.to_string().contains("still has local transaction records"));
    let blocked = client
        .submit_wallet_create(owner, wallet_create_permit_for(owner))
        .await
        .unwrap_err();
    assert!(blocked.is_deposit_wallet_reconciliation_required());
    let _ = handle.await.unwrap();
}

#[tokio::test]
async fn partial_submit_response_alias_records_transaction_owner() {
    let owner = address(WALLET_OWNER);
    let (url, handle) = spawn_server(vec![TestResponse::json(
        "200 OK",
        json!({"transactionId": "tx-alias-partial"}).to_string(),
    )])
    .await;
    let client = test_client(url);

    let error = client
        .submit_wallet_create(owner, wallet_create_permit_for(owner))
        .await
        .unwrap_err();

    assert!(error.is_deposit_wallet_ambiguous_submit());
    assert_eq!(
        client.ambiguous_submit_transaction_ids(owner),
        vec!["tx-alias-partial".to_string()]
    );
    let blocked = client
        .submit_wallet_create(owner, wallet_create_permit_for(owner))
        .await
        .unwrap_err();
    assert!(blocked.is_deposit_wallet_reconciliation_required());
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 1);
}

#[tokio::test]
async fn invalid_partial_submit_response_leaves_recordless_ambiguous_owner_block() {
    let owner = address(WALLET_OWNER);
    for (label, body) in [
        (
            "blank transactionID",
            json!({"transactionID": "", "state": "STATE_NEW"}).to_string(),
        ),
        (
            "blank transactionId alias",
            json!({"transactionId": "", "state": "STATE_NEW"}).to_string(),
        ),
        (
            "array response",
            json!([{"transactionID": "tx-array-partial", "state": "STATE_NEW"}]).to_string(),
        ),
    ] {
        let (url, handle) =
            spawn_server(vec![TestResponse::json("200 OK", body.clone())]).await;
        let client = test_client(url);

        let error = client
            .submit_wallet_create(owner, wallet_create_permit_for(owner))
            .await
            .unwrap_err();

        assert!(
            error.is_deposit_wallet_ambiguous_submit(),
            "{label}: {error}"
        );
        assert!(
            client.ambiguous_submit_transaction_ids(owner).is_empty(),
            "{label}: unexpected transaction ids"
        );
        assert!(
            client.ambiguous_submit_block(owner).is_some(),
            "{label}: owner should remain blocked"
        );
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
    }
}

#[tokio::test]
async fn idless_manual_reconciliation_rejects_observed_transaction_id_owner_record_conflict() {
    let owner = address(WALLET_OWNER);
    let conflicting_payload_hash =
        "0x7123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string();
    let (url, handle) = spawn_server(vec![TestResponse::json(
        "200 OK",
        json!({"transactionID": "tx-record-conflict", "state": "STATE_FAILED"}).to_string(),
    )])
    .await;
    let client = test_client(url);
    client
        .record_transaction_owner(
            "tx-record-conflict",
            owner,
            conflicting_payload_hash,
            WALLET_CREATE_TRANSACTION_TYPE,
        )
        .unwrap();

    let submit_error = client
        .submit_wallet_create(owner, wallet_create_permit_for(owner))
        .await
        .unwrap_err();
    assert!(submit_error.is_deposit_wallet_reconciliation_required());
    assert!(submit_error.to_string().contains("already associated"));
    let payload_hash = client
        .ambiguous_submit_block(owner)
        .expect("ambiguous submit should keep the observed payload blocked");
    assert!(client.ambiguous_submit_transaction_ids(owner).is_empty());
    let evidence = idless_reconciliation_evidence(owner, payload_hash, 1_700_000_001);

    let error = client
        .clear_idless_ambiguous_submit_after_manual_reconciliation(
            evidence,
            manual_reconciliation_permit_token_for(owner),
        )
        .unwrap_err();

    assert!(error.is_deposit_wallet_reconciliation_required());
    assert!(error.to_string().contains("observed a transaction id"));
    assert!(client.ambiguous_submit_block(owner).is_some());
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 1);
}

#[tokio::test]
async fn idless_manual_reconciliation_rejects_observed_new_transaction_id_record_failure() {
    let owner = address(WALLET_OWNER);
    let conflicting_payload_hash =
        "0x8123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string();
    let (url, handle) = spawn_server(vec![TestResponse::json(
        "200 OK",
        json!({"transactionID": "tx-new-record-conflict", "state": "STATE_NEW"}).to_string(),
    )])
    .await;
    let client = test_client(url);
    client
        .record_transaction_owner(
            "tx-new-record-conflict",
            owner,
            conflicting_payload_hash,
            WALLET_CREATE_TRANSACTION_TYPE,
        )
        .unwrap();

    let submit_error = client
        .submit_wallet_create(owner, wallet_create_permit_for(owner))
        .await
        .unwrap_err();
    assert!(submit_error.is_deposit_wallet_reconciliation_required());
    assert!(submit_error.to_string().contains("already associated"));
    let payload_hash = client
        .ambiguous_submit_block(owner)
        .expect("ambiguous submit should keep the observed payload blocked");
    assert!(client.ambiguous_submit_transaction_ids(owner).is_empty());
    let evidence = idless_reconciliation_evidence(owner, payload_hash, 1_700_000_001);

    let error = client
        .clear_idless_ambiguous_submit_after_manual_reconciliation(
            evidence,
            manual_reconciliation_permit_token_for(owner),
        )
        .unwrap_err();

    assert!(error.is_deposit_wallet_reconciliation_required());
    assert!(error.to_string().contains("observed a transaction id"));
    assert!(client.ambiguous_submit_block(owner).is_some());
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 1);
}

#[test]
fn idless_manual_reconciliation_success_clears_recordless_ambiguous_block() {
    let owner = address(WALLET_OWNER);
    let payload_hash =
        "0x1123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string();
    let client = test_client(DepositWalletRelayerUrl::loopback("http://127.0.0.1/").unwrap());

    client.record_ambiguous(owner, payload_hash.clone()).unwrap();
    let evidence = idless_reconciliation_evidence(owner, payload_hash, 1_700_000_001);

    client
        .clear_idless_ambiguous_submit_after_manual_reconciliation(
            evidence,
            manual_reconciliation_permit_token_for(owner),
        )
        .unwrap();

    assert!(client.ambiguous_submit_block(owner).is_none());
    assert!(client.ambiguous_submit_transaction_ids(owner).is_empty());
}

#[test]
fn idless_manual_reconciliation_rejects_unrecorded_transaction_id_observation() {
    let owner = address(WALLET_OWNER);
    let payload_hash =
        "0x4123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string();
    let client = test_client(DepositWalletRelayerUrl::loopback("http://127.0.0.1/").unwrap());

    client.record_ambiguous(owner, payload_hash.clone()).unwrap();
    client
        .record_unrecorded_transaction_id_observation(owner, &payload_hash)
        .unwrap();
    let evidence = idless_reconciliation_evidence(owner, payload_hash, 1_700_000_001);

    let error = client
        .clear_idless_ambiguous_submit_after_manual_reconciliation(
            evidence,
            manual_reconciliation_permit_token_for(owner),
        )
        .unwrap_err();

    assert!(error.is_deposit_wallet_reconciliation_required());
    assert!(error.to_string().contains("observed a transaction id"));
    assert!(client.ambiguous_submit_block(owner).is_some());
}

#[test]
fn transaction_id_manual_reconciliation_rejects_unrecorded_transaction_id_observation() {
    let owner = address(WALLET_OWNER);
    let payload_hash =
        "0x5123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string();
    let client = test_client(DepositWalletRelayerUrl::loopback("http://127.0.0.1/").unwrap());

    client.record_ambiguous(owner, payload_hash.clone()).unwrap();
    client
        .record_transaction_owner(
            "tx-recorded",
            owner,
            payload_hash.clone(),
            WALLET_CREATE_TRANSACTION_TYPE,
        )
        .unwrap();
    let receipt = DepositWalletTransactionReceipt {
        transaction_id: "tx-recorded".to_string(),
        state: RelayerTransactionState::Failed,
        transaction_hash: None,
        owner: Some(owner),
        deposit_wallet: None,
    };
    client
        .record_terminal_observation_from_receipt(owner, &receipt, WALLET_CREATE_TRANSACTION_TYPE)
        .unwrap();
    client
        .record_unrecorded_transaction_id_observation(owner, &payload_hash)
        .unwrap();
    let evidence = manual_reconciliation_evidence(
        owner,
        payload_hash,
        "tx-recorded",
        RelayerTransactionState::Failed,
        None,
    );

    let error = client
        .clear_ambiguous_submit_after_manual_reconciliation(
            evidence,
            manual_reconciliation_permit_token_for(owner),
        )
        .unwrap_err();

    assert!(error.is_deposit_wallet_reconciliation_required());
    assert!(error.to_string().contains("observed a transaction id"));
    assert!(client.ambiguous_submit_block(owner).is_some());
}

#[test]
fn manual_reconciliation_clear_is_idempotent_without_owner_block() {
    let owner = address(WALLET_OWNER);
    let payload_hash =
        "0x3123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string();
    let client = test_client(DepositWalletRelayerUrl::loopback("http://127.0.0.1/").unwrap());

    let evidence = manual_reconciliation_evidence(
        owner,
        payload_hash.clone(),
        "tx-no-block",
        RelayerTransactionState::Failed,
        None,
    );
    client
        .clear_ambiguous_submit_after_manual_reconciliation(
            evidence,
            manual_reconciliation_permit_token_for(owner),
        )
        .unwrap();

    let idless = idless_reconciliation_evidence(owner, payload_hash, 1_700_000_001);
    client
        .clear_idless_ambiguous_submit_after_manual_reconciliation(
            idless,
            manual_reconciliation_permit_token_for(owner),
        )
        .unwrap();

    assert!(client.ambiguous_submit_block(owner).is_none());
    assert!(client.ambiguous_submit_transaction_ids(owner).is_empty());
}

#[test]
fn manual_reconciliation_rejects_stale_or_future_evidence_timestamps() {
    let owner = address(WALLET_OWNER);
    let payload_hash =
        "0x2123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string();
    let client = test_client(DepositWalletRelayerUrl::loopback("http://127.0.0.1/").unwrap());
    client.record_ambiguous(owner, payload_hash.clone()).unwrap();

    for (label, checked_at) in [("predates", 1_699_999_999), ("future", 1_700_000_031)] {
        let evidence = manual_reconciliation_evidence_at(
            owner,
            payload_hash.clone(),
            "tx-reconcile",
            RelayerTransactionState::Failed,
            None,
            checked_at,
        );
        let error = client
            .clear_ambiguous_submit_after_manual_reconciliation(
                evidence,
                manual_reconciliation_permit_token_for(owner),
            )
            .unwrap_err();
        assert!(
            error.is_deposit_wallet_mutation_blocked(),
            "{label}: {error}"
        );
    }

    for (label, checked_at) in [("predates", 1_699_999_999), ("future", 1_700_000_031)] {
        let evidence = idless_reconciliation_evidence(owner, payload_hash.clone(), checked_at);
        let error = client
            .clear_idless_ambiguous_submit_after_manual_reconciliation(
                evidence,
                manual_reconciliation_permit_token_for(owner),
            )
            .unwrap_err();
        assert!(
            error.is_deposit_wallet_mutation_blocked(),
            "{label}: {error}"
        );
    }
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
fn submit_response_parser_covers_abnormal_wire_shapes_and_terminal_states() {
    let alias_fixture = fixture_text("wallet_submit_response_alias.json");
    let alias_receipt = parse_submit_response(alias_fixture.as_bytes()).unwrap();
    assert_eq!(alias_receipt.transaction_id, "tx-alias-fixture");
    assert_eq!(alias_receipt.state, RelayerTransactionState::New);
    assert_eq!(alias_receipt.transaction_hash, None);
    assert_eq!(
        extract_submit_transaction_id(alias_fixture.as_bytes()),
        Some("tx-alias-fixture".to_string())
    );

    let array_body =
        json!([{"transactionID": "tx-array", "state": "STATE_NEW"}]).to_string();
    let error = parse_submit_response(array_body.as_bytes()).unwrap_err();
    assert!(error.is_deposit_wallet_reconciliation_required());

    let id_only = json!({"transactionID": "tx-id-only"}).to_string();
    assert_eq!(
        extract_submit_transaction_id(id_only.as_bytes()),
        Some("tx-id-only".to_string())
    );
    assert!(parse_submit_response(id_only.as_bytes()).is_err());
    let id_only_alias = json!({"transactionId": "tx-id-only-alias"}).to_string();
    assert_eq!(
        extract_submit_transaction_id(id_only_alias.as_bytes()),
        Some("tx-id-only-alias".to_string())
    );
    assert!(parse_submit_response(id_only_alias.as_bytes()).is_err());

    for body in [
        json!({"transactionID": "", "state": "STATE_NEW"}).to_string(),
        json!({"transactionId": "", "state": "STATE_NEW"}).to_string(),
        json!({"transactionID": " tx-leading-space", "state": "STATE_NEW"}).to_string(),
        json!({"transactionID": "tx\nnewline", "state": "STATE_NEW"}).to_string(),
    ] {
        assert!(extract_submit_transaction_id(body.as_bytes()).is_none());
        assert!(parse_submit_response(body.as_bytes()).is_err());
    }

    let malformed_hash = json!({
        "transactionID": "tx-bad-hash",
        "state": "STATE_CONFIRMED",
        "transactionHash": "0x1234"
    })
    .to_string();
    let error = parse_submit_response(malformed_hash.as_bytes()).unwrap_err();
    assert!(error.is_deposit_wallet_reconciliation_required());

    let confirmed = json!({
        "transactionID": "tx-confirmed",
        "state": "STATE_CONFIRMED",
        "transactionHash": "0x38CBFBEAE8FFFA4E2B187EE5978D3EE9CAFC53AF0363ED90A35B7EA9016535D8"
    })
    .to_string();
    let receipt = parse_submit_response(confirmed.as_bytes()).unwrap();
    assert_eq!(receipt.state, RelayerTransactionState::Confirmed);
    assert_eq!(
        receipt.transaction_hash.as_deref(),
        Some("0x38cbfbeae8fffa4e2b187ee5978d3ee9cafc53af0363ed90a35b7ea9016535d8")
    );

    for (state, expected) in [
        ("STATE_INVALID", RelayerTransactionState::Invalid),
        ("STATE_FAILED", RelayerTransactionState::Failed),
    ] {
        let body = json!({"transactionID": format!("tx-{state}"), "state": state}).to_string();
        let receipt = parse_submit_response(body.as_bytes()).unwrap();
        assert_eq!(receipt.state, expected);
        assert_eq!(receipt.transaction_hash, None);
    }

    let unknown = json!({"transactionID": "tx-future", "state": "STATE_FUTURE"}).to_string();
    let receipt = parse_submit_response(unknown.as_bytes()).unwrap();
    assert!(matches!(
        receipt.state,
        RelayerTransactionState::Unknown(ref raw) if raw == "STATE_FUTURE"
    ));
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
async fn get_transaction_for_owner_rejects_production_until_wallet_polling_evidence_is_recorded() {
    let url = DepositWalletRelayerUrl::parse("https://relayer-v2.polymarket.com").unwrap();
    let client = test_client(url);

    let error = client
        .get_transaction_for_owner(address(WALLET_OWNER), "tx-production")
        .await
        .unwrap_err();

    assert!(error.is_deposit_wallet_read_blocked());
    assert!(error.to_string().contains("WALLET polling response fixture"));
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
    assert_eq!(requests[0].path, "/transaction?transactionID=tx-owner");
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
    response.as_object_mut().unwrap().remove("transactionHash");
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
async fn get_transaction_for_owner_reports_pending_states_as_retryable_absent() {
    for (transaction_id, state) in [
        ("tx-new", "STATE_NEW"),
        ("tx-executed", "STATE_EXECUTED"),
        ("tx-mined", "STATE_MINED"),
    ] {
        let mut response = transaction_response_value(transaction_id, state);
        response.as_object_mut().unwrap().remove("transactionHash");
        let (url, handle) =
            spawn_server(vec![TestResponse::json("200 OK", response.to_string())]).await;
        let client = test_client(url);

        let error = client
            .get_transaction_for_owner(address(WALLET_OWNER), transaction_id)
            .await
            .unwrap_err();

        assert!(error.is_deposit_wallet_transaction_absent());
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
    assert_eq!(
        requests[0].path,
        "/transaction?transactionID=tx-missing-http"
    );

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
        ("empty", ""),
        ("too-short", "0x1234"),
        (
            "too-long",
            "0x38cbfbeae8fffa4e2b187ee5978d3ee9cafc53af0363ed90a35b7ea9016535d800",
        ),
        (
            "missing-prefix",
            "38cbfbeae8fffa4e2b187ee5978d3ee9cafc53af0363ed90a35b7ea9016535d8",
        ),
        (
            "uppercase-prefix",
            "0X38cbfbeae8fffa4e2b187ee5978d3ee9cafc53af0363ed90a35b7ea9016535d8",
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
        json!("0x38CBFBEAE8FFFA4E2B187EE5978D3EE9CAFC53AF0363ED90A35B7EA9016535D8");

    let parsed =
        parse_transaction_response(transaction_id, config, response.to_string().as_bytes())
            .unwrap();

    assert_eq!(
        parsed.receipt.transaction_hash.as_deref(),
        Some("0x38cbfbeae8fffa4e2b187ee5978d3ee9cafc53af0363ed90a35b7ea9016535d8")
    );
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
    let mut wallet_create_type = transaction_response_value(target, "STATE_CONFIRMED");
    wallet_create_type["type"] = json!(crate::deposit_wallet::WALLET_CREATE_TRANSACTION_TYPE);
    let wallet_create = parse_transaction_response(
        target,
        config,
        wallet_create_type.to_string().as_bytes(),
    )
    .unwrap();
    assert_eq!(wallet_create.receipt.transaction_id, target);
    assert_eq!(wallet_create.receipt.state, RelayerTransactionState::Confirmed);
    assert_eq!(wallet_create.transaction_type, WALLET_CREATE_TRANSACTION_TYPE);
    let mut wallet_without_to = transaction_response_value(target, "STATE_CONFIRMED");
    wallet_without_to.as_object_mut().unwrap().remove("to");
    let wallet_without_to =
        parse_transaction_response(target, config, wallet_without_to.to_string().as_bytes())
            .unwrap();
    assert_eq!(wallet_without_to.transaction_type, WALLET_TRANSACTION_TYPE);
    let mut wallet_to_deposit_wallet = transaction_response_value(target, "STATE_CONFIRMED");
    wallet_to_deposit_wallet["to"] = wallet_to_deposit_wallet["proxyAddress"].clone();
    let wallet_to_deposit_wallet = parse_transaction_response(
        target,
        config,
        wallet_to_deposit_wallet.to_string().as_bytes(),
    )
    .unwrap();
    assert_eq!(
        wallet_to_deposit_wallet.transaction_type,
        WALLET_TRANSACTION_TYPE
    );
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
    missing_to["type"] = json!(WALLET_CREATE_TRANSACTION_TYPE);
    missing_to.as_object_mut().unwrap().remove("to");
    let mut to_mismatch = transaction_response_value(target, "STATE_CONFIRMED");
    to_mismatch["type"] = json!(WALLET_CREATE_TRANSACTION_TYPE);
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
        .get_wallet_nonce(
            address(WALLET_OWNER),
            wallet_nonce_read_permit_for(address(WALLET_OWNER)),
        )
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
        .get_wallet_nonce(
            address(WALLET_OWNER),
            wallet_nonce_read_permit_for(address(WALLET_OWNER)),
        )
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
        .get_wallet_nonce(
            address(WALLET_OWNER),
            wallet_nonce_read_permit_for(address(WALLET_OWNER)),
        )
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
