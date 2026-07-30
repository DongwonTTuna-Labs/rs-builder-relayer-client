use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use ethers::signers::{LocalWallet, Signer};
use ethers::types::transaction::eip2718::TypedTransaction;
use ethers::types::transaction::eip712::Eip712;
use ethers::types::{Bytes, Signature};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

use crate::deposit_wallet::{
    build_wallet_create_request, deposit_wallet_contract_config, derive_deposit_wallet_address,
    try_build_wallet_batch_request_with_signature, DepositWalletBatchRequest,
    DepositWalletBatchToSign, DepositWalletCall, DepositWalletRequestContext, AMOY_CHAIN_ID,
    WALLET_CREATE_TRANSACTION_TYPE, WALLET_TRANSACTION_TYPE,
};
use crate::deposit_wallet::requests::build_wallet_batch_request_unchecked;

use super::clock::{RelayerClock, SystemClock};
use super::response::{parse_transaction_response, validate_transaction_id};
use super::*;

const API_KEY: &str = "unit-test-relayer-api-key";
const API_KEY_ADDRESS: &str = "0xA6Db23622C9EA7584D5c61C3e7497c80E2CE167B";
const WALLET_OWNER: &str = "0x6e0c80c90ea6c15917308F820Eac91Ce2724B5b5";
const OTHER_OWNER: &str = "0x0000000000000000000000000000000000000001";
const FIXED_NOW_UNIX: u64 = 1_700_000_000;
const FIXED_PERMIT_EXPIRY_UNIX: u64 = 2_000_000_000;
const EXECUTE_DEADLINE_UNIX: u64 = 1_760_000_000;
const TEST_SERVER_TIMEOUT: Duration = Duration::from_secs(2);
const NO_REQUEST_TIMEOUT: Duration = Duration::from_millis(250);

/// Synthetic throwaway key, never a real credential.
const SYNTHETIC_EXECUTE_SIGNER_KEY: [u8; 32] = [0x42u8; 32];

struct FixedClock {
    now_unix: u64,
}

impl RelayerClock for FixedClock {
    fn now_unix(&self) -> u64 {
        self.now_unix
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

fn relayer_auth() -> RelayerKeyAuth {
    RelayerKeyAuth::new(API_KEY, address(API_KEY_ADDRESS)).unwrap()
}

fn read_permit(owner: Address) -> RelayerReadPermit {
    RelayerReadPermit::for_owner(owner, POLYGON_CHAIN_ID)
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

fn polling_test_client(base_url: DepositWalletRelayerUrl) -> DepositWalletRelayerClient {
    let http = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("polling test HTTP client should build");
    DepositWalletRelayerClient::from_parts_with(
        http,
        base_url,
        relayer_auth(),
        deposit_wallet_contract_config(POLYGON_CHAIN_ID).unwrap(),
        Arc::new(SystemClock),
        false,
    )
}

fn mutation_test_client(
    base_url: DepositWalletRelayerUrl,
    now_unix: u64,
    mutation_enabled: bool,
) -> DepositWalletRelayerClient {
    DepositWalletRelayerClient::from_parts_with(
        reqwest_client(Duration::from_secs(2)),
        base_url,
        relayer_auth(),
        deposit_wallet_contract_config(POLYGON_CHAIN_ID).unwrap(),
        Arc::new(FixedClock { now_unix }),
        mutation_enabled,
    )
}

fn mutation_permit(
    mode: RelayerMutationMode,
    operation: RelayerMutationOperation,
    owner: Address,
    chain_id: u64,
    expires_at_unix: u64,
) -> RelayerMutationPermit {
    RelayerMutationPermit::try_new(
        mode,
        operation,
        owner,
        chain_id,
        expires_at_unix,
        "evidence/test",
        "approval/test",
    )
    .unwrap()
}

fn execute_signer() -> LocalWallet {
    LocalWallet::from_bytes(&SYNTHETIC_EXECUTE_SIGNER_KEY)
        .expect("synthetic signer key should be valid")
        .with_chain_id(POLYGON_CHAIN_ID)
}

fn execute_context(owner: Address) -> DepositWalletRequestContext {
    let config = deposit_wallet_contract_config(POLYGON_CHAIN_ID).unwrap();
    DepositWalletRequestContext {
        owner_address: owner,
        deposit_wallet_address: derive_deposit_wallet_address(owner, config).unwrap(),
    }
}

fn execute_calls() -> Vec<DepositWalletCall> {
    vec![DepositWalletCall {
        target: address("0x0000000000000000000000000000000000000042"),
        value: U256::zero(),
        data: Bytes::from(vec![0x12, 0x34, 0x56, 0x78]),
    }]
}

fn execute_permit(mode: RelayerMutationMode, owner: Address) -> RelayerMutationPermit {
    mutation_permit(
        mode,
        RelayerMutationOperation::WalletBatch,
        owner,
        POLYGON_CHAIN_ID,
        FIXED_PERMIT_EXPIRY_UNIX,
    )
}

async fn expected_execute_request(
    signer: &LocalWallet,
    ctx: DepositWalletRequestContext,
    calls: Vec<DepositWalletCall>,
    nonce: U256,
    deadline: U256,
) -> DepositWalletBatchRequest {
    let batch = DepositWalletBatchToSign {
        owner: ctx.owner_address,
        nonce_owner: ctx.owner_address,
        submit_from: ctx.owner_address,
        deposit_wallet: ctx.deposit_wallet_address,
        chain_id: POLYGON_CHAIN_ID,
        nonce,
        deadline,
        calls: calls.clone(),
    };
    let signature = signer.sign_typed_data(&batch).await.unwrap();

    try_build_wallet_batch_request_with_signature(
        ctx,
        deposit_wallet_contract_config(POLYGON_CHAIN_ID).unwrap(),
        nonce,
        deadline,
        calls,
        format!("0x{signature}"),
    )
    .unwrap()
}

fn assert_execute_auth_headers(request: &CapturedRequest) {
    let expected_api_key_address = to_checksum(&address(API_KEY_ADDRESS), None);
    assert_eq!(request.header("RELAYER_API_KEY"), Some(API_KEY));
    assert_eq!(
        request.header("RELAYER_API_KEY_ADDRESS"),
        Some(expected_api_key_address.as_str())
    );
}

const SIGNER_ERROR_SENTINEL: &str = "SECRET-SENTINEL-0xDEADBEEF";

#[derive(Debug)]
struct SecretSignerErrorSource;

impl std::fmt::Display for SecretSignerErrorSource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(SIGNER_ERROR_SENTINEL)
    }
}

impl std::error::Error for SecretSignerErrorSource {}

#[derive(Debug)]
struct FailingSignerError {
    source: SecretSignerErrorSource,
}

impl FailingSignerError {
    fn synthetic() -> Self {
        Self {
            source: SecretSignerErrorSource,
        }
    }
}

impl std::fmt::Display for FailingSignerError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "synthetic signer failure: {SIGNER_ERROR_SENTINEL}")
    }
}

impl std::error::Error for FailingSignerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

#[derive(Debug)]
struct FailingSigner {
    address: Address,
    chain_id: u64,
}

impl FailingSigner {
    fn new(address: Address) -> Self {
        Self {
            address,
            chain_id: POLYGON_CHAIN_ID,
        }
    }
}

impl Signer for FailingSigner {
    type Error = FailingSignerError;

    fn sign_message<'life0, 'async_trait, S>(
        &'life0 self,
        _message: S,
    ) -> Pin<
        Box<
            dyn Future<Output = std::result::Result<Signature, Self::Error>>
                + Send
                + 'async_trait,
        >,
    >
    where
        S: 'async_trait + Send + Sync + AsRef<[u8]>,
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(async { Err(FailingSignerError::synthetic()) })
    }

    fn sign_transaction<'life0, 'life1, 'async_trait>(
        &'life0 self,
        _message: &'life1 TypedTransaction,
    ) -> Pin<
        Box<
            dyn Future<Output = std::result::Result<Signature, Self::Error>>
                + Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        'life1: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(async { Err(FailingSignerError::synthetic()) })
    }

    fn sign_typed_data<'life0, 'life1, 'async_trait, T>(
        &'life0 self,
        _payload: &'life1 T,
    ) -> Pin<
        Box<
            dyn Future<Output = std::result::Result<Signature, Self::Error>>
                + Send
                + 'async_trait,
        >,
    >
    where
        T: Eip712 + Send + Sync + 'async_trait,
        'life0: 'async_trait,
        'life1: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(async { Err(FailingSignerError::synthetic()) })
    }

    fn address(&self) -> Address {
        self.address
    }

    fn chain_id(&self) -> u64 {
        self.chain_id
    }

    fn with_chain_id<T: Into<u64>>(mut self, chain_id: T) -> Self {
        self.chain_id = chain_id.into();
        self
    }
}

fn wallet_batch_request() -> DepositWalletBatchRequest {
    let fixture = fixture_value("wallet_submit_body.json");
    let params = &fixture["depositWalletParams"];
    let calls = params["calls"]
        .as_array()
        .unwrap()
        .iter()
        .map(|call| DepositWalletCall {
            target: address(call["target"].as_str().unwrap()),
            value: U256::from_dec_str(call["value"].as_str().unwrap()).unwrap(),
            data: Bytes::from(
                hex::decode(
                    call["data"]
                        .as_str()
                        .unwrap()
                        .strip_prefix("0x")
                        .unwrap(),
                )
                .unwrap(),
            ),
        })
        .collect();
    let ctx = DepositWalletRequestContext {
        owner_address: address(fixture["from"].as_str().unwrap()),
        deposit_wallet_address: address(params["depositWallet"].as_str().unwrap()),
    };

    build_wallet_batch_request_unchecked(
        ctx,
        deposit_wallet_contract_config(POLYGON_CHAIN_ID).unwrap(),
        U256::from_dec_str(fixture["nonce"].as_str().unwrap()).unwrap(),
        U256::from_dec_str(params["deadline"].as_str().unwrap()).unwrap(),
        calls,
        fixture["signature"].as_str().unwrap().to_string(),
    )
}

fn expected_payload_keccak256<T: serde::Serialize>(request: &T) -> String {
    let body = serde_json::to_vec(request).unwrap();
    format!("0x{}", hex::encode(keccak256(body)))
}

fn expect_dry_run(outcome: RelayerSubmitOutcome) -> Box<DepositWalletDryRunEvidence> {
    match outcome {
        RelayerSubmitOutcome::DryRun(evidence) => evidence,
        RelayerSubmitOutcome::Submitted(receipt) => {
            panic!("expected dry-run evidence, got submitted receipt: {receipt:?}")
        }
    }
}

fn expect_submitted(outcome: RelayerSubmitOutcome) -> DepositWalletSubmitReceipt {
    match outcome {
        RelayerSubmitOutcome::Submitted(receipt) => receipt,
        RelayerSubmitOutcome::DryRun(evidence) => {
            panic!("expected submitted receipt, got dry-run evidence: {evidence:?}")
        }
    }
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

async fn spawn_polling_server(
    responses: Vec<TestResponse>,
) -> (DepositWalletRelayerUrl, JoinHandle<Vec<CapturedRequest>>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("test server should bind");
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let mut requests = Vec::with_capacity(responses.len());
        for response in responses {
            let (mut stream, _) = listener.accept().await.expect("polling server should accept");
            let request = read_polling_request(&mut stream).await;
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

async fn spawn_polling_reset_then_response_server(
    response: TestResponse,
) -> (DepositWalletRelayerUrl, JoinHandle<Vec<CapturedRequest>>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("polling reset server should bind");
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let (mut reset_stream, _) = listener
            .accept()
            .await
            .expect("polling reset server should accept first request");
        let reset_request = read_polling_request(&mut reset_stream).await;
        drop(reset_stream);

        let (mut response_stream, _) = listener
            .accept()
            .await
            .expect("polling reset server should accept retry");
        let retry_request = read_polling_request(&mut response_stream).await;
        write_response(&mut response_stream, response).await;

        vec![reset_request, retry_request]
    });

    (
        DepositWalletRelayerUrl::loopback(&format!("http://{addr}")).unwrap(),
        handle,
    )
}

async fn spawn_polling_held_response_server() -> (
    DepositWalletRelayerUrl,
    JoinHandle<Vec<CapturedRequest>>,
    tokio::sync::oneshot::Receiver<()>,
    tokio::sync::oneshot::Sender<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("polling held-response server should bind");
    let addr = listener.local_addr().unwrap();
    let (request_seen_tx, request_seen_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let handle = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("polling held-response server should accept");
        let request = read_polling_request(&mut stream).await;
        request_seen_tx
            .send(())
            .expect("polling request observer should remain available");
        release_rx
            .await
            .expect("polling held-response server should be released");
        drop(stream);
        vec![request]
    });

    (
        DepositWalletRelayerUrl::loopback(&format!("http://{addr}")).unwrap(),
        handle,
        request_seen_rx,
        release_tx,
    )
}

async fn spawn_polling_response_then_watch_for_retry(
    response: TestResponse,
) -> (
    DepositWalletRelayerUrl,
    JoinHandle<Vec<CapturedRequest>>,
    tokio::sync::oneshot::Receiver<()>,
    tokio::sync::oneshot::Sender<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("polling backoff-cancel server should bind");
    let addr = listener.local_addr().unwrap();
    let (armed_tx, armed_rx) = tokio::sync::oneshot::channel();
    let (stop_tx, mut stop_rx) = tokio::sync::oneshot::channel();
    let handle = tokio::spawn(async move {
        let (mut stream, _) = listener
            .accept()
            .await
            .expect("polling backoff-cancel server should accept first request");
        let first_request = read_polling_request(&mut stream).await;
        write_response(&mut stream, response).await;
        stream
            .flush()
            .await
            .expect("polling response should flush before cancellation is armed");
        armed_tx
            .send(())
            .expect("polling cancellation receiver should remain available");
        drop(stream);

        let mut requests = vec![first_request];
        loop {
            tokio::select! {
                biased;
                accepted = listener.accept() => {
                    let (mut retry_stream, _) =
                        accepted.expect("polling retry accept should succeed");
                    requests.push(read_polling_request(&mut retry_stream).await);
                }
                _ = &mut stop_rx => break,
            }
        }
        requests
    });

    (
        DepositWalletRelayerUrl::loopback(&format!("http://{addr}")).unwrap(),
        handle,
        armed_rx,
        stop_tx,
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

async fn spawn_optional_request_server(
) -> (DepositWalletRelayerUrl, JoinHandle<Vec<CapturedRequest>>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("test server should bind");
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let Ok(accepted) = tokio::time::timeout(NO_REQUEST_TIMEOUT, listener.accept()).await else {
            return Vec::new();
        };
        let (mut stream, _) = accepted.expect("server should accept");
        let request = read_request(&mut stream).await;
        write_response(
            &mut stream,
            TestResponse::json("500 Internal Server Error", "{}"),
        )
        .await;
        vec![request]
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

async fn spawn_nonce_then_reset_server(
    nonce_response: TestResponse,
) -> (DepositWalletRelayerUrl, JoinHandle<Vec<CapturedRequest>>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("test server should bind");
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let (mut nonce_stream, _) = tokio::time::timeout(TEST_SERVER_TIMEOUT, listener.accept())
            .await
            .expect("nonce accept should not hang")
            .expect("nonce connection should accept");
        let nonce_request = read_request(&mut nonce_stream).await;
        write_response(&mut nonce_stream, nonce_response).await;
        drop(nonce_stream);

        let (mut submit_stream, _) = tokio::time::timeout(TEST_SERVER_TIMEOUT, listener.accept())
            .await
            .expect("submit accept should not hang")
            .expect("submit connection should accept");
        let submit_request = read_request(&mut submit_stream).await;
        drop(submit_stream);

        let mut requests = vec![nonce_request, submit_request];
        if let Ok(Ok((mut retry_stream, _))) =
            tokio::time::timeout(NO_REQUEST_TIMEOUT, listener.accept()).await
        {
            requests.push(read_request(&mut retry_stream).await);
            write_response(
                &mut retry_stream,
                TestResponse::json("500 Internal Server Error", "{}"),
            )
            .await;
        }
        requests
    });

    (
        DepositWalletRelayerUrl::loopback(&format!("http://{addr}")).unwrap(),
        handle,
    )
}

async fn spawn_single_response_and_watch_for_retry(
    response: Option<TestResponse>,
) -> (
    DepositWalletRelayerUrl,
    JoinHandle<Vec<CapturedRequest>>,
    tokio::sync::oneshot::Sender<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("test server should bind");
    let addr = listener.local_addr().unwrap();
    let (stop_tx, mut stop_rx) = tokio::sync::oneshot::channel();
    let handle = tokio::spawn(async move {
        let (mut stream, _) = tokio::time::timeout(TEST_SERVER_TIMEOUT, listener.accept())
            .await
            .expect("server accept should not hang")
            .expect("server should accept");
        let first_request = read_request(&mut stream).await;
        if let Some(response) = response {
            write_response(&mut stream, response).await;
        }
        drop(stream);

        let mut requests = vec![first_request];
        loop {
            tokio::select! {
                _ = &mut stop_rx => break,
                accepted = listener.accept() => {
                    let (mut retry_stream, _) = accepted.expect("retry accept should succeed");
                    requests.push(read_request(&mut retry_stream).await);
                    write_response(
                        &mut retry_stream,
                        TestResponse::json("500 Internal Server Error", "{}"),
                    )
                    .await;
                }
            }
        }
        requests
    });

    (
        DepositWalletRelayerUrl::loopback(&format!("http://{addr}")).unwrap(),
        handle,
        stop_tx,
    )
}

async fn finish_retry_watch(
    stop_tx: tokio::sync::oneshot::Sender<()>,
    handle: JoinHandle<Vec<CapturedRequest>>,
) -> Vec<CapturedRequest> {
    tokio::task::yield_now().await;
    let _ = stop_tx.send(());
    handle.await.unwrap()
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

async fn read_polling_request(stream: &mut TcpStream) -> CapturedRequest {
    let mut buffer = Vec::new();
    let headers_end = loop {
        let mut chunk = [0u8; 1024];
        let read = stream
            .read(&mut chunk)
            .await
            .expect("polling request should read");
        assert!(read > 0, "polling request ended before headers completed");
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
        let read = stream
            .read(&mut chunk)
            .await
            .expect("polling request body should read");
        assert!(read > 0, "polling request ended before body completed");
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
    let fixture = fixture_value("wallet_create_transaction_response.json");
    let mut response = fixture
        .as_array()
        .and_then(|items| items.first())
        .cloned()
        .expect("WALLET-CREATE transaction fixture should contain one response");
    response["transactionID"] = json!(transaction_id);
    response["state"] = json!(state);
    if state != "STATE_CONFIRMED" {
        response
            .as_object_mut()
            .unwrap()
            .remove("transactionHash");
    }
    response
}

fn assert_deployed_request(request: &CapturedRequest) {
    let fixture = fixture_value("wallet_deployed_http_request.json");
    assert_eq!(request.method, fixture["method"].as_str().unwrap());
    assert_eq!(request.path, fixture["pathAndQuery"].as_str().unwrap());
    assert_eq!(request.header("RELAYER_API_KEY"), Some(API_KEY));
    assert_eq!(
        request.header("RELAYER_API_KEY_ADDRESS"),
        Some(to_checksum(&address(API_KEY_ADDRESS), None).as_str())
    );
    assert!(request.body.is_empty());
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
    assert!(DepositWalletRelayerUrl::parse("http://relayer-v2.polymarket.com").is_err());
    assert!(DepositWalletRelayerUrl::parse("https://example.com").is_err());
    assert!(DepositWalletRelayerUrl::parse("https://relayer-v2.polymarket.com/path").is_err());
    assert!(DepositWalletRelayerUrl::parse("https://user@relayer-v2.polymarket.com").is_err());
    assert!(DepositWalletRelayerUrl::parse("https://relayer-v2.polymarket.com:8443").is_err());
    assert!(DepositWalletRelayerUrl::parse("https://relayer-v2.polymarket.com?x=1").is_err());
    assert!(DepositWalletRelayerUrl::parse("https://relayer-v2.polymarket.com/#frag").is_err());
    assert!(DepositWalletRelayerUrl::loopback("http://[::1]/").is_ok());
    assert!(DepositWalletRelayerUrl::loopback("http://user@127.0.0.1/").is_err());
    assert!(DepositWalletRelayerUrl::loopback("http://127.0.0.1/?x=1").is_err());
    assert!(DepositWalletRelayerUrl::loopback("http://127.0.0.1/#frag").is_err());
    assert!(DepositWalletRelayerUrl::loopback("http://127.0.0.1/path").is_err());
    assert!(DepositWalletRelayerUrl::loopback("http://192.0.2.1/").is_err());

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

#[test]
fn relayer_read_permit_is_owner_chain_scoped_and_redacts_debug_owner() {
    let owner = address(WALLET_OWNER);
    let permit = read_permit(owner);
    let debug = format!("{permit:?}");
    let owner_checksum = to_checksum(&owner, None);
    let redacted_owner = super::redaction::redacted_address(owner);

    assert_eq!(permit.owner(), owner);
    assert_eq!(permit.chain_id(), POLYGON_CHAIN_ID);
    assert!(debug.contains(&redacted_owner));
    assert!(debug.contains("chain_id: 137"));
    assert!(!debug.contains(&owner_checksum));
    assert!(!debug.contains(&owner_checksum.to_ascii_lowercase()));
}

#[tokio::test]
async fn is_deposit_wallet_deployed_matches_request_and_response_fixtures() {
    let request_fixture = fixture_value("wallet_deployed_http_request.json");
    let response_fixture = fixture_value("wallet_deployed_response_cases.json");
    let accepted = response_fixture["accepted"].as_array().unwrap();
    let responses = accepted
        .iter()
        .map(|case| {
            TestResponse::json("200 OK", case["raw"].as_str().expect("raw response string"))
        })
        .collect();
    let (url, handle) = spawn_server(responses).await;
    let client = test_client(url);
    let owner = address(request_fixture["owner"].as_str().unwrap());
    let permit = read_permit(owner);
    let derived_wallet = derive_deposit_wallet_address(
        owner,
        deposit_wallet_contract_config(POLYGON_CHAIN_ID).unwrap(),
    )
    .unwrap();

    assert_eq!(
        to_checksum(&derived_wallet, None),
        request_fixture["address"].as_str().unwrap()
    );
    for case in accepted {
        let deployed = client
            .is_deposit_wallet_deployed(owner, &permit)
            .await
            .unwrap();
        assert_eq!(
            deployed,
            case["expected"].as_bool().unwrap(),
            "{}",
            case["label"].as_str().unwrap()
        );
    }

    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), accepted.len());
    for request in requests {
        assert_eq!(request.method, request_fixture["method"].as_str().unwrap());
        assert_eq!(
            request.path,
            request_fixture["pathAndQuery"].as_str().unwrap()
        );
        assert!(request.header("RELAYER_API_KEY").is_some());
        assert!(request.header("RELAYER_API_KEY_ADDRESS").is_some());
        assert!(request.body.is_empty());
    }
}

#[tokio::test]
async fn is_deposit_wallet_deployed_rejects_malformed_fixture_responses() {
    let fixture = fixture_value("wallet_deployed_response_cases.json");
    let rejected = fixture["rejected"].as_array().unwrap();
    let responses = rejected
        .iter()
        .map(|case| {
            TestResponse::json("200 OK", case["raw"].as_str().expect("raw response string"))
        })
        .collect();
    let (url, handle) = spawn_server(responses).await;
    let client = test_client(url);
    let owner = address(WALLET_OWNER);
    let permit = read_permit(owner);

    for case in rejected {
        let error = client
            .is_deposit_wallet_deployed(owner, &permit)
            .await
            .unwrap_err();
        assert!(
            matches!(error, RelayerError::Other(ref message) if message == "could not parse deployed response"),
            "{}: {error}",
            case["label"].as_str().unwrap()
        );
    }

    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), rejected.len());
}

#[tokio::test]
async fn is_deposit_wallet_deployed_rejects_oversized_response() {
    let oversized_body = "x".repeat(MAX_SUCCESS_BODY_BYTES + 1);
    let (url, handle) = spawn_server(vec![TestResponse::json_without_content_length(
        "200 OK",
        oversized_body,
    )])
    .await;
    let client = test_client(url);
    let owner = address(WALLET_OWNER);

    let error = client
        .is_deposit_wallet_deployed(owner, &read_permit(owner))
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        RelayerError::Other(ref message) if message == RESPONSE_BODY_TOO_LARGE_MESSAGE
    ));
    assert_eq!(handle.await.unwrap().len(), 1);
}

#[tokio::test]
async fn is_deposit_wallet_deployed_returns_typed_api_error_for_5xx() {
    let (url, handle) = spawn_server(vec![TestResponse::json(
        "503 Service Unavailable",
        "{}",
    )])
    .await;
    let client = test_client(url);
    let owner = address(WALLET_OWNER);

    let error = client
        .is_deposit_wallet_deployed(owner, &read_permit(owner))
        .await
        .unwrap_err();

    assert!(matches!(error, RelayerError::Api { status: 503, .. }));
    assert_eq!(handle.await.unwrap().len(), 1);
}

#[tokio::test]
async fn read_methods_reject_owner_mismatched_permit_before_input_or_http() {
    let (url, handle) = spawn_optional_request_server().await;
    let client = test_client(url);
    let owner = address(WALLET_OWNER);
    let permit = read_permit(address(OTHER_OWNER));

    let deployed_error = client
        .is_deposit_wallet_deployed(owner, &permit)
        .await
        .unwrap_err();
    let nonce_error = client
        .get_wallet_nonce(owner, &permit)
        .await
        .unwrap_err();
    let transaction_error = client
        .get_transaction_for_owner(owner, "", &permit)
        .await
        .unwrap_err();

    for error in [deployed_error, nonce_error, transaction_error] {
        assert!(error.is_deposit_wallet_read_blocked(), "{error}");
    }
    assert!(handle.await.unwrap().is_empty());
}

#[tokio::test]
async fn read_methods_reject_chain_mismatched_permit_before_input_or_http() {
    let (url, handle) = spawn_optional_request_server().await;
    let client = test_client(url);
    let owner = address(WALLET_OWNER);
    let permit = RelayerReadPermit::for_owner(owner, AMOY_CHAIN_ID);

    let deployed_error = client
        .is_deposit_wallet_deployed(owner, &permit)
        .await
        .unwrap_err();
    let nonce_error = client
        .get_wallet_nonce(owner, &permit)
        .await
        .unwrap_err();
    let transaction_error = client
        .get_transaction_for_owner(owner, "", &permit)
        .await
        .unwrap_err();

    for error in [deployed_error, nonce_error, transaction_error] {
        assert!(error.is_deposit_wallet_read_blocked(), "{error}");
    }
    assert!(handle.await.unwrap().is_empty());
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
        .get_wallet_nonce(owner, &read_permit(owner))
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

    let owner = address(WALLET_OWNER);
    let error = client
        .get_wallet_nonce(owner, &read_permit(owner))
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
    let target_requests = target_handle.await.unwrap();
    assert!(target_requests.is_empty());
}

#[tokio::test]
async fn get_wallet_nonce_rejects_mismatched_permit_on_production_before_http() {
    let url = DepositWalletRelayerUrl::parse("https://relayer-v2.polymarket.com").unwrap();
    let client = test_client(url);
    let permit = read_permit(address(OTHER_OWNER));

    let error = client
        .get_wallet_nonce(address(WALLET_OWNER), &permit)
        .await
        .unwrap_err();

    assert!(error.is_deposit_wallet_read_blocked());
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
        WALLET_TRANSACTION_TYPE,
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
        WALLET_TRANSACTION_TYPE,
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
async fn get_transaction_for_owner_rejects_mismatched_permit_on_production_before_http() {
    let url = DepositWalletRelayerUrl::parse("https://relayer-v2.polymarket.com").unwrap();
    let client = test_client(url);
    let permit = read_permit(address(OTHER_OWNER));

    let error = client
        .get_transaction_for_owner(address(WALLET_OWNER), "tx-production", &permit)
        .await
        .unwrap_err();

    assert!(error.is_deposit_wallet_read_blocked());
}

#[tokio::test]
async fn get_transaction_for_owner_rejects_invalid_transaction_id_before_http() {
    let (url, handle) = spawn_server(Vec::new()).await;
    let client = test_client(url);

    for transaction_id in ["", " tx-leading-space", "tx-trailing-space ", "tx\nnewline"] {
        let error = client
            .get_transaction_for_owner(
                address(WALLET_OWNER),
                transaction_id,
                &read_permit(address(WALLET_OWNER)),
            )
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
        .get_transaction_for_owner(
            address(WALLET_OWNER),
            "tx-owner",
            &read_permit(address(WALLET_OWNER)),
        )
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
        .get_transaction_for_owner(
            address(WALLET_OWNER),
            "tx-confirmed-no-hash",
            &read_permit(address(WALLET_OWNER)),
        )
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
            .get_transaction_for_owner(
                address(WALLET_OWNER),
                transaction_id,
                &read_permit(address(WALLET_OWNER)),
            )
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
        .get_transaction_for_owner(
            address(WALLET_OWNER),
            transaction_id,
            &read_permit(address(WALLET_OWNER)),
        )
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
        .get_transaction_for_owner(
            address(WALLET_OWNER),
            "tx-owner",
            &read_permit(address(WALLET_OWNER)),
        )
        .await
        .unwrap_err();

    assert!(error.is_deposit_wallet_reconciliation_required());
    assert!(error.to_string().contains("did not match requested owner"));
    assert!(!error.to_string().contains(OTHER_OWNER));
    let _ = handle.await.unwrap();
}

#[tokio::test]
async fn get_transaction_for_owner_rejects_missing_owner_evidence() {
    let mut response = transaction_response_value("tx-missing-owner", "STATE_CONFIRMED");
    response.as_object_mut().unwrap().remove("owner");
    let (url, handle) = spawn_server(vec![TestResponse::json(
        "200 OK",
        response.to_string(),
    )])
    .await;
    let client = test_client(url);

    let error = client
        .get_transaction_for_owner(
            address(WALLET_OWNER),
            "tx-missing-owner",
            &read_permit(address(WALLET_OWNER)),
        )
        .await
        .unwrap_err();

    assert!(error.is_deposit_wallet_reconciliation_required());
    assert!(error.to_string().contains("did not include owner evidence"));
    assert_eq!(handle.await.unwrap().len(), 1);
}

#[tokio::test]
async fn get_transaction_for_owner_covers_404_missing_array_and_transient_errors() {
    let (url, handle) = spawn_server(vec![TestResponse::json("404 Not Found", "{}")]).await;
    let client = test_client(url);

    let error = client
        .get_transaction_for_owner(
            address(WALLET_OWNER),
            "tx-missing-http",
            &read_permit(address(WALLET_OWNER)),
        )
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
        .get_transaction_for_owner(
            address(WALLET_OWNER),
            "tx-missing-array",
            &read_permit(address(WALLET_OWNER)),
        )
        .await
        .unwrap_err();
    assert!(error.is_deposit_wallet_transaction_absent());
    assert!(error.to_string().contains("did not include requested transaction id hash"));
    let _ = handle.await.unwrap();

    let (url, handle) = spawn_reset_server().await;
    let client = test_client(url);
    let error = client
        .get_transaction_for_owner(
            address(WALLET_OWNER),
            "tx-reset",
            &read_permit(address(WALLET_OWNER)),
        )
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
        WALLET_TRANSACTION_TYPE,
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
        WALLET_TRANSACTION_TYPE,
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
        WALLET_TRANSACTION_TYPE,
        deposit_wallet_contract_config(137).unwrap(),
        body.as_bytes(),
    )
    .unwrap();
    assert_eq!(parsed.receipt.transaction_id, target);

    let error = parse_transaction_response(
        target,
        WALLET_TRANSACTION_TYPE,
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
        WALLET_TRANSACTION_TYPE,
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
        WALLET_TRANSACTION_TYPE,
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
            WALLET_TRANSACTION_TYPE,
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
        WALLET_TRANSACTION_TYPE,
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
            WALLET_TRANSACTION_TYPE,
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
        let object_error = parse_transaction_response(
            target,
            WALLET_TRANSACTION_TYPE,
            config,
            response.to_string().as_bytes(),
        )
        .unwrap_err()
        .error;
        assert!(
            matches!(object_error, RelayerError::Other(ref message) if message.contains("could not parse transaction response object")),
            "{label}: {object_error}"
        );

        let array_error = parse_transaction_response(
            target,
            WALLET_TRANSACTION_TYPE,
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

    let parsed = parse_transaction_response(
        transaction_id,
        WALLET_TRANSACTION_TYPE,
        config,
        response.to_string().as_bytes(),
    )
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
            WALLET_TRANSACTION_TYPE,
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
            WALLET_TRANSACTION_TYPE,
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
        let error = parse_transaction_response(
            target,
            WALLET_TRANSACTION_TYPE,
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
            error.to_string().contains(expected_message),
            "{label} produced unexpected error: {error}"
        );

        let array_error = parse_transaction_response(
            target,
            WALLET_TRANSACTION_TYPE,
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
            &read_permit(address(WALLET_OWNER)),
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
            &read_permit(address(WALLET_OWNER)),
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
            &read_permit(address(WALLET_OWNER)),
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
        .get_transaction_for_owner(
            address(WALLET_OWNER),
            "tx-large-response",
            &read_permit(address(WALLET_OWNER)),
        )
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
        .get_transaction_for_owner(
            address(WALLET_OWNER),
            "tx-too-large-response",
            &read_permit(address(WALLET_OWNER)),
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

#[tokio::test]
async fn mutation_is_default_deny_for_wallet_create_and_wallet_batch_before_http() {
    let (url, handle) = spawn_optional_request_server().await;
    let owner = address(WALLET_OWNER);
    let client = DepositWalletRelayerClient::new(
        url.clone(),
        relayer_auth(),
        deposit_wallet_contract_config(POLYGON_CHAIN_ID).unwrap(),
    )
    .unwrap();
    let create_permit = mutation_permit(
        RelayerMutationMode::Live,
        RelayerMutationOperation::WalletCreate,
        owner,
        POLYGON_CHAIN_ID,
        u64::MAX,
    );

    let create_error = client
        .submit_wallet_create(owner, &create_permit)
        .await
        .unwrap_err();
    assert!(create_error.is_deposit_wallet_mutation_blocked());
    assert!(create_error.to_string().contains("relayer mutation is disabled"));

    let batch_client = mutation_test_client(url, FIXED_NOW_UNIX, false);
    let batch_permit = mutation_permit(
        RelayerMutationMode::Live,
        RelayerMutationOperation::WalletBatch,
        owner,
        POLYGON_CHAIN_ID,
        FIXED_PERMIT_EXPIRY_UNIX,
    );
    let batch_error = batch_client
        .submit_signed_wallet_batch(wallet_batch_request(), &batch_permit)
        .await
        .unwrap_err();
    assert!(batch_error.is_deposit_wallet_mutation_blocked());
    assert!(batch_error.to_string().contains("relayer mutation is disabled"));

    assert!(handle.await.unwrap().is_empty());
}

#[test]
fn mutation_permit_validates_references_expiry_and_scoped_getters() {
    let owner = address(WALLET_OWNER);
    let permit = RelayerMutationPermit::try_new(
        RelayerMutationMode::DryRun,
        RelayerMutationOperation::WalletCreate,
        owner,
        POLYGON_CHAIN_ID,
        FIXED_PERMIT_EXPIRY_UNIX,
        "  evidence/DON-78  ",
        "  approval/DON-78  ",
    )
    .unwrap();
    assert_eq!(permit.mode(), RelayerMutationMode::DryRun);
    assert_eq!(
        permit.operation(),
        RelayerMutationOperation::WalletCreate
    );
    assert_eq!(permit.owner(), owner);
    assert_eq!(permit.chain_id(), POLYGON_CHAIN_ID);
    assert_eq!(permit.expires_at_unix(), FIXED_PERMIT_EXPIRY_UNIX);

    for (label, evidence_ref, approval_ref) in [
        ("empty evidence", "", "approval"),
        ("blank evidence", "   ", "approval"),
        ("control evidence", "ticket\ninternal", "approval"),
        ("empty approval", "evidence", ""),
        ("blank approval", "evidence", "   "),
        ("control approval", "evidence", "ticket\tinternal"),
    ] {
        let error = RelayerMutationPermit::try_new(
            RelayerMutationMode::DryRun,
            RelayerMutationOperation::WalletCreate,
            owner,
            POLYGON_CHAIN_ID,
            FIXED_PERMIT_EXPIRY_UNIX,
            evidence_ref,
            approval_ref,
        )
        .unwrap_err();
        assert!(
            error.is_deposit_wallet_mutation_blocked(),
            "{label}: {error}"
        );
    }

    let oversized = "x".repeat(257);
    for (label, evidence_ref, approval_ref) in [
        ("oversized evidence", oversized.as_str(), "approval"),
        ("oversized approval", "evidence", oversized.as_str()),
    ] {
        let error = RelayerMutationPermit::try_new(
            RelayerMutationMode::Live,
            RelayerMutationOperation::WalletBatch,
            owner,
            POLYGON_CHAIN_ID,
            FIXED_PERMIT_EXPIRY_UNIX,
            evidence_ref,
            approval_ref,
        )
        .unwrap_err();
        assert!(
            error.is_deposit_wallet_mutation_blocked(),
            "{label}: {error}"
        );
    }

    let zero_expiry_error = RelayerMutationPermit::try_new(
        RelayerMutationMode::Live,
        RelayerMutationOperation::WalletCreate,
        owner,
        POLYGON_CHAIN_ID,
        0,
        "evidence",
        "approval",
    )
    .unwrap_err();
    assert!(zero_expiry_error.is_deposit_wallet_mutation_blocked());
}

#[tokio::test]
async fn mutation_permit_scope_and_expiry_fail_before_http() {
    let (url, handle) = spawn_optional_request_server().await;
    let client = mutation_test_client(url, FIXED_NOW_UNIX, true);
    let owner = address(WALLET_OWNER);

    let create_operation_mismatch = mutation_permit(
        RelayerMutationMode::Live,
        RelayerMutationOperation::WalletBatch,
        owner,
        POLYGON_CHAIN_ID,
        FIXED_PERMIT_EXPIRY_UNIX,
    );
    let error = client
        .submit_wallet_create(owner, &create_operation_mismatch)
        .await
        .unwrap_err();
    assert!(error.is_deposit_wallet_mutation_blocked());

    let batch_operation_mismatch = mutation_permit(
        RelayerMutationMode::Live,
        RelayerMutationOperation::WalletCreate,
        owner,
        POLYGON_CHAIN_ID,
        FIXED_PERMIT_EXPIRY_UNIX,
    );
    let error = client
        .submit_signed_wallet_batch(wallet_batch_request(), &batch_operation_mismatch)
        .await
        .unwrap_err();
    assert!(error.is_deposit_wallet_mutation_blocked());

    let owner_mismatch = mutation_permit(
        RelayerMutationMode::Live,
        RelayerMutationOperation::WalletCreate,
        address(OTHER_OWNER),
        POLYGON_CHAIN_ID,
        FIXED_PERMIT_EXPIRY_UNIX,
    );
    let error = client
        .submit_wallet_create(owner, &owner_mismatch)
        .await
        .unwrap_err();
    assert!(error.is_deposit_wallet_mutation_blocked());

    let chain_mismatch = mutation_permit(
        RelayerMutationMode::Live,
        RelayerMutationOperation::WalletCreate,
        owner,
        AMOY_CHAIN_ID,
        FIXED_PERMIT_EXPIRY_UNIX,
    );
    let error = client
        .submit_wallet_create(owner, &chain_mismatch)
        .await
        .unwrap_err();
    assert!(error.is_deposit_wallet_mutation_blocked());

    let expired = mutation_permit(
        RelayerMutationMode::Live,
        RelayerMutationOperation::WalletCreate,
        owner,
        POLYGON_CHAIN_ID,
        FIXED_NOW_UNIX,
    );
    let error = client
        .submit_wallet_create(owner, &expired)
        .await
        .unwrap_err();
    assert!(error.is_deposit_wallet_mutation_blocked());
    assert!(error.to_string().contains("mutation permit expired"));

    assert!(handle.await.unwrap().is_empty());
}

#[tokio::test]
async fn wallet_batch_deadline_guard_treats_equal_clock_as_expired_before_http() {
    let fixture_deadline = 1_760_000_000;
    let (url, handle) = spawn_optional_request_server().await;
    let client = mutation_test_client(url, fixture_deadline, true);
    let owner = address(WALLET_OWNER);
    let permit = mutation_permit(
        RelayerMutationMode::Live,
        RelayerMutationOperation::WalletBatch,
        owner,
        POLYGON_CHAIN_ID,
        FIXED_PERMIT_EXPIRY_UNIX,
    );

    let error = client
        .submit_signed_wallet_batch(wallet_batch_request(), &permit)
        .await
        .unwrap_err();
    assert!(error.is_deposit_wallet_mutation_blocked());
    assert!(error.to_string().contains("batch deadline expired"));
    assert!(handle.await.unwrap().is_empty());
}

#[tokio::test]
async fn dry_run_builds_redacted_create_and_batch_evidence_without_http() {
    let (url, handle) = spawn_optional_request_server().await;
    let client = mutation_test_client(url, FIXED_NOW_UNIX, false);
    let owner = address(WALLET_OWNER);
    let config = deposit_wallet_contract_config(POLYGON_CHAIN_ID).unwrap();

    let create_permit = RelayerMutationPermit::try_new(
        RelayerMutationMode::DryRun,
        RelayerMutationOperation::WalletCreate,
        owner,
        POLYGON_CHAIN_ID,
        FIXED_PERMIT_EXPIRY_UNIX,
        "  evidence/create-001  ",
        "  approval/create-001  ",
    )
    .unwrap();
    let create_request = build_wallet_create_request(owner, config);
    let create_expected_hash = expected_payload_keccak256(&create_request);
    let create_evidence = expect_dry_run(
        client
            .submit_wallet_create(owner, &create_permit)
            .await
            .unwrap(),
    );
    let expected_wallet = derive_deposit_wallet_address(owner, config).unwrap();

    assert_eq!(
        create_evidence.operation(),
        WALLET_CREATE_TRANSACTION_TYPE
    );
    assert_eq!(create_evidence.endpoint_path(), SUBMIT_PATH);
    assert_eq!(create_evidence.chain_id(), POLYGON_CHAIN_ID);
    assert_eq!(
        create_evidence.owner(),
        super::redaction::redacted_address(owner)
    );
    assert_eq!(
        create_evidence.deposit_wallet(),
        super::redaction::redacted_address(expected_wallet)
    );
    assert_eq!(create_evidence.to(), to_checksum(&config.factory, None));
    assert_eq!(
        create_evidence.payload_keccak256(),
        create_expected_hash
    );
    assert_eq!(create_evidence.nonce(), None);
    assert_eq!(create_evidence.deadline(), None);
    assert!(create_evidence.calls().is_empty());
    assert_eq!(create_evidence.evidence_ref(), "evidence/create-001");
    assert_eq!(
        create_evidence.operator_approval_ref(),
        "approval/create-001"
    );
    assert_eq!(
        create_evidence.redaction(),
        "signature, auth headers, and full submit body are intentionally omitted"
    );
    let create_json = serde_json::to_value(create_evidence.as_ref()).unwrap();
    assert_eq!(create_json["evidence_ref"], json!("evidence/create-001"));
    assert_eq!(
        create_json["operator_approval_ref"],
        json!("approval/create-001")
    );

    let batch_permit = RelayerMutationPermit::try_new(
        RelayerMutationMode::DryRun,
        RelayerMutationOperation::WalletBatch,
        owner,
        POLYGON_CHAIN_ID,
        FIXED_PERMIT_EXPIRY_UNIX,
        "  evidence/batch-001  ",
        "  approval/batch-001  ",
    )
    .unwrap();
    let batch_request = wallet_batch_request();
    let batch_expected_hash = expected_payload_keccak256(&batch_request);
    let batch_evidence = expect_dry_run(
        client
            .submit_signed_wallet_batch(batch_request, &batch_permit)
            .await
            .unwrap(),
    );
    let batch_fixture = fixture_value("wallet_submit_body.json");
    let call_fixture = &batch_fixture["depositWalletParams"]["calls"][0];
    let call = &batch_evidence.calls()[0];

    assert_eq!(batch_evidence.operation(), WALLET_TRANSACTION_TYPE);
    assert_eq!(batch_evidence.endpoint_path(), SUBMIT_PATH);
    assert_eq!(batch_evidence.chain_id(), POLYGON_CHAIN_ID);
    assert_eq!(
        batch_evidence.owner(),
        super::redaction::redacted_address(owner)
    );
    assert_eq!(
        batch_evidence.deposit_wallet(),
        super::redaction::redacted_address(address(
            batch_fixture["depositWalletParams"]["depositWallet"]
                .as_str()
                .unwrap()
        ))
    );
    assert_eq!(batch_evidence.to(), to_checksum(&config.factory, None));
    assert_eq!(batch_evidence.payload_keccak256(), batch_expected_hash);
    assert_eq!(batch_evidence.nonce(), Some("31"));
    assert_eq!(batch_evidence.deadline(), Some("1760000000"));
    assert_eq!(batch_evidence.calls().len(), 1);
    assert_eq!(
        call.target(),
        super::redaction::redacted_address(address(call_fixture["target"].as_str().unwrap()))
    );
    assert_eq!(call.value(), "0");
    assert_eq!(call.selector(), Some("0x095ea7b3"));
    assert_eq!(call.data_len(), 68);
    assert_eq!(batch_evidence.evidence_ref(), "evidence/batch-001");
    assert_eq!(
        batch_evidence.operator_approval_ref(),
        "approval/batch-001"
    );
    assert_eq!(
        batch_evidence.redaction(),
        "signature, auth headers, and full submit body are intentionally omitted"
    );
    let batch_json = serde_json::to_value(batch_evidence.as_ref()).unwrap();
    assert_eq!(batch_json["evidence_ref"], json!("evidence/batch-001"));
    assert_eq!(
        batch_json["operator_approval_ref"],
        json!("approval/batch-001")
    );

    assert!(handle.await.unwrap().is_empty());
}

#[tokio::test]
async fn dry_run_and_permit_debug_redact_replayable_and_authorization_material() {
    let (url, handle) = spawn_optional_request_server().await;
    let client = mutation_test_client(url, FIXED_NOW_UNIX, false);
    let owner = address(WALLET_OWNER);
    let permit = RelayerMutationPermit::try_new(
        RelayerMutationMode::DryRun,
        RelayerMutationOperation::WalletBatch,
        owner,
        POLYGON_CHAIN_ID,
        FIXED_PERMIT_EXPIRY_UNIX,
        " evidence-sensitive-ref ",
        " operator-sensitive-ref ",
    )
    .unwrap();
    let fixture = fixture_value("wallet_submit_body.json");
    let signature = fixture["signature"].as_str().unwrap();
    let calldata = fixture["depositWalletParams"]["calls"][0]["data"]
        .as_str()
        .unwrap();

    let evidence = expect_dry_run(
        client
            .submit_signed_wallet_batch(wallet_batch_request(), &permit)
            .await
            .unwrap(),
    );
    let evidence_debug = format!("{evidence:?}");
    let evidence_json = serde_json::to_string(evidence.as_ref()).unwrap();
    for forbidden in [signature, calldata, API_KEY, "RELAYER_API_KEY"] {
        assert!(
            !evidence_debug.contains(forbidden),
            "evidence Debug leaked forbidden material"
        );
        assert!(
            !evidence_json.contains(forbidden),
            "evidence JSON leaked forbidden material"
        );
    }
    let evidence_object = serde_json::to_value(evidence.as_ref()).unwrap();
    assert!(evidence_object.get("signature").is_none());
    assert!(evidence_object.get("data").is_none());
    assert!(evidence_object.get("body").is_none());

    let permit_debug = format!("{permit:?}");
    assert!(!permit_debug.contains("evidence-sensitive-ref"));
    assert!(!permit_debug.contains("operator-sensitive-ref"));
    assert!(permit_debug.contains("evidence_ref_len"));
    assert!(permit_debug.contains("operator_approval_ref_len"));
    assert!(handle.await.unwrap().is_empty());
}

#[tokio::test]
async fn live_submit_posts_fixture_bodies_and_preserves_receipts() {
    let responses = vec![
        TestResponse::json(
            "200 OK",
            json!({"transactionID": "tx-create-001", "state": "STATE_NEW"}).to_string(),
        ),
        TestResponse::json(
            "200 OK",
            json!({"transactionID": "tx-batch-001", "state": "STATE_NEW"}).to_string(),
        ),
    ];
    let (url, handle) = spawn_server(responses).await;
    let client = mutation_test_client(url, FIXED_NOW_UNIX, true);
    let owner = address(WALLET_OWNER);
    let config = deposit_wallet_contract_config(POLYGON_CHAIN_ID).unwrap();

    let create_request = build_wallet_create_request(owner, config);
    let create_hash = expected_payload_keccak256(&create_request);
    let create_permit = mutation_permit(
        RelayerMutationMode::Live,
        RelayerMutationOperation::WalletCreate,
        owner,
        POLYGON_CHAIN_ID,
        FIXED_PERMIT_EXPIRY_UNIX,
    );
    let create_receipt = expect_submitted(
        client
            .submit_wallet_create(owner, &create_permit)
            .await
            .unwrap(),
    );
    assert_eq!(create_receipt.transaction_id(), "tx-create-001");
    assert_eq!(create_receipt.state(), &RelayerTransactionState::New);
    assert_eq!(create_receipt.payload_keccak256(), create_hash);
    let create_debug = format!("{create_receipt:?}");
    assert!(!create_debug.contains("tx-create-001"));
    assert!(create_debug.contains("sha3:0x"));

    let batch_request = wallet_batch_request();
    let batch_hash = expected_payload_keccak256(&batch_request);
    let batch_permit = mutation_permit(
        RelayerMutationMode::Live,
        RelayerMutationOperation::WalletBatch,
        owner,
        POLYGON_CHAIN_ID,
        FIXED_PERMIT_EXPIRY_UNIX,
    );
    let batch_receipt = expect_submitted(
        client
            .submit_signed_wallet_batch(batch_request, &batch_permit)
            .await
            .unwrap(),
    );
    assert_eq!(batch_receipt.transaction_id(), "tx-batch-001");
    assert_eq!(batch_receipt.state(), &RelayerTransactionState::New);
    assert_eq!(batch_receipt.payload_keccak256(), batch_hash);

    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 2);
    for request in &requests {
        assert_eq!(request.method, "POST");
        assert_eq!(request.path, SUBMIT_PATH);
        assert!(request.header("RELAYER_API_KEY").is_some());
        assert!(request.header("RELAYER_API_KEY_ADDRESS").is_some());
        assert_eq!(request.header("content-type"), Some("application/json"));
    }
    assert_eq!(
        serde_json::from_str::<Value>(&requests[0].body).unwrap(),
        fixture_value("wallet_create_submit_body.json")
    );
    assert_eq!(
        serde_json::from_str::<Value>(&requests[1].body).unwrap(),
        fixture_value("wallet_submit_body.json")
    );
}

#[tokio::test]
async fn submit_response_anomalies_require_reconciliation_without_resubmission() {
    let owner = address(WALLET_OWNER);
    let invalid_responses = vec![
        (
            "missing transaction id",
            json!({"state": "STATE_NEW"}).to_string(),
        ),
        (
            "empty transaction id",
            json!({"transactionID": "", "state": "STATE_NEW"}).to_string(),
        ),
        (
            "blank transaction id",
            json!({"transactionID": "   ", "state": "STATE_NEW"}).to_string(),
        ),
        (
            "control transaction id",
            json!({"transactionID": "tx\ninvalid", "state": "STATE_NEW"}).to_string(),
        ),
        (
            "missing state",
            json!({"transactionID": "tx-missing-state"}).to_string(),
        ),
        ("invalid json", "not-json".to_string()),
    ];

    for (label, body) in invalid_responses {
        let (url, handle, stop_tx) =
            spawn_single_response_and_watch_for_retry(Some(TestResponse::json("200 OK", body)))
                .await;
        let client = mutation_test_client(url, FIXED_NOW_UNIX, true);
        let permit = mutation_permit(
            RelayerMutationMode::Live,
            RelayerMutationOperation::WalletCreate,
            owner,
            POLYGON_CHAIN_ID,
            FIXED_PERMIT_EXPIRY_UNIX,
        );

        let error = client
            .submit_wallet_create(owner, &permit)
            .await
            .unwrap_err();
        assert!(
            error.is_deposit_wallet_reconciliation_required(),
            "{label}: {error}"
        );
        assert!(
            error
                .to_string()
                .contains("submit response did not include a valid transactionID"),
            "{label}: {error}"
        );
        assert_eq!(
            finish_retry_watch(stop_tx, handle).await.len(),
            1,
            "{label}"
        );
    }

    for (status, expected_status) in [
        ("503 Service Unavailable", 503),
        ("429 Too Many Requests", 429),
    ] {
        let (url, handle, stop_tx) =
            spawn_single_response_and_watch_for_retry(Some(TestResponse::json(
                status,
                "response ignored",
            )))
            .await;
        let client = mutation_test_client(url, FIXED_NOW_UNIX, true);
        let permit = mutation_permit(
            RelayerMutationMode::Live,
            RelayerMutationOperation::WalletCreate,
            owner,
            POLYGON_CHAIN_ID,
            FIXED_PERMIT_EXPIRY_UNIX,
        );
        let error = client
            .submit_wallet_create(owner, &permit)
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            RelayerError::Api { status, .. } if status == expected_status
        ));
        assert_eq!(finish_retry_watch(stop_tx, handle).await.len(), 1);
    }
}

#[tokio::test]
async fn submit_transport_and_oversized_success_responses_require_reconciliation() {
    let owner = address(WALLET_OWNER);

    let (url, handle, stop_tx) = spawn_single_response_and_watch_for_retry(None).await;
    let client = mutation_test_client(url, FIXED_NOW_UNIX, true);
    let permit = mutation_permit(
        RelayerMutationMode::Live,
        RelayerMutationOperation::WalletCreate,
        owner,
        POLYGON_CHAIN_ID,
        FIXED_PERMIT_EXPIRY_UNIX,
    );
    let error = client
        .submit_wallet_create(owner, &permit)
        .await
        .unwrap_err();
    assert!(error.is_deposit_wallet_reconciliation_required());
    assert!(!matches!(error, RelayerError::Http(_)));
    assert_eq!(finish_retry_watch(stop_tx, handle).await.len(), 1);

    let content_length_too_large =
        TestResponse::json_without_content_length("200 OK", "").with_header(
            "content-length",
            (MAX_SUCCESS_BODY_BYTES + 1).to_string(),
        );
    let (url, handle, stop_tx) =
        spawn_single_response_and_watch_for_retry(Some(content_length_too_large)).await;
    let client = mutation_test_client(url, FIXED_NOW_UNIX, true);
    let permit = mutation_permit(
        RelayerMutationMode::Live,
        RelayerMutationOperation::WalletCreate,
        owner,
        POLYGON_CHAIN_ID,
        FIXED_PERMIT_EXPIRY_UNIX,
    );
    let error = client
        .submit_wallet_create(owner, &permit)
        .await
        .unwrap_err();
    assert!(error.is_deposit_wallet_reconciliation_required());
    assert_eq!(finish_retry_watch(stop_tx, handle).await.len(), 1);

    let oversized_body = "x".repeat(MAX_SUCCESS_BODY_BYTES + 1);
    let (url, handle, stop_tx) = spawn_single_response_and_watch_for_retry(Some(
        TestResponse::json_without_content_length("200 OK", oversized_body),
    ))
    .await;
    let client = mutation_test_client(url, FIXED_NOW_UNIX, true);
    let permit = mutation_permit(
        RelayerMutationMode::Live,
        RelayerMutationOperation::WalletCreate,
        owner,
        POLYGON_CHAIN_ID,
        FIXED_PERMIT_EXPIRY_UNIX,
    );
    let error = client
        .submit_wallet_create(owner, &permit)
        .await
        .unwrap_err();
    assert!(error.is_deposit_wallet_reconciliation_required());
    assert_eq!(finish_retry_watch(stop_tx, handle).await.len(), 1);
}

#[tokio::test]
async fn submit_preserves_unknown_transaction_state_without_success_classification() {
    let (url, handle) = spawn_server(vec![TestResponse::json(
        "200 OK",
        json!({"transactionID": "tx-future-state", "state": "STATE_FUTURE"}).to_string(),
    )])
    .await;
    let client = mutation_test_client(url, FIXED_NOW_UNIX, true);
    let owner = address(WALLET_OWNER);
    let permit = mutation_permit(
        RelayerMutationMode::Live,
        RelayerMutationOperation::WalletCreate,
        owner,
        POLYGON_CHAIN_ID,
        FIXED_PERMIT_EXPIRY_UNIX,
    );

    let receipt = expect_submitted(
        client
            .submit_wallet_create(owner, &permit)
            .await
            .unwrap(),
    );
    assert_eq!(
        receipt.state(),
        &RelayerTransactionState::Unknown("STATE_FUTURE".to_string())
    );
    assert!(!receipt.state().is_success());
    let receipt_debug = format!("{receipt:?}");
    assert!(!receipt_debug.contains("STATE_FUTURE"));
    assert!(receipt_debug.contains("<unrecognized relayer state>"));
    assert_eq!(handle.await.unwrap().len(), 1);
}

#[tokio::test]
async fn rollback_latch_disables_all_clones_while_reads_and_dry_run_continue() {
    let owner = address(WALLET_OWNER);
    let transaction_id = "tx-rollback-read";
    let responses = vec![
        TestResponse::json(
            "200 OK",
            json!({"transactionID": "tx-before-rollback", "state": "STATE_NEW"}).to_string(),
        ),
        TestResponse::json("200 OK", json!({"deployed": true}).to_string()),
        TestResponse::json("200 OK", json!({"nonce": 31}).to_string()),
        TestResponse::json(
            "200 OK",
            transaction_response_value(transaction_id, "STATE_NEW").to_string(),
        ),
    ];
    let (url, handle) = spawn_server(responses).await;
    let client = DepositWalletRelayerClient::new_with_mutation_enabled(
        url,
        relayer_auth(),
        deposit_wallet_contract_config(POLYGON_CHAIN_ID).unwrap(),
    )
    .unwrap();
    let cloned_client = client.clone();
    let live_permit = mutation_permit(
        RelayerMutationMode::Live,
        RelayerMutationOperation::WalletCreate,
        owner,
        POLYGON_CHAIN_ID,
        u64::MAX,
    );

    let receipt = expect_submitted(
        client
            .submit_wallet_create(owner, &live_permit)
            .await
            .unwrap(),
    );
    assert_eq!(receipt.transaction_id(), "tx-before-rollback");

    client.disable_mutation();
    let same_client_error = client
        .submit_wallet_create(owner, &live_permit)
        .await
        .unwrap_err();
    let clone_error = cloned_client
        .submit_wallet_create(owner, &live_permit)
        .await
        .unwrap_err();
    assert!(same_client_error.is_deposit_wallet_mutation_blocked());
    assert!(clone_error.is_deposit_wallet_mutation_blocked());

    let read_permit = read_permit(owner);
    assert!(client
        .is_deposit_wallet_deployed(owner, &read_permit)
        .await
        .unwrap());
    assert_eq!(
        client.get_wallet_nonce(owner, &read_permit).await.unwrap(),
        U256::from(31u64)
    );
    let transaction = client
        .get_transaction_for_owner(owner, transaction_id, &read_permit)
        .await
        .unwrap();
    assert_eq!(transaction.transaction_id, transaction_id);
    assert_eq!(transaction.state, RelayerTransactionState::New);

    let dry_run_permit = mutation_permit(
        RelayerMutationMode::DryRun,
        RelayerMutationOperation::WalletCreate,
        owner,
        POLYGON_CHAIN_ID,
        u64::MAX,
    );
    let evidence = expect_dry_run(
        client
            .submit_wallet_create(owner, &dry_run_permit)
            .await
            .unwrap(),
    );
    assert_eq!(evidence.operation(), WALLET_CREATE_TRANSACTION_TYPE);

    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 4);
    assert_eq!(requests[0].path, SUBMIT_PATH);
    assert!(requests[1].path.starts_with(DEPLOYED_PATH));
    assert!(requests[2].path.starts_with("/nonce"));
    assert!(requests[3].path.starts_with(TRANSACTION_PATH));
}

#[test]
fn mutation_permit_debug_redacts_owner_and_reference_contents() {
    let owner = address(WALLET_OWNER);
    let owner_checksum = to_checksum(&owner, None);
    let permit = RelayerMutationPermit::try_new(
        RelayerMutationMode::Live,
        RelayerMutationOperation::WalletBatch,
        owner,
        POLYGON_CHAIN_ID,
        FIXED_PERMIT_EXPIRY_UNIX,
        "evidence-reference-secret-looking",
        "operator-approval-secret-looking",
    )
    .unwrap();

    let debug = format!("{permit:?}");
    assert!(debug.contains(&super::redaction::redacted_address(owner)));
    assert!(debug.contains("evidence_ref_len"));
    assert!(debug.contains("operator_approval_ref_len"));
    assert!(!debug.contains(&owner_checksum));
    assert!(!debug.contains(&owner_checksum.to_ascii_lowercase()));
    assert!(!debug.contains("evidence-reference-secret-looking"));
    assert!(!debug.contains("operator-approval-secret-looking"));
}

#[tokio::test]
async fn deployment_lifecycle_short_circuits_when_wallet_is_already_deployed() {
    let (url, handle) = spawn_server(vec![TestResponse::json(
        "200 OK",
        json!({"deployed": true}).to_string(),
    )])
    .await;
    let client = mutation_test_client(url, FIXED_NOW_UNIX, true);
    let owner = address(WALLET_OWNER);
    let read_permit = read_permit(owner);
    let live_permit = mutation_permit(
        RelayerMutationMode::Live,
        RelayerMutationOperation::WalletCreate,
        owner,
        POLYGON_CHAIN_ID,
        FIXED_PERMIT_EXPIRY_UNIX,
    );

    let status = client
        .ensure_deposit_wallet_deployment(
            owner,
            DepositWalletDeploymentPolicy::DeployIfMissing,
            &read_permit,
            Some(&live_permit),
        )
        .await
        .unwrap();

    assert_eq!(status, DepositWalletDeploymentStatus::AlreadyDeployed);
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_deployed_request(&requests[0]);
}

#[tokio::test]
async fn deployment_lifecycle_predeployed_policy_blocks_missing_wallet() {
    let (url, handle) = spawn_server(vec![TestResponse::json(
        "200 OK",
        json!({"deployed": false}).to_string(),
    )])
    .await;
    let client = test_client(url);
    let owner = address(WALLET_OWNER);

    let error = client
        .ensure_deposit_wallet_deployment(
            owner,
            DepositWalletDeploymentPolicy::Predeployed,
            &read_permit(owner),
            None,
        )
        .await
        .unwrap_err();

    assert!(error.is_deposit_wallet_mutation_blocked());
    assert!(error
        .to_string()
        .contains("predeployed policy forbids WALLET-CREATE"));
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_deployed_request(&requests[0]);
}

#[tokio::test]
async fn deployment_lifecycle_requires_explicit_mutation_permit() {
    let (url, handle) = spawn_server(vec![TestResponse::json(
        "200 OK",
        json!({"deployed": false}).to_string(),
    )])
    .await;
    let client = test_client(url);
    let owner = address(WALLET_OWNER);

    let error = client
        .ensure_deposit_wallet_deployment(
            owner,
            DepositWalletDeploymentPolicy::DeployIfMissing,
            &read_permit(owner),
            None,
        )
        .await
        .unwrap_err();

    assert!(error.is_deposit_wallet_mutation_blocked());
    assert!(error
        .to_string()
        .contains("deployment requires an explicit mutation permit"));
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_deployed_request(&requests[0]);
}

#[tokio::test]
async fn deployment_lifecycle_live_create_preserves_fixture_body_and_receipt() {
    let (url, handle) = spawn_server(vec![
        TestResponse::json("200 OK", json!({"deployed": false}).to_string()),
        TestResponse::json(
            "200 OK",
            json!({"transactionID": "tx-lifecycle-create", "state": "STATE_NEW"})
                .to_string(),
        ),
    ])
    .await;
    let client = mutation_test_client(url, FIXED_NOW_UNIX, true);
    let owner = address(WALLET_OWNER);
    let config = deposit_wallet_contract_config(POLYGON_CHAIN_ID).unwrap();
    let expected_hash = expected_payload_keccak256(&build_wallet_create_request(owner, config));
    let live_permit = mutation_permit(
        RelayerMutationMode::Live,
        RelayerMutationOperation::WalletCreate,
        owner,
        POLYGON_CHAIN_ID,
        FIXED_PERMIT_EXPIRY_UNIX,
    );

    let status = client
        .ensure_deposit_wallet_deployment(
            owner,
            DepositWalletDeploymentPolicy::DeployIfMissing,
            &read_permit(owner),
            Some(&live_permit),
        )
        .await
        .unwrap();
    let receipt = match status {
        DepositWalletDeploymentStatus::CreateSubmitted(receipt) => receipt,
        other => panic!("expected create submission, got {other:?}"),
    };

    assert_eq!(receipt.transaction_id(), "tx-lifecycle-create");
    assert_eq!(receipt.state(), &RelayerTransactionState::New);
    assert_eq!(receipt.payload_keccak256(), expected_hash);

    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 2);
    assert_deployed_request(&requests[0]);
    assert_eq!(requests[1].method, "POST");
    assert_eq!(requests[1].path, SUBMIT_PATH);
    let submit_body = serde_json::from_str::<Value>(&requests[1].body).unwrap();
    assert_eq!(submit_body, fixture_value("wallet_create_submit_body.json"));
    assert!(submit_body.get("signature").is_none());
}

#[tokio::test]
async fn deployment_lifecycle_dry_run_preserves_evidence_without_submit_http() {
    let (url, handle) = spawn_server(vec![TestResponse::json(
        "200 OK",
        json!({"deployed": false}).to_string(),
    )])
    .await;
    let client = mutation_test_client(url, FIXED_NOW_UNIX, false);
    let owner = address(WALLET_OWNER);
    let config = deposit_wallet_contract_config(POLYGON_CHAIN_ID).unwrap();
    let expected_wallet = derive_deposit_wallet_address(owner, config).unwrap();
    let expected_hash = expected_payload_keccak256(&build_wallet_create_request(owner, config));
    let dry_run_permit = mutation_permit(
        RelayerMutationMode::DryRun,
        RelayerMutationOperation::WalletCreate,
        owner,
        POLYGON_CHAIN_ID,
        FIXED_PERMIT_EXPIRY_UNIX,
    );

    let status = client
        .ensure_deposit_wallet_deployment(
            owner,
            DepositWalletDeploymentPolicy::DeployIfMissing,
            &read_permit(owner),
            Some(&dry_run_permit),
        )
        .await
        .unwrap();
    let evidence = match status {
        DepositWalletDeploymentStatus::CreateDryRun(evidence) => evidence,
        other => panic!("expected create dry-run, got {other:?}"),
    };

    assert_eq!(evidence.operation(), WALLET_CREATE_TRANSACTION_TYPE);
    assert_eq!(evidence.endpoint_path(), SUBMIT_PATH);
    assert_eq!(evidence.chain_id(), POLYGON_CHAIN_ID);
    assert_eq!(
        evidence.owner(),
        super::redaction::redacted_address(owner)
    );
    assert_eq!(
        evidence.deposit_wallet(),
        super::redaction::redacted_address(expected_wallet)
    );
    assert_eq!(evidence.to(), to_checksum(&config.factory, None));
    assert_eq!(evidence.payload_keccak256(), expected_hash);
    assert_eq!(evidence.nonce(), None);
    assert_eq!(evidence.deadline(), None);
    assert!(evidence.calls().is_empty());

    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_deployed_request(&requests[0]);
}

#[tokio::test]
async fn deployment_lifecycle_propagates_closed_mutation_gate_after_preflight() {
    let (url, handle) = spawn_server(vec![TestResponse::json(
        "200 OK",
        json!({"deployed": false}).to_string(),
    )])
    .await;
    let client = mutation_test_client(url, FIXED_NOW_UNIX, false);
    let owner = address(WALLET_OWNER);
    let live_permit = mutation_permit(
        RelayerMutationMode::Live,
        RelayerMutationOperation::WalletCreate,
        owner,
        POLYGON_CHAIN_ID,
        FIXED_PERMIT_EXPIRY_UNIX,
    );

    let error = client
        .ensure_deposit_wallet_deployment(
            owner,
            DepositWalletDeploymentPolicy::DeployIfMissing,
            &read_permit(owner),
            Some(&live_permit),
        )
        .await
        .unwrap_err();

    assert!(error.is_deposit_wallet_mutation_blocked());
    assert!(error
        .to_string()
        .contains("relayer mutation is disabled for this client"));
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_deployed_request(&requests[0]);
}

#[tokio::test]
async fn deployment_readiness_maps_confirmed_pending_and_error_states() {
    let cases = [
        ("tx-create-confirmed", "STATE_CONFIRMED"),
        ("tx-create-new", "STATE_NEW"),
        ("tx-create-executed", "STATE_EXECUTED"),
        ("tx-create-mined", "STATE_MINED"),
        ("tx-create-failed", "STATE_FAILED"),
        ("tx-create-invalid", "STATE_INVALID"),
        ("tx-create-unknown", "STATE_FUTURE"),
    ];
    let responses = cases
        .iter()
        .map(|(transaction_id, state)| {
            TestResponse::json(
                "200 OK",
                json!([wallet_create_transaction_response_value(
                    transaction_id,
                    state
                )])
                .to_string(),
            )
        })
        .collect();
    let (url, handle) = spawn_server(responses).await;
    let client = test_client(url);
    let owner = address(WALLET_OWNER);
    let permit = read_permit(owner);

    assert_eq!(
        client
            .check_deposit_wallet_deployment_readiness(owner, cases[0].0, &permit)
            .await
            .unwrap(),
        DepositWalletReadiness::Ready
    );
    for ((transaction_id, _), expected_state) in cases[1..4].iter().zip([
        RelayerTransactionState::New,
        RelayerTransactionState::Executed,
        RelayerTransactionState::Mined,
    ]) {
        assert_eq!(
            client
                .check_deposit_wallet_deployment_readiness(owner, transaction_id, &permit)
                .await
                .unwrap(),
            DepositWalletReadiness::Pending(expected_state)
        );
    }

    let failed = client
        .check_deposit_wallet_deployment_readiness(owner, cases[4].0, &permit)
        .await
        .unwrap_err();
    assert!(matches!(failed, RelayerError::TransactionFailed(_)));
    let invalid = client
        .check_deposit_wallet_deployment_readiness(owner, cases[5].0, &permit)
        .await
        .unwrap_err();
    assert!(matches!(invalid, RelayerError::TransactionInvalid(_)));
    let unknown = client
        .check_deposit_wallet_deployment_readiness(owner, cases[6].0, &permit)
        .await
        .unwrap_err();
    assert!(unknown.is_deposit_wallet_reconciliation_required());
    assert!(!unknown.to_string().contains("STATE_FUTURE"));

    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), cases.len());
    for (request, (transaction_id, _)) in requests.iter().zip(cases) {
        assert_eq!(request.method, "GET");
        assert_eq!(request.path, format!("/transaction?id={transaction_id}"));
        assert!(request.body.is_empty());
    }
}

#[tokio::test]
async fn deployment_readiness_keeps_wallet_and_wallet_create_types_isolated() {
    let responses = vec![
        TestResponse::json(
            "200 OK",
            json!([transaction_response_value(
                "tx-wallet-on-create-path",
                "STATE_CONFIRMED"
            )])
            .to_string(),
        ),
        TestResponse::json(
            "200 OK",
            json!([wallet_create_transaction_response_value(
                "tx-create-on-wallet-path",
                "STATE_CONFIRMED"
            )])
            .to_string(),
        ),
    ];
    let (url, handle) = spawn_server(responses).await;
    let client = test_client(url);
    let owner = address(WALLET_OWNER);
    let permit = read_permit(owner);

    let create_path_error = client
        .check_deposit_wallet_deployment_readiness(
            owner,
            "tx-wallet-on-create-path",
            &permit,
        )
        .await
        .unwrap_err();
    assert!(create_path_error.is_deposit_wallet_reconciliation_required());
    assert!(create_path_error
        .to_string()
        .contains("type was not WALLET-CREATE"));

    let wallet_path_error = client
        .get_transaction_for_owner(owner, "tx-create-on-wallet-path", &permit)
        .await
        .unwrap_err();
    assert!(wallet_path_error.is_deposit_wallet_reconciliation_required());
    assert!(wallet_path_error
        .to_string()
        .contains("type was not WALLET"));

    assert_eq!(handle.await.unwrap().len(), 2);
}

#[tokio::test]
async fn deployment_lifecycle_methods_reject_mismatched_read_permits_before_http() {
    let (url, handle) = spawn_optional_request_server().await;
    let client = test_client(url);
    let owner = address(WALLET_OWNER);
    let mismatched_permit = read_permit(address(OTHER_OWNER));

    let ensure_error = client
        .ensure_deposit_wallet_deployment(
            owner,
            DepositWalletDeploymentPolicy::Predeployed,
            &mismatched_permit,
            None,
        )
        .await
        .unwrap_err();
    let readiness_error = client
        .check_deposit_wallet_deployment_readiness(owner, "", &mismatched_permit)
        .await
        .unwrap_err();

    assert!(ensure_error.is_deposit_wallet_read_blocked());
    assert!(readiness_error.is_deposit_wallet_read_blocked());
    assert!(handle.await.unwrap().is_empty());
}

#[tokio::test]
async fn execute_wallet_batch_fetches_fresh_nonce_then_submits_verified_live_body() {
    let signer = execute_signer();
    let owner = signer.address();
    let ctx = execute_context(owner);
    let calls = execute_calls();
    let deadline = U256::from(EXECUTE_DEADLINE_UNIX);
    let expected_request = expected_execute_request(
        &signer,
        ctx.clone(),
        calls.clone(),
        U256::from(31u64),
        deadline,
    )
    .await;
    let expected_hash = expected_payload_keccak256(&expected_request);
    let (url, handle) = spawn_server(vec![
        TestResponse::json("200 OK", json!({"nonce": "31"}).to_string()),
        TestResponse::json(
            "200 OK",
            json!({"transactionID": "tx-batch-1", "state": "STATE_NEW"}).to_string(),
        ),
    ])
    .await;
    let client = mutation_test_client(url, FIXED_NOW_UNIX, true);
    let permit = execute_permit(RelayerMutationMode::Live, owner);
    let read_permit = read_permit(owner);

    assert_ne!(relayer_auth().api_key_address(), owner);
    let receipt = expect_submitted(
        client
            .execute_wallet_batch(
                ctx,
                calls,
                deadline,
                &signer,
                &read_permit,
                &permit,
            )
            .await
            .unwrap(),
    );

    assert_eq!(receipt.transaction_id(), "tx-batch-1");
    assert_eq!(receipt.state(), &RelayerTransactionState::New);
    assert_eq!(receipt.payload_keccak256(), expected_hash);

    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].method, "GET");
    assert_eq!(
        requests[0].path,
        format!(
            "/nonce?address={}&type=WALLET",
            to_checksum(&owner, None)
        )
    );
    assert!(requests[0].body.is_empty());
    assert_execute_auth_headers(&requests[0]);

    assert_eq!(requests[1].method, "POST");
    assert_eq!(requests[1].path, SUBMIT_PATH);
    assert_execute_auth_headers(&requests[1]);
    let submitted_body: Value = serde_json::from_str(&requests[1].body).unwrap();
    assert_eq!(submitted_body, serde_json::to_value(expected_request).unwrap());
    assert_eq!(submitted_body["nonce"], json!("31"));
}

#[tokio::test]
async fn execute_wallet_batch_dry_run_reads_nonce_and_preserves_it_in_evidence() {
    let signer = execute_signer();
    let owner = signer.address();
    let ctx = execute_context(owner);
    let (url, handle) = spawn_server(vec![TestResponse::json(
        "200 OK",
        json!({"nonce": "31"}).to_string(),
    )])
    .await;
    let client = mutation_test_client(url, FIXED_NOW_UNIX, false);
    let permit = execute_permit(RelayerMutationMode::DryRun, owner);

    let evidence = expect_dry_run(
        client
            .execute_wallet_batch(
                ctx,
                execute_calls(),
                U256::from(EXECUTE_DEADLINE_UNIX),
                &signer,
                &read_permit(owner),
                &permit,
            )
            .await
            .unwrap(),
    );

    assert_eq!(evidence.nonce(), Some("31"));
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "GET");
    assert!(requests[0].path.starts_with("/nonce?"));
    assert_execute_auth_headers(&requests[0]);
}

#[tokio::test]
async fn execute_wallet_batch_rejects_signer_identity_before_nonce_read() {
    let signer = execute_signer();
    let owner = address(OTHER_OWNER);
    let ctx = execute_context(owner);
    let (url, handle) = spawn_optional_request_server().await;
    let client = mutation_test_client(url, FIXED_NOW_UNIX, true);
    let permit = execute_permit(RelayerMutationMode::Live, owner);

    let error = client
        .execute_wallet_batch(
            ctx,
            execute_calls(),
            U256::from(EXECUTE_DEADLINE_UNIX),
            &signer,
            &read_permit(owner),
            &permit,
        )
        .await
        .unwrap_err();

    assert!(matches!(error, RelayerError::Signing(_)));
    assert!(error
        .to_string()
        .contains("batch signer address did not match deposit wallet owner"));
    assert!(!error.is_deposit_wallet_mutation_blocked());
    assert!(!error.is_deposit_wallet_read_blocked());
    assert!(!error.is_deposit_wallet_reconciliation_required());
    assert!(!error.is_deposit_wallet_transaction_absent());
    assert!(handle.await.unwrap().is_empty());
}

#[tokio::test]
async fn execute_wallet_batch_prevalidates_mutation_and_read_permits_before_http() {
    let signer = execute_signer();
    let owner = signer.address();
    let ctx = execute_context(owner);
    let (url, handle) = spawn_optional_request_server().await;
    let client = mutation_test_client(url, FIXED_NOW_UNIX, true);
    let wrong_operation = mutation_permit(
        RelayerMutationMode::Live,
        RelayerMutationOperation::WalletCreate,
        owner,
        POLYGON_CHAIN_ID,
        FIXED_PERMIT_EXPIRY_UNIX,
    );
    let valid_mutation = execute_permit(RelayerMutationMode::Live, owner);
    let wrong_chain = mutation_permit(
        RelayerMutationMode::Live,
        RelayerMutationOperation::WalletBatch,
        owner,
        AMOY_CHAIN_ID,
        FIXED_PERMIT_EXPIRY_UNIX,
    );

    let operation_error = client
        .execute_wallet_batch(
            ctx.clone(),
            execute_calls(),
            U256::from(EXECUTE_DEADLINE_UNIX),
            &signer,
            &read_permit(owner),
            &wrong_operation,
        )
        .await
        .unwrap_err();
    assert!(operation_error.is_deposit_wallet_mutation_blocked());

    let read_error = client
        .execute_wallet_batch(
            ctx.clone(),
            execute_calls(),
            U256::from(EXECUTE_DEADLINE_UNIX),
            &signer,
            &read_permit(address(OTHER_OWNER)),
            &valid_mutation,
        )
        .await
        .unwrap_err();
    assert!(read_error.is_deposit_wallet_read_blocked());

    let chain_error = client
        .execute_wallet_batch(
            ctx,
            execute_calls(),
            U256::from(EXECUTE_DEADLINE_UNIX),
            &signer,
            &read_permit(owner),
            &wrong_chain,
        )
        .await
        .unwrap_err();
    assert!(chain_error.is_deposit_wallet_mutation_blocked());

    assert!(handle.await.unwrap().is_empty());
}

#[tokio::test]
async fn execute_wallet_batch_rejects_expired_deadline_before_nonce_read() {
    let signer = execute_signer();
    let owner = signer.address();
    let (url, handle) = spawn_optional_request_server().await;
    let client = mutation_test_client(url, FIXED_NOW_UNIX, true);
    let permit = execute_permit(RelayerMutationMode::Live, owner);

    let error = client
        .execute_wallet_batch(
            execute_context(owner),
            execute_calls(),
            U256::from(FIXED_NOW_UNIX),
            &signer,
            &read_permit(owner),
            &permit,
        )
        .await
        .unwrap_err();

    assert!(error.is_deposit_wallet_mutation_blocked());
    assert!(error
        .to_string()
        .contains("batch deadline expired before signing"));
    assert!(handle.await.unwrap().is_empty());
}

#[tokio::test]
async fn execute_wallet_batch_rejects_wrong_wallet_before_nonce_read() {
    let signer = execute_signer();
    let owner = signer.address();
    let mut ctx = execute_context(owner);
    ctx.deposit_wallet_address = address("0x0000000000000000000000000000000000000001");
    assert_ne!(ctx.deposit_wallet_address, execute_context(owner).deposit_wallet_address);
    let (url, handle) = spawn_optional_request_server().await;
    let client = mutation_test_client(url, FIXED_NOW_UNIX, true);
    let permit = execute_permit(RelayerMutationMode::Live, owner);

    let error = client
        .execute_wallet_batch(
            ctx,
            execute_calls(),
            U256::from(EXECUTE_DEADLINE_UNIX),
            &signer,
            &read_permit(owner),
            &permit,
        )
        .await
        .unwrap_err();

    assert!(matches!(error, RelayerError::Signing(_)));
    assert!(error.to_string().contains(
        "deposit wallet request context wallet does not match owner/config derived wallet"
    ));
    assert!(handle.await.unwrap().is_empty());
}

#[tokio::test]
async fn execute_wallet_batch_rejects_oversized_batch_before_nonce_read() {
    let signer = execute_signer();
    let owner = signer.address();
    let oversized_calls = vec![execute_calls()[0].clone(); 257];
    let (url, handle) = spawn_optional_request_server().await;
    let client = mutation_test_client(url, FIXED_NOW_UNIX, true);
    let permit = execute_permit(RelayerMutationMode::Live, owner);

    let error = client
        .execute_wallet_batch(
            execute_context(owner),
            oversized_calls,
            U256::from(EXECUTE_DEADLINE_UNIX),
            &signer,
            &read_permit(owner),
            &permit,
        )
        .await
        .unwrap_err();

    assert!(matches!(error, RelayerError::Signing(_)));
    assert!(error.to_string().contains("call count exceeds maximum"));
    assert!(handle.await.unwrap().is_empty());
}

#[tokio::test]
async fn execute_wallet_batch_preserves_submit_api_error_without_duplicate_post() {
    let signer = execute_signer();
    let owner = signer.address();
    let (url, handle) = spawn_server(vec![
        TestResponse::json("200 OK", json!({"nonce": "31"}).to_string()),
        TestResponse::json("503 Service Unavailable", "{}"),
    ])
    .await;
    let client = mutation_test_client(url, FIXED_NOW_UNIX, true);
    let permit = execute_permit(RelayerMutationMode::Live, owner);

    let error = client
        .execute_wallet_batch(
            execute_context(owner),
            execute_calls(),
            U256::from(EXECUTE_DEADLINE_UNIX),
            &signer,
            &read_permit(owner),
            &permit,
        )
        .await
        .unwrap_err();

    assert!(matches!(error, RelayerError::Api { status: 503, .. }));
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].method, "GET");
    assert_eq!(requests[1].method, "POST");
}

#[tokio::test]
async fn execute_wallet_batch_classifies_submit_disconnect_for_reconciliation() {
    let signer = execute_signer();
    let owner = signer.address();
    let (url, handle) = spawn_nonce_then_reset_server(TestResponse::json(
        "200 OK",
        json!({"nonce": "31"}).to_string(),
    ))
    .await;
    let client = mutation_test_client(url, FIXED_NOW_UNIX, true);
    let permit = execute_permit(RelayerMutationMode::Live, owner);

    let error = client
        .execute_wallet_batch(
            execute_context(owner),
            execute_calls(),
            U256::from(EXECUTE_DEADLINE_UNIX),
            &signer,
            &read_permit(owner),
            &permit,
        )
        .await
        .unwrap_err();

    assert!(error.is_deposit_wallet_reconciliation_required());
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].method, "GET");
    assert_eq!(requests[1].method, "POST");
}

#[tokio::test]
async fn execute_wallet_batch_closed_latch_allows_nonce_read_but_blocks_post() {
    let signer = execute_signer();
    let owner = signer.address();
    let (url, handle) = spawn_server(vec![TestResponse::json(
        "200 OK",
        json!({"nonce": "31"}).to_string(),
    )])
    .await;
    let client = mutation_test_client(url, FIXED_NOW_UNIX, false);
    let permit = execute_permit(RelayerMutationMode::Live, owner);

    let error = client
        .execute_wallet_batch(
            execute_context(owner),
            execute_calls(),
            U256::from(EXECUTE_DEADLINE_UNIX),
            &signer,
            &read_permit(owner),
            &permit,
        )
        .await
        .unwrap_err();

    assert!(error.is_deposit_wallet_mutation_blocked());
    assert!(error.to_string().contains("relayer mutation is disabled"));
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "GET");
    assert!(requests[0].path.starts_with("/nonce?"));
}

#[tokio::test]
async fn execute_wallet_batch_discards_signer_error_and_source_material() {
    let owner = execute_signer().address();
    let signer = FailingSigner::new(owner);
    let synthetic_backend_error = FailingSignerError::synthetic();
    assert!(synthetic_backend_error
        .to_string()
        .contains(SIGNER_ERROR_SENTINEL));
    assert!(std::error::Error::source(&synthetic_backend_error)
        .unwrap()
        .to_string()
        .contains(SIGNER_ERROR_SENTINEL));
    let (url, handle) = spawn_server(vec![TestResponse::json(
        "200 OK",
        json!({"nonce": "31"}).to_string(),
    )])
    .await;
    let client = mutation_test_client(url, FIXED_NOW_UNIX, true);
    let permit = execute_permit(RelayerMutationMode::Live, owner);

    let error = client
        .execute_wallet_batch(
            execute_context(owner),
            execute_calls(),
            U256::from(EXECUTE_DEADLINE_UNIX),
            &signer,
            &read_permit(owner),
            &permit,
        )
        .await
        .unwrap_err();

    assert!(matches!(error, RelayerError::Signing(_)));
    assert_eq!(error.to_string(), "Signing error: batch signing failed");
    assert!(!format!("{error}").contains(SIGNER_ERROR_SENTINEL));
    assert!(!format!("{error:?}").contains(SIGNER_ERROR_SENTINEL));
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "GET");
}

#[tokio::test(start_paused = true)]
async fn poll_wallet_transaction_returns_confirmed_without_delay() {
    let transaction_id = "tx-poll-confirmed";
    let (url, handle) = spawn_polling_server(vec![TestResponse::json(
        "200 OK",
        transaction_response_value(transaction_id, "STATE_CONFIRMED").to_string(),
    )])
    .await;
    let client = polling_test_client(url);
    let owner = address(WALLET_OWNER);
    let policy =
        RelayerPollPolicy::try_new(3, Duration::from_secs(1), Duration::from_secs(60)).unwrap();
    let started = tokio::time::Instant::now();

    let outcome = client
        .poll_wallet_transaction(
            owner,
            transaction_id,
            policy,
            &read_permit(owner),
            std::future::pending::<()>(),
        )
        .await
        .unwrap();

    let requests = handle.await.unwrap();
    let RelayerPollOutcome::Confirmed(receipt) = outcome else {
        panic!(
            "expected confirmed polling outcome, got {outcome:?} after {:?}",
            tokio::time::Instant::now() - started
        );
    };
    assert_eq!(receipt.transaction_id, transaction_id);
    assert_eq!(receipt.state, RelayerTransactionState::Confirmed);
    assert_eq!(tokio::time::Instant::now() - started, Duration::ZERO);
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "GET");
    assert_eq!(requests[0].path, format!("/transaction?id={transaction_id}"));
}

#[tokio::test(start_paused = true)]
async fn poll_deposit_wallet_deployment_returns_confirmed_without_delay() {
    let transaction_id = "tx-poll-deployment-confirmed";
    let (url, handle) = spawn_polling_server(vec![TestResponse::json(
        "200 OK",
        wallet_create_transaction_response_value(transaction_id, "STATE_CONFIRMED").to_string(),
    )])
    .await;
    let client = polling_test_client(url);
    let owner = address(WALLET_OWNER);
    let policy =
        RelayerPollPolicy::try_new(3, Duration::from_secs(1), Duration::from_secs(60)).unwrap();
    let started = tokio::time::Instant::now();

    let outcome = client
        .poll_deposit_wallet_deployment(
            owner,
            transaction_id,
            policy,
            &read_permit(owner),
            std::future::pending::<()>(),
        )
        .await
        .unwrap();

    let RelayerPollOutcome::Confirmed(receipt) = outcome else {
        panic!("expected confirmed deployment polling outcome, got {outcome:?}");
    };
    assert_eq!(receipt.transaction_id, transaction_id);
    assert_eq!(receipt.state, RelayerTransactionState::Confirmed);
    assert_eq!(tokio::time::Instant::now() - started, Duration::ZERO);
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "GET");
    assert_eq!(requests[0].path, format!("/transaction?id={transaction_id}"));
}

#[tokio::test(start_paused = true)]
async fn poll_wallet_transaction_applies_exact_exponential_intervals() {
    let transaction_id = "tx-poll-progress";
    let responses = ["STATE_NEW", "STATE_EXECUTED", "STATE_CONFIRMED"]
        .into_iter()
        .map(|state| {
            TestResponse::json(
                "200 OK",
                transaction_response_value(transaction_id, state).to_string(),
            )
        })
        .collect();
    let (url, handle) = spawn_polling_server(responses).await;
    let client = polling_test_client(url);
    let owner = address(WALLET_OWNER);
    let policy =
        RelayerPollPolicy::try_new(3, Duration::from_secs(1), Duration::from_secs(60)).unwrap();
    let started = tokio::time::Instant::now();

    let outcome = client
        .poll_wallet_transaction(
            owner,
            transaction_id,
            policy,
            &read_permit(owner),
            std::future::pending::<()>(),
        )
        .await
        .unwrap();

    assert!(matches!(outcome, RelayerPollOutcome::Confirmed(_)));
    assert_eq!(
        tokio::time::Instant::now() - started,
        Duration::from_secs(3)
    );
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 3);
    assert!(requests
        .iter()
        .all(|request| request.path == format!("/transaction?id={transaction_id}")));
}

#[tokio::test(start_paused = true)]
async fn poll_wallet_transaction_exhaustion_preserves_last_pending_state() {
    let transaction_id = "tx-poll-exhausted";
    let responses = (0..3)
        .map(|_| {
            TestResponse::json(
                "200 OK",
                transaction_response_value(transaction_id, "STATE_NEW").to_string(),
            )
        })
        .collect();
    let (url, handle) = spawn_polling_server(responses).await;
    let client = polling_test_client(url);
    let owner = address(WALLET_OWNER);
    let policy =
        RelayerPollPolicy::try_new(3, Duration::from_secs(1), Duration::from_secs(60)).unwrap();
    let started = tokio::time::Instant::now();

    let outcome = client
        .poll_wallet_transaction(
            owner,
            transaction_id,
            policy,
            &read_permit(owner),
            std::future::pending::<()>(),
        )
        .await
        .unwrap();

    assert_eq!(
        outcome,
        RelayerPollOutcome::Exhausted {
            attempts: 3,
            last_state: Some(RelayerTransactionState::New),
        }
    );
    assert_eq!(
        tokio::time::Instant::now() - started,
        Duration::from_secs(3)
    );
    assert_eq!(handle.await.unwrap().len(), 3);
}

#[tokio::test(start_paused = true)]
async fn poll_wallet_transaction_caps_exponential_backoff() {
    let transaction_id = "tx-poll-backoff-cap";
    let responses = (0..4)
        .map(|_| {
            TestResponse::json(
                "200 OK",
                transaction_response_value(transaction_id, "STATE_NEW").to_string(),
            )
        })
        .collect();
    let (url, handle) = spawn_polling_server(responses).await;
    let client = polling_test_client(url);
    let owner = address(WALLET_OWNER);
    let policy =
        RelayerPollPolicy::try_new(4, Duration::from_secs(1), Duration::from_secs(2)).unwrap();
    let started = tokio::time::Instant::now();

    let outcome = client
        .poll_wallet_transaction(
            owner,
            transaction_id,
            policy,
            &read_permit(owner),
            std::future::pending::<()>(),
        )
        .await
        .unwrap();

    assert_eq!(
        outcome,
        RelayerPollOutcome::Exhausted {
            attempts: 4,
            last_state: Some(RelayerTransactionState::New),
        }
    );
    assert_eq!(
        tokio::time::Instant::now() - started,
        Duration::from_secs(5)
    );
    assert_eq!(handle.await.unwrap().len(), 4);
}

#[tokio::test(start_paused = true)]
async fn poll_wallet_transaction_stops_on_failed_invalid_and_unknown_states() {
    for (transaction_id, state) in [
        ("tx-poll-failed", "STATE_FAILED"),
        ("tx-poll-invalid", "STATE_INVALID"),
        ("tx-poll-unknown", "STATE_FUTURE"),
    ] {
        let (url, handle) = spawn_polling_server(vec![TestResponse::json(
            "200 OK",
            transaction_response_value(transaction_id, state).to_string(),
        )])
        .await;
        let client = polling_test_client(url);
        let owner = address(WALLET_OWNER);
        let policy =
            RelayerPollPolicy::try_new(3, Duration::from_secs(1), Duration::from_secs(60)).unwrap();
        let started = tokio::time::Instant::now();

        let error = client
            .poll_wallet_transaction(
                owner,
                transaction_id,
                policy,
                &read_permit(owner),
                std::future::pending::<()>(),
            )
            .await
            .unwrap_err();

        match state {
            "STATE_FAILED" => assert!(matches!(error, RelayerError::TransactionFailed(_))),
            "STATE_INVALID" => assert!(matches!(error, RelayerError::TransactionInvalid(_))),
            "STATE_FUTURE" => {
                assert!(error.is_deposit_wallet_reconciliation_required());
            }
            _ => unreachable!("test enumerates every state"),
        }
        assert_eq!(tokio::time::Instant::now() - started, Duration::ZERO);
        assert_eq!(handle.await.unwrap().len(), 1);
    }
}

#[tokio::test(start_paused = true)]
async fn poll_wallet_transaction_retries_api_errors_on_policy_schedule() {
    let transaction_id = "tx-poll-transient-api";
    let responses = vec![
        TestResponse::json("429 Too Many Requests", "{}")
            .with_header("Retry-After", "60"),
        TestResponse::json("500 Internal Server Error", "{}"),
        TestResponse::json(
            "200 OK",
            transaction_response_value(transaction_id, "STATE_CONFIRMED").to_string(),
        ),
    ];
    let (url, handle) = spawn_polling_server(responses).await;
    let client = polling_test_client(url);
    let owner = address(WALLET_OWNER);
    let policy =
        RelayerPollPolicy::try_new(3, Duration::from_secs(1), Duration::from_secs(60)).unwrap();
    let started = tokio::time::Instant::now();

    let outcome = client
        .poll_wallet_transaction(
            owner,
            transaction_id,
            policy,
            &read_permit(owner),
            std::future::pending::<()>(),
        )
        .await
        .unwrap();

    assert!(matches!(outcome, RelayerPollOutcome::Confirmed(_)));
    assert_eq!(
        tokio::time::Instant::now() - started,
        Duration::from_secs(3),
        "Retry-After is intentionally not used as the polling interval"
    );
    assert_eq!(handle.await.unwrap().len(), 3);
}

#[tokio::test(start_paused = true)]
async fn poll_wallet_transaction_retries_transport_error_on_policy_schedule() {
    let transaction_id = "tx-poll-transient-transport";
    let (url, handle) = spawn_polling_reset_then_response_server(TestResponse::json(
        "200 OK",
        transaction_response_value(transaction_id, "STATE_CONFIRMED").to_string(),
    ))
    .await;
    let client = polling_test_client(url);
    let owner = address(WALLET_OWNER);
    let policy =
        RelayerPollPolicy::try_new(2, Duration::from_secs(1), Duration::from_secs(60)).unwrap();
    let started = tokio::time::Instant::now();

    let outcome = client
        .poll_wallet_transaction(
            owner,
            transaction_id,
            policy,
            &read_permit(owner),
            std::future::pending::<()>(),
        )
        .await
        .unwrap();

    assert!(matches!(outcome, RelayerPollOutcome::Confirmed(_)));
    assert_eq!(
        tokio::time::Instant::now() - started,
        Duration::from_secs(1)
    );
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests
        .iter()
        .all(|request| request.path == format!("/transaction?id={transaction_id}")));
}

#[tokio::test(start_paused = true)]
async fn poll_wallet_transaction_cancels_in_flight_read_without_retry() {
    let transaction_id = "tx-poll-cancel-wait";
    let (url, handle, request_seen, release_server) =
        spawn_polling_held_response_server().await;
    let client = polling_test_client(url);
    let owner = address(WALLET_OWNER);
    let policy =
        RelayerPollPolicy::try_new(3, Duration::from_secs(1), Duration::from_secs(60)).unwrap();
    let cancel = async move {
        request_seen
            .await
            .expect("polling server should observe the first request");
        tokio::time::sleep(Duration::from_millis(500)).await;
    };
    let outcome = client
        .poll_wallet_transaction(
            owner,
            transaction_id,
            policy,
            &read_permit(owner),
            cancel,
        )
        .await
        .unwrap();

    release_server
        .send(())
        .expect("polling held-response server should remain available");
    assert_eq!(outcome, RelayerPollOutcome::Cancelled { attempts: 0 });
    assert_eq!(handle.await.unwrap().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn poll_wallet_transaction_cancels_during_backoff_without_another_read() {
    let transaction_id = "tx-poll-cancel-backoff";
    let response = TestResponse::json(
        "200 OK",
        transaction_response_value(transaction_id, "STATE_NEW").to_string(),
    );
    let (url, handle, armed_rx, stop_server) =
        spawn_polling_response_then_watch_for_retry(response).await;
    let client = polling_test_client(url);
    let owner = address(WALLET_OWNER);
    let policy =
        RelayerPollPolicy::try_new(3, Duration::from_secs(1), Duration::from_secs(60)).unwrap();
    let cancel = async move {
        let _ = armed_rx.await.ok();
        tokio::time::sleep(Duration::from_millis(500)).await;
    };

    let result = client
        .poll_wallet_transaction(
            owner,
            transaction_id,
            policy,
            &read_permit(owner),
            cancel,
        )
        .await;

    stop_server
        .send(())
        .expect("polling retry watcher should remain available");
    let requests = handle.await.unwrap();
    let outcome = result.unwrap();
    assert_eq!(outcome, RelayerPollOutcome::Cancelled { attempts: 1 });
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "GET");
    assert_eq!(requests[0].path, format!("/transaction?id={transaction_id}"));
}

#[tokio::test(start_paused = true)]
async fn poll_wallet_transaction_prioritizes_immediate_cancellation_before_http() {
    let (url, handle) = spawn_polling_server(Vec::new()).await;
    let client = polling_test_client(url);
    let owner = address(WALLET_OWNER);
    let policy =
        RelayerPollPolicy::try_new(3, Duration::from_secs(1), Duration::from_secs(60)).unwrap();
    let started = tokio::time::Instant::now();

    let outcome = client
        .poll_wallet_transaction(
            owner,
            "tx-poll-cancel-immediate",
            policy,
            &read_permit(owner),
            std::future::ready(()),
        )
        .await
        .unwrap();

    assert_eq!(outcome, RelayerPollOutcome::Cancelled { attempts: 0 });
    assert_eq!(tokio::time::Instant::now() - started, Duration::ZERO);
    assert!(handle.await.unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn polling_keeps_wallet_and_wallet_create_transaction_types_isolated() {
    let owner = address(WALLET_OWNER);
    let policy =
        RelayerPollPolicy::try_new(3, Duration::from_secs(1), Duration::from_secs(60)).unwrap();

    let wallet_transaction_id = "tx-poll-wallet-type";
    let (wallet_url, wallet_handle) = spawn_polling_server(vec![TestResponse::json(
        "200 OK",
        wallet_create_transaction_response_value(wallet_transaction_id, "STATE_CONFIRMED")
            .to_string(),
    )])
    .await;
    let wallet_client = polling_test_client(wallet_url);
    let wallet_error = wallet_client
        .poll_wallet_transaction(
            owner,
            wallet_transaction_id,
            policy,
            &read_permit(owner),
            std::future::pending::<()>(),
        )
        .await
        .unwrap_err();
    assert!(wallet_error.is_deposit_wallet_reconciliation_required());
    assert_eq!(wallet_handle.await.unwrap().len(), 1);

    let deployment_transaction_id = "tx-poll-deployment-type";
    let (deployment_url, deployment_handle) = spawn_polling_server(vec![TestResponse::json(
        "200 OK",
        transaction_response_value(deployment_transaction_id, "STATE_CONFIRMED").to_string(),
    )])
    .await;
    let deployment_client = polling_test_client(deployment_url);
    let deployment_error = deployment_client
        .poll_deposit_wallet_deployment(
            owner,
            deployment_transaction_id,
            policy,
            &read_permit(owner),
            std::future::pending::<()>(),
        )
        .await
        .unwrap_err();
    assert!(deployment_error.is_deposit_wallet_reconciliation_required());
    assert_eq!(deployment_handle.await.unwrap().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn poll_wallet_transaction_treats_missing_array_item_as_transient() {
    let transaction_id = "tx-poll-after-absence";
    let responses = vec![
        TestResponse::json(
            "200 OK",
            json!([transaction_response_value(
                "tx-poll-unrelated",
                "STATE_CONFIRMED"
            )])
            .to_string(),
        ),
        TestResponse::json(
            "200 OK",
            transaction_response_value(transaction_id, "STATE_CONFIRMED").to_string(),
        ),
    ];
    let (url, handle) = spawn_polling_server(responses).await;
    let client = polling_test_client(url);
    let owner = address(WALLET_OWNER);
    let policy =
        RelayerPollPolicy::try_new(3, Duration::from_secs(1), Duration::from_secs(60)).unwrap();
    let started = tokio::time::Instant::now();

    let outcome = client
        .poll_wallet_transaction(
            owner,
            transaction_id,
            policy,
            &read_permit(owner),
            std::future::pending::<()>(),
        )
        .await
        .unwrap();

    assert!(matches!(outcome, RelayerPollOutcome::Confirmed(_)));
    assert_eq!(
        tokio::time::Instant::now() - started,
        Duration::from_secs(1)
    );
    assert_eq!(handle.await.unwrap().len(), 2);
}

#[tokio::test(start_paused = true)]
async fn poll_wallet_transaction_rejects_mismatched_permit_before_http() {
    let (url, handle) = spawn_polling_server(Vec::new()).await;
    let client = polling_test_client(url);
    let owner = address(WALLET_OWNER);
    let policy =
        RelayerPollPolicy::try_new(3, Duration::from_secs(1), Duration::from_secs(60)).unwrap();

    let error = client
        .poll_wallet_transaction(
            owner,
            "tx-poll-permit",
            policy,
            &read_permit(address(OTHER_OWNER)),
            std::future::pending::<()>(),
        )
        .await
        .unwrap_err();

    assert!(error.is_deposit_wallet_read_blocked());
    assert!(handle.await.unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn relayer_poll_policy_validates_bounds_and_exposes_values() {
    let policy = RelayerPollPolicy::try_new(
        100,
        Duration::from_millis(1),
        Duration::from_secs(600),
    )
    .unwrap();
    assert_eq!(policy.max_attempts(), 100);
    assert_eq!(policy.initial_interval(), Duration::from_millis(1));
    assert_eq!(policy.max_interval(), Duration::from_secs(600));

    for invalid in [
        RelayerPollPolicy::try_new(0, Duration::from_secs(1), Duration::from_secs(60)),
        RelayerPollPolicy::try_new(101, Duration::from_secs(1), Duration::from_secs(60)),
        RelayerPollPolicy::try_new(1, Duration::ZERO, Duration::from_secs(60)),
        RelayerPollPolicy::try_new(1, Duration::from_secs(2), Duration::from_secs(1)),
        RelayerPollPolicy::try_new(1, Duration::from_secs(1), Duration::from_secs(601)),
    ] {
        let error = invalid.unwrap_err();
        assert!(
            matches!(error, RelayerError::Other(ref message) if message.starts_with("invalid poll policy: ")),
            "{error}"
        );
    }
}
