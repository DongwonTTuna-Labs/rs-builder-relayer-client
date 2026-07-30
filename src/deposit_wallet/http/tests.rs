use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering as AtomicOrdering};
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
const STALE_INTENT_LEASE_ERROR: &str =
    "stale mutation intent lease; a newer write superseded this lease";

/// Synthetic throwaway key, never a real credential.
const SYNTHETIC_EXECUTE_SIGNER_KEY: [u8; 32] = [0x42u8; 32];

struct FixedClock {
    now_unix: u64,
}

struct AdvancingClock {
    now_unix: AtomicU64,
}

impl RelayerClock for AdvancingClock {
    fn now_unix(&self) -> u64 {
        self.now_unix.fetch_add(1, AtomicOrdering::SeqCst)
    }
}

#[derive(Default)]
struct CountingMutationIntentStore {
    inner: InMemoryMutationIntentStore,
    try_begin_calls: AtomicUsize,
    load_calls: AtomicUsize,
}

impl MutationIntentStore for CountingMutationIntentStore {
    fn load(&self, owner: Address, chain_id: u64) -> Result<Option<MutationIntentRecord>> {
        self.load_calls.fetch_add(1, AtomicOrdering::SeqCst);
        self.inner.load(owner, chain_id)
    }

    fn try_begin(&self, template: MutationIntentRecord) -> Result<TryBeginOutcome> {
        self.try_begin_calls.fetch_add(1, AtomicOrdering::SeqCst);
        self.inner.try_begin(template)
    }

    fn update(
        &self,
        expected_epoch: u64,
        expected_revision: u64,
        record: MutationIntentRecord,
    ) -> Result<bool> {
        self.inner
            .update(expected_epoch, expected_revision, record)
    }
}

struct FailingBeginMutationIntentStore;

impl MutationIntentStore for FailingBeginMutationIntentStore {
    fn load(&self, _owner: Address, _chain_id: u64) -> Result<Option<MutationIntentRecord>> {
        Ok(None)
    }

    fn try_begin(&self, _template: MutationIntentRecord) -> Result<TryBeginOutcome> {
        Err(RelayerError::Other(
            "synthetic mutation intent begin failure".to_string(),
        ))
    }

    fn update(
        &self,
        _expected_epoch: u64,
        _expected_revision: u64,
        _record: MutationIntentRecord,
    ) -> Result<bool> {
        panic!("update must not run after a failed begin")
    }
}

struct FailingSecondUpdateMutationIntentStore {
    inner: InMemoryMutationIntentStore,
}

impl MutationIntentStore for FailingSecondUpdateMutationIntentStore {
    fn load(&self, owner: Address, chain_id: u64) -> Result<Option<MutationIntentRecord>> {
        self.inner.load(owner, chain_id)
    }

    fn try_begin(&self, template: MutationIntentRecord) -> Result<TryBeginOutcome> {
        self.inner.try_begin(template)
    }

    fn update(
        &self,
        expected_epoch: u64,
        expected_revision: u64,
        record: MutationIntentRecord,
    ) -> Result<bool> {
        if expected_revision == 1 {
            return Err(RelayerError::Other(
                "synthetic mutation intent update failure".to_string(),
            ));
        }
        self.inner
            .update(expected_epoch, expected_revision, record)
    }
}

#[derive(Default)]
struct ReconciliationCasMutationIntentStore {
    inner: InMemoryMutationIntentStore,
    false_updates_remaining: AtomicUsize,
}

impl ReconciliationCasMutationIntentStore {
    fn fail_next_updates(&self, count: usize) {
        self.false_updates_remaining
            .store(count, AtomicOrdering::SeqCst);
    }
}

impl MutationIntentStore for ReconciliationCasMutationIntentStore {
    fn load(&self, owner: Address, chain_id: u64) -> Result<Option<MutationIntentRecord>> {
        self.inner.load(owner, chain_id)
    }

    fn try_begin(&self, template: MutationIntentRecord) -> Result<TryBeginOutcome> {
        self.inner.try_begin(template)
    }

    fn update(
        &self,
        expected_epoch: u64,
        expected_revision: u64,
        record: MutationIntentRecord,
    ) -> Result<bool> {
        if self
            .false_updates_remaining
            .fetch_update(
                AtomicOrdering::SeqCst,
                AtomicOrdering::SeqCst,
                |remaining| remaining.checked_sub(1),
            )
            .is_ok()
        {
            return Ok(false);
        }
        self.inner
            .update(expected_epoch, expected_revision, record)
    }
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

fn intent_registry(store: Arc<dyn MutationIntentStore>) -> OwnerMutationRegistry {
    OwnerMutationRegistry::with_clock(
        store,
        Arc::new(FixedClock {
            now_unix: FIXED_NOW_UNIX,
        }),
    )
}

fn expect_intent_begin_error(
    result: Result<MutationIntentLease<'_>>,
) -> RelayerError {
    match result {
        Ok(_) => panic!("mutation intent begin unexpectedly succeeded"),
        Err(error) => error,
    }
}

fn intent_receipt(
    transaction_id: &str,
    state: RelayerTransactionState,
) -> DepositWalletTransactionReceipt {
    DepositWalletTransactionReceipt {
        transaction_id: transaction_id.to_string(),
        state,
        transaction_hash: None,
        owner: None,
        deposit_wallet: None,
    }
}

fn serialized_intent_record(
    owner: Address,
    chain_id: u64,
    epoch: u64,
    revision: u64,
    status: MutationIntentStatus,
) -> MutationIntentRecord {
    serde_json::from_value(json!({
        "owner": owner,
        "chain_id": chain_id,
        "epoch": epoch,
        "revision": revision,
        "operation": "WalletBatch",
        "status": status,
        "nonce": null,
        "payload_keccak256": null,
        "deadline_unix": null,
        "transaction_id": null,
        "last_observed_state": null,
        "created_at_unix": FIXED_NOW_UNIX,
        "updated_at_unix": FIXED_NOW_UNIX,
    }))
    .expect("serialized mutation intent should deserialize")
}

fn test_payload_keccak256() -> String {
    format!("0x{}", "11".repeat(32))
}

fn test_reconciliation_evidence(
    decision: ReconciliationDecision,
) -> ReconciliationEvidence {
    ReconciliationEvidence::try_new(
        "operator/ticket-59",
        decision,
        "reviewed against venue and on-chain evidence",
    )
    .unwrap()
}

fn prepare_submitted_intent(
    registry: &OwnerMutationRegistry,
    owner: Address,
    operation: RelayerMutationOperation,
    transaction_id: &str,
) {
    let mut lease = registry
        .begin_intent(owner, POLYGON_CHAIN_ID, operation)
        .unwrap();
    lease.record_payload(&test_payload_keccak256(), None).unwrap();
    lease.record_submitted(transaction_id).unwrap();
}

fn prepare_ambiguous_intent(
    registry: &OwnerMutationRegistry,
    owner: Address,
    operation: RelayerMutationOperation,
) {
    let mut lease = registry
        .begin_intent(owner, POLYGON_CHAIN_ID, operation)
        .unwrap();
    lease.record_payload(&test_payload_keccak256(), None).unwrap();
    lease.record_ambiguous_without_id().unwrap();
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

#[test]
fn reconciliation_evidence_validates_redacts_and_round_trips_with_legacy_records() {
    let evidence = ReconciliationEvidence::try_new(
        "  operator/ticket-59  ",
        ReconciliationDecision::ConfirmedOnChain,
        "  checked venue receipt\nand on-chain state  ",
    )
    .unwrap();
    assert_eq!(evidence.operator_ref(), "operator/ticket-59");
    assert_eq!(
        evidence.decision(),
        ReconciliationDecision::ConfirmedOnChain
    );
    assert_eq!(
        evidence.summary(),
        "checked venue receipt\nand on-chain state"
    );
    assert_eq!(evidence.recorded_at_unix(), 0);

    for operator_ref in ["", "   ", "operator\nref", "operator\tref", "operator\0ref"] {
        assert!(ReconciliationEvidence::try_new(
            operator_ref,
            ReconciliationDecision::NotAccepted,
            "valid summary"
        )
        .is_err());
    }
    assert!(ReconciliationEvidence::try_new(
        "r".repeat(256),
        ReconciliationDecision::NotAccepted,
        "valid summary"
    )
    .is_ok());
    assert!(ReconciliationEvidence::try_new(
        "r".repeat(257),
        ReconciliationDecision::NotAccepted,
        "valid summary"
    )
    .is_err());
    for summary in ["", "   ", "bad\tsummary", "bad\rsummary", "bad\0summary"] {
        assert!(ReconciliationEvidence::try_new(
            "operator/ref",
            ReconciliationDecision::Superseded,
            summary
        )
        .is_err());
    }
    assert!(ReconciliationEvidence::try_new(
        "operator/ref",
        ReconciliationDecision::Superseded,
        "s".repeat(1024)
    )
    .is_ok());
    assert!(ReconciliationEvidence::try_new(
        "operator/ref",
        ReconciliationDecision::Superseded,
        "s".repeat(1025)
    )
    .is_err());

    let debug = format!("{evidence:?}");
    assert!(!debug.contains(evidence.operator_ref()));
    assert!(!debug.contains(evidence.summary()));
    assert!(debug.contains("operator_ref_len"));
    assert!(debug.contains("summary_len"));
    assert!(debug.contains("ConfirmedOnChain"));

    let serialized = serde_json::to_string(&evidence).unwrap();
    let round_trip: ReconciliationEvidence = serde_json::from_str(&serialized).unwrap();
    assert_eq!(round_trip, evidence);
    assert!(serialized.contains("operator/ticket-59"));

    let legacy = serialized_intent_record(
        address(WALLET_OWNER),
        POLYGON_CHAIN_ID,
        0,
        0,
        MutationIntentStatus::Preparing,
    );
    assert_eq!(legacy.reconciliation(), None);
}

#[test]
fn reconcile_manually_resolves_all_unresolved_states_and_requires_current_generation() {
    let owner = address(WALLET_OWNER);
    for status in [
        MutationIntentStatus::Preparing,
        MutationIntentStatus::Submitted,
        MutationIntentStatus::AmbiguousNoId,
    ] {
        let registry = intent_registry(Arc::new(InMemoryMutationIntentStore::default()));
        let mut lease = registry
            .begin_intent(
                owner,
                POLYGON_CHAIN_ID,
                RelayerMutationOperation::WalletBatch,
            )
            .unwrap();
        match status {
            MutationIntentStatus::Preparing => {}
            MutationIntentStatus::Submitted => lease.record_submitted("tx-manual").unwrap(),
            MutationIntentStatus::AmbiguousNoId => {
                lease.record_ambiguous_without_id().unwrap()
            }
            _ => unreachable!("test enumerates unresolved statuses only"),
        }
        drop(lease);
        let epoch = registry
            .intent(owner, POLYGON_CHAIN_ID)
            .unwrap()
            .unwrap()
            .epoch();

        registry
            .reconcile_manually(
                owner,
                POLYGON_CHAIN_ID,
                epoch,
                test_reconciliation_evidence(ReconciliationDecision::NotAccepted),
            )
            .unwrap();
        let reconciled = registry
            .intent(owner, POLYGON_CHAIN_ID)
            .unwrap()
            .unwrap();
        assert_eq!(reconciled.status(), MutationIntentStatus::Reconciled);
        let evidence = reconciled.reconciliation().unwrap();
        assert_eq!(evidence.recorded_at_unix(), FIXED_NOW_UNIX);
        assert_eq!(evidence.decision(), ReconciliationDecision::NotAccepted);
        let record_debug = format!("{reconciled:?}");
        assert!(record_debug.contains("NotAccepted"));
        assert!(record_debug.contains("operator_ref_len"));
        assert!(!record_debug.contains(evidence.operator_ref()));
        assert!(registry
            .begin_intent(
                owner,
                POLYGON_CHAIN_ID,
                RelayerMutationOperation::WalletBatch
            )
            .is_ok());
    }

    let absent_registry = intent_registry(Arc::new(InMemoryMutationIntentStore::default()));
    let absent_error = absent_registry
        .reconcile_manually(
            owner,
            POLYGON_CHAIN_ID,
            0,
            test_reconciliation_evidence(ReconciliationDecision::Superseded),
        )
        .unwrap_err();
    assert_eq!(
        absent_error.to_string(),
        "no unresolved mutation intent to reconcile"
    );

    let resolved_registry = intent_registry(Arc::new(InMemoryMutationIntentStore::default()));
    let mut resolved_lease = resolved_registry
        .begin_intent(
            owner,
            POLYGON_CHAIN_ID,
            RelayerMutationOperation::WalletBatch,
        )
        .unwrap();
    resolved_lease.abandon_before_submit().unwrap();
    drop(resolved_lease);
    let resolved_error = resolved_registry
        .reconcile_manually(
            owner,
            POLYGON_CHAIN_ID,
            0,
            test_reconciliation_evidence(ReconciliationDecision::Superseded),
        )
        .unwrap_err();
    assert_eq!(
        resolved_error.to_string(),
        "no unresolved mutation intent to reconcile"
    );

    let aba_registry = intent_registry(Arc::new(InMemoryMutationIntentStore::default()));
    let mut generation_a = aba_registry
        .begin_intent(
            owner,
            POLYGON_CHAIN_ID,
            RelayerMutationOperation::WalletBatch,
        )
        .unwrap();
    generation_a.abandon_before_submit().unwrap();
    drop(generation_a);
    let generation_b = aba_registry
        .begin_intent(
            owner,
            POLYGON_CHAIN_ID,
            RelayerMutationOperation::WalletBatch,
        )
        .unwrap();
    drop(generation_b);
    let before = aba_registry
        .intent(owner, POLYGON_CHAIN_ID)
        .unwrap()
        .unwrap();
    assert_eq!(before.epoch(), 1);
    let stale_error = aba_registry
        .reconcile_manually(
            owner,
            POLYGON_CHAIN_ID,
            0,
            test_reconciliation_evidence(ReconciliationDecision::NotAccepted),
        )
        .unwrap_err();
    assert_eq!(
        stale_error.to_string(),
        "mutation intent generation changed; re-inspect before reconciling"
    );
    assert_eq!(
        aba_registry
            .intent(owner, POLYGON_CHAIN_ID)
            .unwrap()
            .unwrap(),
        before
    );
}

#[test]
fn manual_reconciliation_retries_one_cas_miss_and_reports_a_second_miss() {
    let owner = address(WALLET_OWNER);
    let retry_store = Arc::new(ReconciliationCasMutationIntentStore::default());
    let retry_registry = intent_registry(retry_store.clone());
    prepare_ambiguous_intent(
        &retry_registry,
        owner,
        RelayerMutationOperation::WalletBatch,
    );
    retry_store.fail_next_updates(1);
    retry_registry
        .reconcile_manually(
            owner,
            POLYGON_CHAIN_ID,
            0,
            test_reconciliation_evidence(ReconciliationDecision::NotAccepted),
        )
        .unwrap();
    assert_eq!(
        retry_registry
            .intent(owner, POLYGON_CHAIN_ID)
            .unwrap()
            .unwrap()
            .status(),
        MutationIntentStatus::Reconciled
    );

    let failure_store = Arc::new(ReconciliationCasMutationIntentStore::default());
    let failure_registry = intent_registry(failure_store.clone());
    prepare_ambiguous_intent(
        &failure_registry,
        owner,
        RelayerMutationOperation::WalletBatch,
    );
    let before = failure_registry
        .intent(owner, POLYGON_CHAIN_ID)
        .unwrap()
        .unwrap();
    failure_store.fail_next_updates(2);
    let error = failure_registry
        .reconcile_manually(
            owner,
            POLYGON_CHAIN_ID,
            0,
            test_reconciliation_evidence(ReconciliationDecision::NotAccepted),
        )
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "concurrent intent update; retry reconciliation"
    );
    assert_eq!(
        failure_registry
            .intent(owner, POLYGON_CHAIN_ID)
            .unwrap()
            .unwrap(),
        before
    );
}

#[tokio::test(start_paused = true)]
async fn transaction_adoption_is_epoch_fenced_evidence_bound_and_poll_resolved() {
    let owner = address(WALLET_OWNER);
    let registry = intent_registry(Arc::new(InMemoryMutationIntentStore::default()));
    let mut generation_a = registry
        .begin_intent(
            owner,
            POLYGON_CHAIN_ID,
            RelayerMutationOperation::WalletBatch,
        )
        .unwrap();
    generation_a.abandon_before_submit().unwrap();
    drop(generation_a);
    prepare_ambiguous_intent(&registry, owner, RelayerMutationOperation::WalletBatch);
    let ambiguous = registry
        .intent(owner, POLYGON_CHAIN_ID)
        .unwrap()
        .unwrap();
    assert_eq!(ambiguous.epoch(), 1);

    let stale_error = registry
        .adopt_transaction(
            owner,
            POLYGON_CHAIN_ID,
            0,
            "tx-adopted",
            test_reconciliation_evidence(ReconciliationDecision::ConfirmedOnChain),
        )
        .unwrap_err();
    assert_eq!(
        stale_error.to_string(),
        "mutation intent generation changed; re-inspect before reconciling"
    );
    assert_eq!(
        registry.intent(owner, POLYGON_CHAIN_ID).unwrap().unwrap(),
        ambiguous
    );
    assert!(expect_intent_begin_error(registry.begin_intent(
        owner,
        POLYGON_CHAIN_ID,
        RelayerMutationOperation::WalletBatch,
    ))
    .is_deposit_wallet_mutation_blocked());

    let invalid_error = registry
        .adopt_transaction(
            owner,
            POLYGON_CHAIN_ID,
            1,
            "bad\ntransaction",
            test_reconciliation_evidence(ReconciliationDecision::ConfirmedOnChain),
        )
        .unwrap_err();
    assert!(invalid_error.to_string().contains("transaction id must be"));
    assert_eq!(
        registry.intent(owner, POLYGON_CHAIN_ID).unwrap().unwrap(),
        ambiguous
    );

    registry
        .adopt_transaction(
            owner,
            POLYGON_CHAIN_ID,
            1,
            "tx-adopted",
            test_reconciliation_evidence(ReconciliationDecision::ConfirmedOnChain),
        )
        .unwrap();
    let adopted = registry
        .intent(owner, POLYGON_CHAIN_ID)
        .unwrap()
        .unwrap();
    assert_eq!(adopted.status(), MutationIntentStatus::Submitted);
    assert_eq!(adopted.transaction_id(), Some("tx-adopted"));
    assert_eq!(
        adopted.reconciliation().unwrap().decision(),
        ReconciliationDecision::ConfirmedOnChain
    );
    assert_eq!(
        adopted.reconciliation().unwrap().recorded_at_unix(),
        FIXED_NOW_UNIX
    );

    let (url, handle) = spawn_polling_server(vec![TestResponse::json(
        "200 OK",
        transaction_response_value("tx-adopted", "STATE_CONFIRMED").to_string(),
    )])
    .await;
    let client = polling_test_client(url);
    let outcome = registry
        .gate(&client)
        .reconcile_by_polling(
            owner,
            RelayerPollPolicy::try_new(
                1,
                Duration::from_millis(1),
                Duration::from_millis(1),
            )
            .unwrap(),
            &read_permit(owner),
            std::future::pending::<()>(),
        )
        .await
        .unwrap();
    assert_eq!(
        outcome,
        IntentReconcileOutcome::Resolved(MutationIntentStatus::Confirmed)
    );
    assert_eq!(
        registry
            .intent(owner, POLYGON_CHAIN_ID)
            .unwrap()
            .unwrap()
            .status(),
        MutationIntentStatus::Confirmed
    );
    assert!(registry
        .begin_intent(
            owner,
            POLYGON_CHAIN_ID,
            RelayerMutationOperation::WalletBatch
        )
        .is_ok());
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 1);
    assert!(requests.iter().all(|request| request.method != "POST"));
    assert_eq!(requests[0].path, "/transaction?id=tx-adopted");
}

#[test]
fn transaction_adoption_rejects_non_ambiguous_statuses() {
    let owner = address(WALLET_OWNER);
    for submitted in [false, true] {
        let registry = intent_registry(Arc::new(InMemoryMutationIntentStore::default()));
        let mut lease = registry
            .begin_intent(
                owner,
                POLYGON_CHAIN_ID,
                RelayerMutationOperation::WalletBatch,
            )
            .unwrap();
        if submitted {
            lease.record_submitted("tx-existing").unwrap();
        }
        drop(lease);
        let before = registry
            .intent(owner, POLYGON_CHAIN_ID)
            .unwrap()
            .unwrap();
        let error = registry
            .adopt_transaction(
                owner,
                POLYGON_CHAIN_ID,
                before.epoch(),
                "tx-candidate",
                test_reconciliation_evidence(ReconciliationDecision::ConfirmedOnChain),
            )
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "transaction adoption requires an ambiguous mutation intent"
        );
        assert_eq!(
            registry.intent(owner, POLYGON_CHAIN_ID).unwrap().unwrap(),
            before
        );
    }
}

#[tokio::test(start_paused = true)]
async fn reconcile_by_polling_maps_confirmed_and_terminal_failure_without_submit() {
    let owner = address(WALLET_OWNER);
    for (transaction_id, state, expected_status) in [
        (
            "tx-reconcile-confirmed",
            "STATE_CONFIRMED",
            MutationIntentStatus::Confirmed,
        ),
        (
            "tx-reconcile-failed",
            "STATE_FAILED",
            MutationIntentStatus::Failed,
        ),
    ] {
        let (url, handle) = spawn_polling_server(vec![TestResponse::json(
            "200 OK",
            transaction_response_value(transaction_id, state).to_string(),
        )])
        .await;
        let client = polling_test_client(url);
        let registry = intent_registry(Arc::new(InMemoryMutationIntentStore::default()));
        prepare_submitted_intent(
            &registry,
            owner,
            RelayerMutationOperation::WalletBatch,
            transaction_id,
        );

        let outcome = registry
            .gate(&client)
            .reconcile_by_polling(
                owner,
                RelayerPollPolicy::try_new(
                    1,
                    Duration::from_millis(1),
                    Duration::from_millis(1),
                )
                .unwrap(),
                &read_permit(owner),
                std::future::pending::<()>(),
            )
            .await
            .unwrap();
        assert_eq!(outcome, IntentReconcileOutcome::Resolved(expected_status));
        assert_eq!(
            registry
                .intent(owner, POLYGON_CHAIN_ID)
                .unwrap()
                .unwrap()
                .status(),
            expected_status
        );
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
        assert!(requests.iter().all(|request| request.method != "POST"));
    }
}

#[tokio::test(start_paused = true)]
async fn reconcile_by_polling_preserves_pending_cancelled_and_unknown_locks_without_submit() {
    let owner = address(WALLET_OWNER);
    let policy = RelayerPollPolicy::try_new(
        2,
        Duration::from_secs(1),
        Duration::from_secs(1),
    )
    .unwrap();

    let pending_id = "tx-reconcile-pending";
    let (pending_url, pending_handle) = spawn_polling_server(vec![
        TestResponse::json(
            "200 OK",
            transaction_response_value(pending_id, "STATE_NEW").to_string(),
        ),
        TestResponse::json(
            "200 OK",
            transaction_response_value(pending_id, "STATE_NEW").to_string(),
        ),
    ])
    .await;
    let pending_client = polling_test_client(pending_url);
    let pending_registry = intent_registry(Arc::new(InMemoryMutationIntentStore::default()));
    prepare_submitted_intent(
        &pending_registry,
        owner,
        RelayerMutationOperation::WalletBatch,
        pending_id,
    );
    let pending_outcome = pending_registry
        .gate(&pending_client)
        .reconcile_by_polling(
            owner,
            policy,
            &read_permit(owner),
            std::future::pending::<()>(),
        )
        .await
        .unwrap();
    assert_eq!(
        pending_outcome,
        IntentReconcileOutcome::StillPending {
            attempts: 2,
            last_state: Some(RelayerTransactionState::New),
        }
    );
    let pending_record = pending_registry
        .intent(owner, POLYGON_CHAIN_ID)
        .unwrap()
        .unwrap();
    assert_eq!(pending_record.status(), MutationIntentStatus::Submitted);
    assert_eq!(pending_record.last_observed_state(), Some("New"));
    let pending_requests = pending_handle.await.unwrap();
    assert_eq!(pending_requests.len(), 2);
    assert!(pending_requests
        .iter()
        .all(|request| request.method != "POST"));

    let cancel_id = "tx-reconcile-cancelled";
    let (cancel_url, cancel_handle) = spawn_polling_server(Vec::new()).await;
    let cancel_client = polling_test_client(cancel_url);
    let cancel_registry = intent_registry(Arc::new(InMemoryMutationIntentStore::default()));
    prepare_submitted_intent(
        &cancel_registry,
        owner,
        RelayerMutationOperation::WalletBatch,
        cancel_id,
    );
    let cancel_before = cancel_registry
        .intent(owner, POLYGON_CHAIN_ID)
        .unwrap()
        .unwrap();
    let cancel_outcome = cancel_registry
        .gate(&cancel_client)
        .reconcile_by_polling(
            owner,
            policy,
            &read_permit(owner),
            std::future::ready(()),
        )
        .await
        .unwrap();
    assert_eq!(
        cancel_outcome,
        IntentReconcileOutcome::Cancelled { attempts: 0 }
    );
    assert_eq!(
        cancel_registry
            .intent(owner, POLYGON_CHAIN_ID)
            .unwrap()
            .unwrap(),
        cancel_before
    );
    assert!(cancel_handle.await.unwrap().is_empty());

    let unknown_id = "tx-reconcile-unknown";
    let (unknown_url, unknown_handle) = spawn_polling_server(vec![TestResponse::json(
        "200 OK",
        transaction_response_value(unknown_id, "STATE_FUTURE").to_string(),
    )])
    .await;
    let unknown_client = polling_test_client(unknown_url);
    let unknown_registry = intent_registry(Arc::new(InMemoryMutationIntentStore::default()));
    prepare_submitted_intent(
        &unknown_registry,
        owner,
        RelayerMutationOperation::WalletBatch,
        unknown_id,
    );
    let unknown_before = unknown_registry
        .intent(owner, POLYGON_CHAIN_ID)
        .unwrap()
        .unwrap();
    let unknown_error = unknown_registry
        .gate(&unknown_client)
        .reconcile_by_polling(
            owner,
            policy,
            &read_permit(owner),
            std::future::pending::<()>(),
        )
        .await
        .unwrap_err();
    assert!(unknown_error.is_deposit_wallet_reconciliation_required());
    assert_eq!(
        unknown_registry
            .intent(owner, POLYGON_CHAIN_ID)
            .unwrap()
            .unwrap(),
        unknown_before
    );
    let unknown_requests = unknown_handle.await.unwrap();
    assert_eq!(unknown_requests.len(), 1);
    assert!(unknown_requests
        .iter()
        .all(|request| request.method != "POST"));
}

#[tokio::test]
async fn reconcile_by_polling_rejects_non_submitted_records_before_http() {
    let owner = address(WALLET_OWNER);
    let (url, handle) = spawn_optional_request_server().await;
    let client = polling_test_client(url);
    let policy = RelayerPollPolicy::try_new(
        1,
        Duration::from_millis(1),
        Duration::from_millis(1),
    )
    .unwrap();

    let absent_registry = intent_registry(Arc::new(InMemoryMutationIntentStore::default()));
    let absent_error = absent_registry
        .gate(&client)
        .reconcile_by_polling(
            owner,
            policy,
            &read_permit(owner),
            std::future::pending::<()>(),
        )
        .await
        .unwrap_err();
    assert_eq!(
        absent_error.to_string(),
        "no submitted mutation intent to reconcile for this owner"
    );

    for ambiguous in [false, true] {
        let registry = intent_registry(Arc::new(InMemoryMutationIntentStore::default()));
        if ambiguous {
            prepare_ambiguous_intent(&registry, owner, RelayerMutationOperation::WalletBatch);
        } else {
            let lease = registry
                .begin_intent(
                    owner,
                    POLYGON_CHAIN_ID,
                    RelayerMutationOperation::WalletBatch,
                )
                .unwrap();
            drop(lease);
        }
        let error = registry
            .gate(&client)
            .reconcile_by_polling(
                owner,
                policy,
                &read_permit(owner),
                std::future::pending::<()>(),
            )
            .await
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "intent has no transaction id; use the recent-transaction report and manual adoption or reconciliation"
        );
    }

    let resolved_registry = intent_registry(Arc::new(InMemoryMutationIntentStore::default()));
    let mut resolved = resolved_registry
        .begin_intent(
            owner,
            POLYGON_CHAIN_ID,
            RelayerMutationOperation::WalletBatch,
        )
        .unwrap();
    resolved.abandon_before_submit().unwrap();
    drop(resolved);
    let resolved_error = resolved_registry
        .gate(&client)
        .reconcile_by_polling(
            owner,
            policy,
            &read_permit(owner),
            std::future::pending::<()>(),
        )
        .await
        .unwrap_err();
    assert_eq!(
        resolved_error.to_string(),
        "no submitted mutation intent to reconcile for this owner"
    );

    let defensive_store = Arc::new(InMemoryMutationIntentStore::default());
    defensive_store
        .seed_for_test(serialized_intent_record(
            owner,
            POLYGON_CHAIN_ID,
            0,
            0,
            MutationIntentStatus::Submitted,
        ))
        .unwrap();
    let defensive_registry = intent_registry(defensive_store);
    let defensive_error = defensive_registry
        .gate(&client)
        .reconcile_by_polling(
            owner,
            policy,
            &read_permit(owner),
            std::future::pending::<()>(),
        )
        .await
        .unwrap_err();
    assert_eq!(
        defensive_error.to_string(),
        "intent has no transaction id; use the recent-transaction report and manual adoption or reconciliation"
    );
    assert!(handle.await.unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn reconcile_by_polling_uses_wallet_create_type_and_rejects_wallet_receipt() {
    let owner = address(WALLET_OWNER);
    let transaction_id = "tx-reconcile-create-type";
    let (url, handle) = spawn_polling_server(vec![TestResponse::json(
        "200 OK",
        transaction_response_value(transaction_id, "STATE_CONFIRMED").to_string(),
    )])
    .await;
    let client = polling_test_client(url);
    let registry = intent_registry(Arc::new(InMemoryMutationIntentStore::default()));
    prepare_submitted_intent(
        &registry,
        owner,
        RelayerMutationOperation::WalletCreate,
        transaction_id,
    );
    let before = registry
        .intent(owner, POLYGON_CHAIN_ID)
        .unwrap()
        .unwrap();

    let error = registry
        .gate(&client)
        .reconcile_by_polling(
            owner,
            RelayerPollPolicy::try_new(
                1,
                Duration::from_millis(1),
                Duration::from_millis(1),
            )
            .unwrap(),
            &read_permit(owner),
            std::future::pending::<()>(),
        )
        .await
        .unwrap_err();
    assert!(error.is_deposit_wallet_reconciliation_required());
    assert_eq!(
        registry.intent(owner, POLYGON_CHAIN_ID).unwrap().unwrap(),
        before
    );
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 1);
    assert!(requests.iter().all(|request| request.method != "POST"));
}

#[tokio::test]
async fn ambiguous_candidate_report_filters_and_redacts_fixture_without_state_change() {
    let owner = address(WALLET_OWNER);
    let (url, handle) = spawn_server(vec![TestResponse::json(
        "200 OK",
        fixture_text("wallet_recent_transactions_response.json"),
    )])
    .await;
    let client = test_client(url);
    let registry = intent_registry(Arc::new(InMemoryMutationIntentStore::default()));
    prepare_ambiguous_intent(&registry, owner, RelayerMutationOperation::WalletBatch);
    let before = registry
        .intent(owner, POLYGON_CHAIN_ID)
        .unwrap()
        .unwrap();

    let report = registry
        .gate(&client)
        .report_ambiguous_candidates(owner, &read_permit(owner))
        .await
        .unwrap();
    assert_eq!(report.owner(), "0x6e0c...B5b5");
    assert_eq!(report.intent_status(), "AmbiguousNoId");
    let payload_hash = test_payload_keccak256();
    assert_eq!(
        report.intent_payload_keccak256(),
        Some(payload_hash.as_str())
    );
    assert_eq!(report.intent_epoch(), before.epoch());
    assert_eq!(report.intent_created_at_unix(), FIXED_NOW_UNIX);
    assert_eq!(report.skipped_items(), 4);
    assert_eq!(
        report.redaction(),
        "auth material and raw bodies are intentionally omitted; candidates are read-only"
    );
    assert_eq!(report.candidates().len(), 3);
    assert_eq!(
        (
            report.candidates()[0].transaction_id(),
            report.candidates()[0].state_label(),
            report.candidates()[0].tx_type(),
            report.candidates()[0].created_at(),
        ),
        (
            "recent-wallet-confirmed",
            "Confirmed",
            WALLET_TRANSACTION_TYPE,
            Some("2024-07-14T21:13:08.819782Z"),
        )
    );
    assert_eq!(
        (
            report.candidates()[1].transaction_id(),
            report.candidates()[1].state_label(),
            report.candidates()[1].tx_type(),
        ),
        (
            "recent-create-executed",
            "Executed",
            WALLET_CREATE_TRANSACTION_TYPE,
        )
    );
    assert_eq!(
        (
            report.candidates()[2].transaction_id(),
            report.candidates()[2].state_label(),
            report.candidates()[2].tx_type(),
            report.candidates()[2].created_at(),
        ),
        (
            "recent-wallet-new",
            "New",
            WALLET_TRANSACTION_TYPE,
            None,
        )
    );
    let serialized = serde_json::to_string(&report).unwrap();
    assert!(!serialized.contains("STATE_FUTURE_SECRET_LABEL"));
    assert!(!serialized.contains(API_KEY));
    assert_eq!(
        registry.intent(owner, POLYGON_CHAIN_ID).unwrap().unwrap(),
        before
    );

    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "GET");
    assert_eq!(requests[0].path, "/transactions");
    assert!(requests[0].body.is_empty());
    assert_eq!(requests[0].header("RELAYER_API_KEY"), Some(API_KEY));
    assert_eq!(
        requests[0].header("RELAYER_API_KEY_ADDRESS"),
        Some(to_checksum(&address(API_KEY_ADDRESS), None).as_str())
    );
}

#[tokio::test]
async fn ambiguous_candidate_report_rejects_scope_before_http_and_enforces_item_limit() {
    let owner = address(WALLET_OWNER);
    let (permit_url, permit_handle) = spawn_optional_request_server().await;
    let permit_client = test_client(permit_url);
    let permit_registry = intent_registry(Arc::new(InMemoryMutationIntentStore::default()));
    let permit_error = permit_registry
        .gate(&permit_client)
        .report_ambiguous_candidates(owner, &read_permit(address(OTHER_OWNER)))
        .await
        .unwrap_err();
    assert!(permit_error.is_deposit_wallet_read_blocked());
    assert!(permit_handle.await.unwrap().is_empty());

    let (absent_url, absent_handle) = spawn_optional_request_server().await;
    let absent_client = test_client(absent_url);
    let absent_registry = intent_registry(Arc::new(InMemoryMutationIntentStore::default()));
    let absent_error = absent_registry
        .gate(&absent_client)
        .report_ambiguous_candidates(owner, &read_permit(owner))
        .await
        .unwrap_err();
    assert_eq!(
        absent_error.to_string(),
        "no unresolved mutation intent; nothing to report"
    );
    assert!(absent_handle.await.unwrap().is_empty());

    let (resolved_url, resolved_handle) = spawn_optional_request_server().await;
    let resolved_client = test_client(resolved_url);
    let resolved_registry = intent_registry(Arc::new(InMemoryMutationIntentStore::default()));
    let mut resolved = resolved_registry
        .begin_intent(
            owner,
            POLYGON_CHAIN_ID,
            RelayerMutationOperation::WalletBatch,
        )
        .unwrap();
    resolved.abandon_before_submit().unwrap();
    drop(resolved);
    let resolved_error = resolved_registry
        .gate(&resolved_client)
        .report_ambiguous_candidates(owner, &read_permit(owner))
        .await
        .unwrap_err();
    assert_eq!(
        resolved_error.to_string(),
        "no unresolved mutation intent; nothing to report"
    );
    assert!(resolved_handle.await.unwrap().is_empty());

    let oversized = Value::Array((0..33).map(|_| json!({})).collect()).to_string();
    let (limit_url, limit_handle) =
        spawn_server(vec![TestResponse::json("200 OK", oversized)]).await;
    let limit_client = test_client(limit_url);
    let limit_registry = intent_registry(Arc::new(InMemoryMutationIntentStore::default()));
    prepare_ambiguous_intent(
        &limit_registry,
        owner,
        RelayerMutationOperation::WalletBatch,
    );
    let limit_before = limit_registry
        .intent(owner, POLYGON_CHAIN_ID)
        .unwrap()
        .unwrap();
    let limit_error = limit_registry
        .gate(&limit_client)
        .report_ambiguous_candidates(owner, &read_permit(owner))
        .await
        .unwrap_err();
    assert_eq!(
        limit_error.to_string(),
        "transactions response item limit exceeded"
    );
    assert_eq!(
        limit_registry
            .intent(owner, POLYGON_CHAIN_ID)
            .unwrap()
            .unwrap(),
        limit_before
    );
    let limit_requests = limit_handle.await.unwrap();
    assert_eq!(limit_requests.len(), 1);
    assert!(limit_requests.iter().all(|request| request.method != "POST"));
}

#[tokio::test]
async fn ambiguous_candidate_report_rejects_non_array_or_invalid_json() {
    let owner = address(WALLET_OWNER);
    for body in ["{}", "not-json"] {
        let (url, handle) = spawn_server(vec![TestResponse::json("200 OK", body)]).await;
        let client = test_client(url);
        let registry = intent_registry(Arc::new(InMemoryMutationIntentStore::default()));
        prepare_ambiguous_intent(&registry, owner, RelayerMutationOperation::WalletBatch);
        let before = registry
            .intent(owner, POLYGON_CHAIN_ID)
            .unwrap()
            .unwrap();
        let error = registry
            .gate(&client)
            .report_ambiguous_candidates(owner, &read_permit(owner))
            .await
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "could not parse transactions response"
        );
        assert_eq!(
            registry.intent(owner, POLYGON_CHAIN_ID).unwrap().unwrap(),
            before
        );
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
        assert!(requests.iter().all(|request| request.method != "POST"));
    }
}

#[test]
fn mutation_intent_lease_lifecycle_preserves_versions_timestamps_and_terminal_reentry() {
    let store = Arc::new(InMemoryMutationIntentStore::default());
    let registry = OwnerMutationRegistry::with_clock(
        store,
        Arc::new(AdvancingClock {
            now_unix: AtomicU64::new(FIXED_NOW_UNIX),
        }),
    );
    let owner = address(WALLET_OWNER);
    let mut lease = registry
        .begin_intent(
            owner,
            POLYGON_CHAIN_ID,
            RelayerMutationOperation::WalletBatch,
        )
        .unwrap();

    let preparing = registry
        .intent(owner, POLYGON_CHAIN_ID)
        .unwrap()
        .unwrap();
    assert_eq!(preparing.owner(), owner);
    assert_eq!(preparing.chain_id(), POLYGON_CHAIN_ID);
    assert_eq!(preparing.epoch(), 0);
    assert_eq!(preparing.revision(), 0);
    assert_eq!(preparing.operation(), RelayerMutationOperation::WalletBatch);
    assert_eq!(preparing.status(), MutationIntentStatus::Preparing);
    assert_eq!(preparing.nonce(), None);
    assert_eq!(preparing.created_at_unix(), FIXED_NOW_UNIX);
    assert_eq!(preparing.updated_at_unix(), FIXED_NOW_UNIX);

    let payload_hash = test_payload_keccak256();
    lease
        .record_payload(&payload_hash, Some(U256::from(u64::MAX) + U256::one()))
        .unwrap();
    let payload_record = registry
        .intent(owner, POLYGON_CHAIN_ID)
        .unwrap()
        .unwrap();
    assert_eq!(payload_record.revision(), 1);
    assert_eq!(payload_record.payload_keccak256(), Some(payload_hash.as_str()));
    assert_eq!(payload_record.deadline_unix(), Some(u64::MAX));
    assert_eq!(payload_record.created_at_unix(), FIXED_NOW_UNIX);
    assert_eq!(payload_record.updated_at_unix(), FIXED_NOW_UNIX + 1);

    lease.record_submitted("tx-intent-lifecycle").unwrap();
    let submitted = registry
        .intent(owner, POLYGON_CHAIN_ID)
        .unwrap()
        .unwrap();
    assert_eq!(submitted.status(), MutationIntentStatus::Submitted);
    assert_eq!(submitted.revision(), 2);
    assert_eq!(submitted.transaction_id(), Some("tx-intent-lifecycle"));
    assert_eq!(submitted.created_at_unix(), FIXED_NOW_UNIX);
    assert_eq!(submitted.updated_at_unix(), FIXED_NOW_UNIX + 2);

    lease
        .record_observed_receipt(&intent_receipt(
            "tx-intent-lifecycle",
            RelayerTransactionState::Confirmed,
        ))
        .unwrap();
    let confirmed = registry
        .intent(owner, POLYGON_CHAIN_ID)
        .unwrap()
        .unwrap();
    assert_eq!(confirmed.status(), MutationIntentStatus::Confirmed);
    assert_eq!(confirmed.revision(), 3);
    assert_eq!(confirmed.last_observed_state(), Some("Confirmed"));
    assert_eq!(confirmed.created_at_unix(), FIXED_NOW_UNIX);
    assert_eq!(confirmed.updated_at_unix(), FIXED_NOW_UNIX + 3);

    let next = registry
        .begin_intent(
            owner,
            POLYGON_CHAIN_ID,
            RelayerMutationOperation::WalletBatch,
        )
        .unwrap();
    let next_record = registry
        .intent(owner, POLYGON_CHAIN_ID)
        .unwrap()
        .unwrap();
    assert_eq!(next_record.status(), MutationIntentStatus::Preparing);
    assert_eq!(next_record.epoch(), 1);
    assert_eq!(next_record.revision(), 0);
    assert_eq!(next_record.created_at_unix(), FIXED_NOW_UNIX + 4);
    drop(next);
}

#[test]
fn mutation_intent_binding_and_terminal_failure_rules_reject_misdelivery_and_regression() {
    let store = Arc::new(InMemoryMutationIntentStore::default());
    let registry = intent_registry(store);
    let owner = address(WALLET_OWNER);
    let mut lease = registry
        .begin_intent(
            owner,
            POLYGON_CHAIN_ID,
            RelayerMutationOperation::WalletBatch,
        )
        .unwrap();
    lease
        .record_payload(&test_payload_keccak256(), Some(U256::from(123u64)))
        .unwrap();
    lease.record_submitted("tx-bound-a").unwrap();
    let submitted = registry
        .intent(owner, POLYGON_CHAIN_ID)
        .unwrap()
        .unwrap();

    let mismatch = lease
        .record_observed_receipt(&intent_receipt(
            "tx-bound-b",
            RelayerTransactionState::Confirmed,
        ))
        .unwrap_err();
    assert!(mismatch
        .to_string()
        .contains("observed receipt does not match this intent's transaction"));
    assert_eq!(
        registry.intent(owner, POLYGON_CHAIN_ID).unwrap().unwrap(),
        submitted
    );

    let duplicate_submit = lease.record_submitted("tx-bound-a").unwrap_err();
    assert!(duplicate_submit
        .to_string()
        .contains("invalid mutation intent transition"));
    assert_eq!(
        registry.intent(owner, POLYGON_CHAIN_ID).unwrap().unwrap(),
        submitted
    );

    registry
        .record_terminal_failure(
            owner,
            POLYGON_CHAIN_ID,
            "tx-bound-b",
            &RelayerError::TransactionFailed("synthetic".to_string()),
        )
        .unwrap();
    assert_eq!(
        registry.intent(owner, POLYGON_CHAIN_ID).unwrap().unwrap(),
        submitted
    );

    let reconciliation = RelayerError::reconciliation_required("synthetic ambiguous failure");
    let error = registry
        .record_terminal_failure(
            owner,
            POLYGON_CHAIN_ID,
            "tx-bound-a",
            &reconciliation,
        )
        .unwrap_err();
    assert!(error
        .to_string()
        .contains("only bound terminal failures may resolve an intent"));
    assert_eq!(
        registry
            .intent(owner, POLYGON_CHAIN_ID)
            .unwrap()
            .unwrap()
            .status(),
        MutationIntentStatus::Submitted
    );

    registry
        .record_terminal_failure(
            owner,
            POLYGON_CHAIN_ID,
            "tx-bound-a",
            &RelayerError::TransactionFailed("synthetic".to_string()),
        )
        .unwrap();
    let failed = registry
        .intent(owner, POLYGON_CHAIN_ID)
        .unwrap()
        .unwrap();
    assert_eq!(failed.status(), MutationIntentStatus::Failed);
    assert_eq!(failed.last_observed_state(), Some("Failed"));
    assert!(registry
        .begin_intent(
            owner,
            POLYGON_CHAIN_ID,
            RelayerMutationOperation::WalletBatch,
        )
        .is_ok());
}

#[test]
fn mutation_intent_poll_outcomes_are_transaction_bound_and_distrust_variant_names() {
    let store = Arc::new(InMemoryMutationIntentStore::default());
    let registry = intent_registry(store);
    let owner = address(WALLET_OWNER);
    let mut lease = registry
        .begin_intent(
            owner,
            POLYGON_CHAIN_ID,
            RelayerMutationOperation::WalletBatch,
        )
        .unwrap();
    lease
        .record_payload(&test_payload_keccak256(), None)
        .unwrap();
    lease.record_submitted("tx-poll-a").unwrap();
    let submitted = registry
        .intent(owner, POLYGON_CHAIN_ID)
        .unwrap()
        .unwrap();

    registry
        .record_poll_outcome(
            owner,
            POLYGON_CHAIN_ID,
            "tx-poll-a",
            &RelayerPollOutcome::Confirmed(intent_receipt(
                "tx-poll-a",
                RelayerTransactionState::New,
            )),
        )
        .unwrap();
    assert_eq!(
        registry.intent(owner, POLYGON_CHAIN_ID).unwrap().unwrap(),
        submitted
    );

    registry
        .record_poll_outcome(
            owner,
            POLYGON_CHAIN_ID,
            "tx-poll-b",
            &RelayerPollOutcome::Confirmed(intent_receipt(
                "tx-poll-b",
                RelayerTransactionState::Confirmed,
            )),
        )
        .unwrap();
    assert_eq!(
        registry.intent(owner, POLYGON_CHAIN_ID).unwrap().unwrap(),
        submitted
    );

    registry
        .record_poll_outcome(
            owner,
            POLYGON_CHAIN_ID,
            "tx-poll-a",
            &RelayerPollOutcome::Confirmed(intent_receipt(
                "tx-poll-b",
                RelayerTransactionState::Confirmed,
            )),
        )
        .unwrap();
    assert_eq!(
        registry.intent(owner, POLYGON_CHAIN_ID).unwrap().unwrap(),
        submitted
    );

    registry
        .record_poll_outcome(
            owner,
            POLYGON_CHAIN_ID,
            "tx-poll-a",
            &RelayerPollOutcome::Exhausted {
                attempts: 3,
                last_state: Some(RelayerTransactionState::Mined),
            },
        )
        .unwrap();
    let exhausted = registry
        .intent(owner, POLYGON_CHAIN_ID)
        .unwrap()
        .unwrap();
    assert_eq!(exhausted.status(), MutationIntentStatus::Submitted);
    assert_eq!(exhausted.last_observed_state(), Some("Mined"));

    registry
        .record_poll_outcome(
            owner,
            POLYGON_CHAIN_ID,
            "tx-poll-a",
            &RelayerPollOutcome::Cancelled { attempts: 3 },
        )
        .unwrap();
    assert_eq!(
        registry.intent(owner, POLYGON_CHAIN_ID).unwrap().unwrap(),
        exhausted
    );

    registry
        .record_poll_outcome(
            owner,
            POLYGON_CHAIN_ID,
            "tx-poll-a",
            &RelayerPollOutcome::Confirmed(intent_receipt(
                "tx-poll-a",
                RelayerTransactionState::Confirmed,
            )),
        )
        .unwrap();
    assert_eq!(
        registry
            .intent(owner, POLYGON_CHAIN_ID)
            .unwrap()
            .unwrap()
            .status(),
        MutationIntentStatus::Confirmed
    );

    let preparing = registry
        .begin_intent(
            owner,
            POLYGON_CHAIN_ID,
            RelayerMutationOperation::WalletBatch,
        )
        .unwrap();
    let preparing_record = registry
        .intent(owner, POLYGON_CHAIN_ID)
        .unwrap()
        .unwrap();
    registry
        .record_poll_outcome(
            owner,
            POLYGON_CHAIN_ID,
            "tx-poll-a",
            &RelayerPollOutcome::Confirmed(intent_receipt(
                "tx-poll-a",
                RelayerTransactionState::Confirmed,
            )),
        )
        .unwrap();
    assert_eq!(
        registry.intent(owner, POLYGON_CHAIN_ID).unwrap().unwrap(),
        preparing_record
    );
    drop(preparing);
}

#[test]
fn mutation_intents_block_unresolved_scope_and_keep_other_owner_or_chain_independent() {
    let store = Arc::new(InMemoryMutationIntentStore::default());
    let registry = intent_registry(store);
    let owner = address(WALLET_OWNER);
    let other_owner = address(OTHER_OWNER);
    let mut preparing = registry
        .begin_intent(
            owner,
            POLYGON_CHAIN_ID,
            RelayerMutationOperation::WalletBatch,
        )
        .unwrap();

    let blocked = expect_intent_begin_error(registry.begin_intent(
            owner,
            POLYGON_CHAIN_ID,
            RelayerMutationOperation::WalletBatch,
        ));
    assert!(blocked.is_deposit_wallet_mutation_blocked());
    assert!(blocked.to_string().contains("status Preparing"));
    assert!(registry
        .begin_intent(
            other_owner,
            POLYGON_CHAIN_ID,
            RelayerMutationOperation::WalletBatch,
        )
        .is_ok());
    assert!(registry
        .begin_intent(
            owner,
            AMOY_CHAIN_ID,
            RelayerMutationOperation::WalletBatch,
        )
        .is_ok());

    preparing.record_ambiguous_without_id().unwrap();
    let blocked = expect_intent_begin_error(registry.begin_intent(
            owner,
            POLYGON_CHAIN_ID,
            RelayerMutationOperation::WalletBatch,
        ));
    assert!(blocked.to_string().contains("status AmbiguousNoId"));

    let submitted_store = Arc::new(InMemoryMutationIntentStore::default());
    let submitted_registry = intent_registry(submitted_store);
    let mut submitted = submitted_registry
        .begin_intent(
            owner,
            POLYGON_CHAIN_ID,
            RelayerMutationOperation::WalletBatch,
        )
        .unwrap();
    submitted.record_submitted("tx-unresolved").unwrap();
    let blocked = expect_intent_begin_error(submitted_registry.begin_intent(
            owner,
            POLYGON_CHAIN_ID,
            RelayerMutationOperation::WalletBatch,
        ));
    assert!(blocked.to_string().contains("status Submitted"));

    let failed_store = Arc::new(InMemoryMutationIntentStore::default());
    let failed_registry = intent_registry(failed_store);
    let mut failed = failed_registry
        .begin_intent(
            owner,
            POLYGON_CHAIN_ID,
            RelayerMutationOperation::WalletBatch,
        )
        .unwrap();
    failed.abandon_before_submit().unwrap();
    assert!(failed_registry
        .begin_intent(
            owner,
            POLYGON_CHAIN_ID,
            RelayerMutationOperation::WalletBatch,
        )
        .is_ok());

    let reconciled_store = InMemoryMutationIntentStore::default();
    reconciled_store
        .try_begin(serialized_intent_record(
            owner,
            POLYGON_CHAIN_ID,
            99,
            99,
            MutationIntentStatus::Reconciled,
        ))
        .unwrap();
    let outcome = reconciled_store
        .try_begin(serialized_intent_record(
            owner,
            POLYGON_CHAIN_ID,
            777,
            777,
            MutationIntentStatus::Preparing,
        ))
        .unwrap();
    let TryBeginOutcome::Started(reconciled_successor) = outcome else {
        panic!("a reconciled record must admit a successor")
    };
    assert_eq!(reconciled_successor.epoch(), 1);
    assert_eq!(reconciled_successor.revision(), 0);
}

#[test]
fn mutation_intent_transition_guards_leave_records_unchanged() {
    let owner = address(WALLET_OWNER);
    let store = Arc::new(InMemoryMutationIntentStore::default());
    let registry = intent_registry(store);
    let mut lease = registry
        .begin_intent(
            owner,
            POLYGON_CHAIN_ID,
            RelayerMutationOperation::WalletBatch,
        )
        .unwrap();
    let preparing = registry
        .intent(owner, POLYGON_CHAIN_ID)
        .unwrap()
        .unwrap();
    let error = lease
        .record_observed_receipt(&intent_receipt(
            "tx-transition",
            RelayerTransactionState::New,
        ))
        .unwrap_err();
    assert!(error
        .to_string()
        .contains("invalid mutation intent transition"));
    assert_eq!(
        registry.intent(owner, POLYGON_CHAIN_ID).unwrap().unwrap(),
        preparing
    );

    lease.record_submitted("tx-transition").unwrap();
    let submitted = registry
        .intent(owner, POLYGON_CHAIN_ID)
        .unwrap()
        .unwrap();
    assert!(lease.abandon_before_submit().is_err());
    assert_eq!(
        registry.intent(owner, POLYGON_CHAIN_ID).unwrap().unwrap(),
        submitted
    );

    let failed_store = Arc::new(InMemoryMutationIntentStore::default());
    let failed_registry = intent_registry(failed_store);
    let mut failed = failed_registry
        .begin_intent(
            owner,
            POLYGON_CHAIN_ID,
            RelayerMutationOperation::WalletBatch,
        )
        .unwrap();
    failed.abandon_before_submit().unwrap();
    let failed_record = failed_registry
        .intent(owner, POLYGON_CHAIN_ID)
        .unwrap()
        .unwrap();
    assert!(failed
        .record_payload(&test_payload_keccak256(), None)
        .is_err());
    assert_eq!(
        failed_registry
            .intent(owner, POLYGON_CHAIN_ID)
            .unwrap()
            .unwrap(),
        failed_record
    );
}

#[test]
fn mutation_intent_store_assigns_generations_fences_stale_writers_and_fails_closed_at_bounds() {
    fn assert_send<T: Send>() {}
    assert_send::<MutationIntentLease<'static>>();

    let owner = address(WALLET_OWNER);
    let counting_store = Arc::new(CountingMutationIntentStore::default());
    let counting_registry = intent_registry(counting_store.clone());
    let first = counting_registry
        .begin_intent(
            owner,
            POLYGON_CHAIN_ID,
            RelayerMutationOperation::WalletBatch,
        )
        .unwrap();
    assert!(counting_registry
        .begin_intent(
            owner,
            POLYGON_CHAIN_ID,
            RelayerMutationOperation::WalletBatch,
        )
        .is_err());
    assert_eq!(
        counting_store
            .try_begin_calls
            .load(AtomicOrdering::SeqCst),
        2
    );
    assert_eq!(counting_store.load_calls.load(AtomicOrdering::SeqCst), 0);
    drop(first);

    let generation_store = InMemoryMutationIntentStore::default();
    generation_store
        .seed_for_test(serialized_intent_record(
            owner,
            POLYGON_CHAIN_ID,
            41,
            9,
            MutationIntentStatus::Confirmed,
        ))
        .unwrap();
    let started = generation_store
        .try_begin(serialized_intent_record(
            owner,
            POLYGON_CHAIN_ID,
            999,
            999,
            MutationIntentStatus::Preparing,
        ))
        .unwrap();
    let TryBeginOutcome::Started(assigned) = started else {
        panic!("resolved generation must start")
    };
    assert_eq!(assigned.epoch(), 42);
    assert_eq!(assigned.revision(), 0);

    let invariant_store = InMemoryMutationIntentStore::default();
    let started = invariant_store
        .try_begin(serialized_intent_record(
            owner,
            POLYGON_CHAIN_ID,
            99,
            99,
            MutationIntentStatus::Preparing,
        ))
        .unwrap();
    let TryBeginOutcome::Started(invariant_record) = started else {
        panic!("first invariant record must start")
    };
    let mut tampered = serde_json::to_value(&invariant_record).unwrap();
    tampered["created_at_unix"] = json!(1);
    tampered["updated_at_unix"] = json!(FIXED_NOW_UNIX + 10);
    let tampered = serde_json::from_value(tampered).unwrap();
    assert!(invariant_store.update(0, 0, tampered).unwrap());
    let preserved = invariant_store
        .load(owner, POLYGON_CHAIN_ID)
        .unwrap()
        .unwrap();
    assert_eq!(preserved.created_at_unix(), FIXED_NOW_UNIX);
    assert_eq!(preserved.updated_at_unix(), FIXED_NOW_UNIX + 10);
    assert_eq!(preserved.revision(), 1);

    let fencing_store = Arc::new(InMemoryMutationIntentStore::default());
    let fencing_registry = intent_registry(fencing_store.clone());
    let mut lease_a = fencing_registry
        .begin_intent(
            owner,
            POLYGON_CHAIN_ID,
            RelayerMutationOperation::WalletBatch,
        )
        .unwrap();
    lease_a
        .record_payload(&test_payload_keccak256(), None)
        .unwrap();
    lease_a.record_submitted("tx-fence-a").unwrap();
    let revision_two = fencing_registry
        .intent(owner, POLYGON_CHAIN_ID)
        .unwrap()
        .unwrap();
    assert_eq!(revision_two.revision(), 2);
    assert!(!fencing_store
        .update(revision_two.epoch(), 0, revision_two.clone())
        .unwrap());
    assert_eq!(
        fencing_registry
            .intent(owner, POLYGON_CHAIN_ID)
            .unwrap()
            .unwrap(),
        revision_two
    );

    fencing_registry
        .record_poll_outcome(
            owner,
            POLYGON_CHAIN_ID,
            "tx-fence-a",
            &RelayerPollOutcome::Confirmed(intent_receipt(
                "tx-fence-a",
                RelayerTransactionState::Confirmed,
            )),
        )
        .unwrap();
    let lease_b = fencing_registry
        .begin_intent(
            owner,
            POLYGON_CHAIN_ID,
            RelayerMutationOperation::WalletBatch,
        )
        .unwrap();
    let lease_b_record = fencing_registry
        .intent(owner, POLYGON_CHAIN_ID)
        .unwrap()
        .unwrap();
    let stale = lease_a
        .record_observed_receipt(&intent_receipt(
            "tx-fence-a",
            RelayerTransactionState::Mined,
        ))
        .unwrap_err();
    assert_eq!(stale.to_string(), STALE_INTENT_LEASE_ERROR);
    assert_eq!(
        lease_a
            .record_observed_receipt(&intent_receipt(
                "tx-fence-a",
                RelayerTransactionState::Mined,
            ))
            .unwrap_err()
            .to_string(),
        STALE_INTENT_LEASE_ERROR
    );
    assert_eq!(
        fencing_registry
            .intent(owner, POLYGON_CHAIN_ID)
            .unwrap()
            .unwrap(),
        lease_b_record
    );
    drop(lease_b);

    let epoch_store = InMemoryMutationIntentStore::default();
    epoch_store
        .seed_for_test(serialized_intent_record(
            owner,
            POLYGON_CHAIN_ID,
            u64::MAX,
            0,
            MutationIntentStatus::Confirmed,
        ))
        .unwrap();
    assert!(epoch_store
        .try_begin(serialized_intent_record(
            owner,
            POLYGON_CHAIN_ID,
            0,
            0,
            MutationIntentStatus::Preparing,
        ))
        .unwrap_err()
        .to_string()
        .contains("epoch exhausted"));

    let revision_store = InMemoryMutationIntentStore::default();
    let max_revision = serialized_intent_record(
        owner,
        POLYGON_CHAIN_ID,
        7,
        u64::MAX,
        MutationIntentStatus::Submitted,
    );
    revision_store
        .seed_for_test(max_revision.clone())
        .unwrap();
    assert!(revision_store
        .update(7, u64::MAX, max_revision)
        .unwrap_err()
        .to_string()
        .contains("revision exhausted"));
}

#[test]
fn mutation_intent_restart_recovery_uses_registry_terminal_failure_without_rebuilding_lease() {
    let store = Arc::new(InMemoryMutationIntentStore::default());
    let owner = address(WALLET_OWNER);
    {
        let registry = intent_registry(store.clone());
        let mut lease = registry
            .begin_intent(
                owner,
                POLYGON_CHAIN_ID,
                RelayerMutationOperation::WalletBatch,
            )
            .unwrap();
        lease.record_submitted("tx-restart").unwrap();
    }

    let restarted = intent_registry(store);
    assert!(expect_intent_begin_error(restarted.begin_intent(
            owner,
            POLYGON_CHAIN_ID,
            RelayerMutationOperation::WalletBatch,
        ))
        .is_deposit_wallet_mutation_blocked());
    restarted
        .record_terminal_failure(
            owner,
            POLYGON_CHAIN_ID,
            "tx-stale-delivery",
            &RelayerError::TransactionInvalid("synthetic".to_string()),
        )
        .unwrap();
    assert_eq!(
        restarted
            .intent(owner, POLYGON_CHAIN_ID)
            .unwrap()
            .unwrap()
            .status(),
        MutationIntentStatus::Submitted
    );
    restarted
        .record_terminal_failure(
            owner,
            POLYGON_CHAIN_ID,
            "tx-restart",
            &RelayerError::TransactionInvalid("synthetic".to_string()),
        )
        .unwrap();
    let failed = restarted
        .intent(owner, POLYGON_CHAIN_ID)
        .unwrap()
        .unwrap();
    assert_eq!(failed.status(), MutationIntentStatus::Failed);
    assert_eq!(failed.last_observed_state(), Some("Invalid"));
    let successor = restarted
        .begin_intent(
            owner,
            POLYGON_CHAIN_ID,
            RelayerMutationOperation::WalletBatch,
        )
        .unwrap();
    assert_eq!(
        restarted
            .intent(owner, POLYGON_CHAIN_ID)
            .unwrap()
            .unwrap()
            .epoch(),
        1
    );
    drop(successor);
}

#[test]
fn mutation_intent_store_errors_fail_closed_and_post_submit_recording_failure_keeps_lock() {
    let owner = address(WALLET_OWNER);
    let failing_registry = intent_registry(Arc::new(FailingBeginMutationIntentStore));
    let error = expect_intent_begin_error(failing_registry.begin_intent(
            owner,
            POLYGON_CHAIN_ID,
            RelayerMutationOperation::WalletBatch,
        ));
    assert!(error
        .to_string()
        .contains("synthetic mutation intent begin failure"));

    let update_store = Arc::new(FailingSecondUpdateMutationIntentStore {
        inner: InMemoryMutationIntentStore::default(),
    });
    let update_registry = intent_registry(update_store);
    let mut lease = update_registry
        .begin_intent(
            owner,
            POLYGON_CHAIN_ID,
            RelayerMutationOperation::WalletBatch,
        )
        .unwrap();
    lease
        .record_payload(&test_payload_keccak256(), None)
        .unwrap();
    let error = lease.record_submitted("tx-store-failure").unwrap_err();
    assert!(error
        .to_string()
        .contains("synthetic mutation intent update failure"));
    assert!(!error.to_string().contains("tx-store-failure"));
    assert_eq!(
        update_registry
            .intent(owner, POLYGON_CHAIN_ID)
            .unwrap()
            .unwrap()
            .status(),
        MutationIntentStatus::Preparing
    );
    assert_eq!(
        lease
            .record_submitted("tx-store-failure")
            .unwrap_err()
            .to_string(),
        STALE_INTENT_LEASE_ERROR
    );
    assert!(expect_intent_begin_error(update_registry.begin_intent(
            owner,
            POLYGON_CHAIN_ID,
            RelayerMutationOperation::WalletBatch,
        ))
        .is_deposit_wallet_mutation_blocked());
}

#[tokio::test]
async fn intent_gated_execute_records_submission_then_confirmed_poll_and_reopens_owner() {
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
            json!({"transactionID": "tx-gated-execute", "state": "STATE_NEW"}).to_string(),
        ),
    ])
    .await;
    let client = mutation_test_client(url, FIXED_NOW_UNIX, true);
    let store = Arc::new(InMemoryMutationIntentStore::default());
    let registry = intent_registry(store);
    let permit = execute_permit(RelayerMutationMode::Live, owner);

    let receipt = expect_submitted(
        registry
            .gate(&client)
            .execute_wallet_batch(
                ctx,
                calls,
                deadline,
                &signer,
                &read_permit(owner),
                &permit,
            )
            .await
            .unwrap(),
    );
    assert_eq!(receipt.transaction_id(), "tx-gated-execute");
    assert_eq!(receipt.payload_keccak256(), expected_hash);
    let submitted = registry
        .intent(owner, POLYGON_CHAIN_ID)
        .unwrap()
        .unwrap();
    assert_eq!(submitted.status(), MutationIntentStatus::Submitted);
    assert_eq!(submitted.payload_keccak256(), Some(expected_hash.as_str()));
    assert_eq!(submitted.deadline_unix(), Some(EXECUTE_DEADLINE_UNIX));
    assert_eq!(submitted.transaction_id(), Some("tx-gated-execute"));
    assert_eq!(submitted.nonce(), None);

    registry
        .record_poll_outcome(
            owner,
            POLYGON_CHAIN_ID,
            "tx-gated-execute",
            &RelayerPollOutcome::Confirmed(intent_receipt(
                "tx-gated-execute",
                RelayerTransactionState::Confirmed,
            )),
        )
        .unwrap();
    assert_eq!(
        registry
            .intent(owner, POLYGON_CHAIN_ID)
            .unwrap()
            .unwrap()
            .status(),
        MutationIntentStatus::Confirmed
    );
    let successor = registry
        .begin_intent(
            owner,
            POLYGON_CHAIN_ID,
            RelayerMutationOperation::WalletBatch,
        )
        .unwrap();
    drop(successor);

    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].method, "GET");
    assert!(requests[0].path.starts_with("/nonce?"));
    assert_eq!(requests[1].method, "POST");
    assert_eq!(requests[1].path, SUBMIT_PATH);
    assert_eq!(
        serde_json::from_str::<Value>(&requests[1].body).unwrap(),
        serde_json::to_value(expected_request).unwrap()
    );
}

#[tokio::test]
async fn intent_gated_execute_rejects_existing_owner_before_nonce_or_http() {
    let signer = execute_signer();
    let owner = signer.address();
    let (url, handle) = spawn_optional_request_server().await;
    let client = mutation_test_client(url, FIXED_NOW_UNIX, true);
    let store = Arc::new(InMemoryMutationIntentStore::default());
    let registry = intent_registry(store);
    let existing = registry
        .begin_intent(
            owner,
            POLYGON_CHAIN_ID,
            RelayerMutationOperation::WalletBatch,
        )
        .unwrap();

    let error = registry
        .gate(&client)
        .execute_wallet_batch(
            execute_context(owner),
            execute_calls(),
            U256::from(EXECUTE_DEADLINE_UNIX),
            &signer,
            &read_permit(owner),
            &execute_permit(RelayerMutationMode::Live, owner),
        )
        .await
        .unwrap_err();
    assert!(error.is_deposit_wallet_mutation_blocked());
    assert!(error.to_string().contains("status Preparing"));
    assert!(handle.await.unwrap().is_empty());
    drop(existing);
}

#[tokio::test]
async fn intent_gated_execute_maps_disconnect_to_ambiguous_and_local_deadline_failure_to_failed() {
    let signer = execute_signer();
    let owner = signer.address();
    let (url, handle) = spawn_nonce_then_reset_server(TestResponse::json(
        "200 OK",
        json!({"nonce": "31"}).to_string(),
    ))
    .await;
    let client = mutation_test_client(url, FIXED_NOW_UNIX, true);
    let store = Arc::new(InMemoryMutationIntentStore::default());
    let registry = intent_registry(store);

    let error = registry
        .gate(&client)
        .execute_wallet_batch(
            execute_context(owner),
            execute_calls(),
            U256::from(EXECUTE_DEADLINE_UNIX),
            &signer,
            &read_permit(owner),
            &execute_permit(RelayerMutationMode::Live, owner),
        )
        .await
        .unwrap_err();
    assert!(error.is_deposit_wallet_reconciliation_required());
    let ambiguous = registry
        .intent(owner, POLYGON_CHAIN_ID)
        .unwrap()
        .unwrap();
    assert_eq!(ambiguous.status(), MutationIntentStatus::AmbiguousNoId);
    assert!(registry
        .begin_intent(
            owner,
            POLYGON_CHAIN_ID,
            RelayerMutationOperation::WalletBatch,
        )
        .is_err());
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].method, "GET");
    assert_eq!(requests[1].method, "POST");

    let (url, handle) = spawn_optional_request_server().await;
    let client = mutation_test_client(url, FIXED_NOW_UNIX, true);
    let store = Arc::new(InMemoryMutationIntentStore::default());
    let registry = intent_registry(store);
    let error = registry
        .gate(&client)
        .execute_wallet_batch(
            execute_context(owner),
            execute_calls(),
            U256::from(FIXED_NOW_UNIX),
            &signer,
            &read_permit(owner),
            &execute_permit(RelayerMutationMode::Live, owner),
        )
        .await
        .unwrap_err();
    assert!(error.is_deposit_wallet_mutation_blocked());
    let failed = registry
        .intent(owner, POLYGON_CHAIN_ID)
        .unwrap()
        .unwrap();
    assert_eq!(failed.status(), MutationIntentStatus::Failed);
    assert_eq!(failed.last_observed_state(), None);
    assert!(registry
        .begin_intent(
            owner,
            POLYGON_CHAIN_ID,
            RelayerMutationOperation::WalletBatch,
        )
        .is_ok());
    assert!(handle.await.unwrap().is_empty());
}

#[tokio::test]
async fn intent_gated_dry_run_reads_nonce_without_creating_a_lease() {
    let signer = execute_signer();
    let owner = signer.address();
    let (url, handle) = spawn_server(vec![TestResponse::json(
        "200 OK",
        json!({"nonce": "31"}).to_string(),
    )])
    .await;
    let client = mutation_test_client(url, FIXED_NOW_UNIX, false);
    let store = Arc::new(InMemoryMutationIntentStore::default());
    let registry = intent_registry(store);

    let evidence = expect_dry_run(
        registry
            .gate(&client)
            .execute_wallet_batch(
                execute_context(owner),
                execute_calls(),
                U256::from(EXECUTE_DEADLINE_UNIX),
                &signer,
                &read_permit(owner),
                &execute_permit(RelayerMutationMode::DryRun, owner),
            )
            .await
            .unwrap(),
    );
    assert_eq!(evidence.nonce(), Some("31"));
    assert!(registry
        .intent(owner, POLYGON_CHAIN_ID)
        .unwrap()
        .is_none());
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "GET");
}

#[tokio::test]
async fn intent_gated_create_and_lifecycle_dry_runs_never_create_a_lease() {
    let owner = address(WALLET_OWNER);
    let dry_run_permit = mutation_permit(
        RelayerMutationMode::DryRun,
        RelayerMutationOperation::WalletCreate,
        owner,
        POLYGON_CHAIN_ID,
        FIXED_PERMIT_EXPIRY_UNIX,
    );

    let (url, handle) = spawn_optional_request_server().await;
    let client = mutation_test_client(url, FIXED_NOW_UNIX, false);
    let submit_registry = intent_registry(Arc::new(InMemoryMutationIntentStore::default()));
    let evidence = expect_dry_run(
        submit_registry
            .gate(&client)
            .submit_wallet_create(owner, &dry_run_permit)
            .await
            .unwrap(),
    );
    assert_eq!(evidence.operation(), WALLET_CREATE_TRANSACTION_TYPE);
    assert!(submit_registry
        .intent(owner, POLYGON_CHAIN_ID)
        .unwrap()
        .is_none());
    assert!(handle.await.unwrap().is_empty());

    let (url, handle) = spawn_server(vec![
        TestResponse::json("200 OK", json!({"deployed": false}).to_string()),
        TestResponse::json("200 OK", json!({"deployed": false}).to_string()),
    ])
    .await;
    let client = mutation_test_client(url, FIXED_NOW_UNIX, false);
    let lifecycle_registry = intent_registry(Arc::new(InMemoryMutationIntentStore::default()));
    let status = lifecycle_registry
        .gate(&client)
        .ensure_deposit_wallet_deployment(
            owner,
            DepositWalletDeploymentPolicy::DeployIfMissing,
            &read_permit(owner),
            Some(&dry_run_permit),
        )
        .await
        .unwrap();
    assert!(matches!(
        status,
        DepositWalletDeploymentStatus::CreateDryRun(_)
    ));
    assert!(lifecycle_registry
        .intent(owner, POLYGON_CHAIN_ID)
        .unwrap()
        .is_none());
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 2);
    assert_deployed_request(&requests[0]);
    assert_deployed_request(&requests[1]);
}

#[tokio::test]
async fn intent_gated_invalid_transaction_id_is_ambiguous_without_echoing_raw_identity() {
    let owner = address(WALLET_OWNER);
    let raw_invalid_transaction_id = " sensitive-transaction-token ";
    let (url, handle) = spawn_server(vec![TestResponse::json(
        "200 OK",
        json!({
            "transactionID": raw_invalid_transaction_id,
            "state": "STATE_NEW"
        })
        .to_string(),
    )])
    .await;
    let client = mutation_test_client(url, FIXED_NOW_UNIX, true);
    let registry = intent_registry(Arc::new(InMemoryMutationIntentStore::default()));
    let permit = mutation_permit(
        RelayerMutationMode::Live,
        RelayerMutationOperation::WalletCreate,
        owner,
        POLYGON_CHAIN_ID,
        FIXED_PERMIT_EXPIRY_UNIX,
    );

    let error = registry
        .gate(&client)
        .submit_wallet_create(owner, &permit)
        .await
        .unwrap_err();
    assert!(error.is_deposit_wallet_reconciliation_required());
    assert!(!error.to_string().contains(raw_invalid_transaction_id));
    let record = registry
        .intent(owner, POLYGON_CHAIN_ID)
        .unwrap()
        .unwrap();
    assert_eq!(record.status(), MutationIntentStatus::AmbiguousNoId);
    assert_eq!(record.transaction_id(), None);
    assert_eq!(handle.await.unwrap().len(), 1);
}

#[tokio::test]
async fn intent_gated_api_failures_use_conservative_ambiguous_phase_classification() {
    let owner = execute_signer().address();
    let create_permit = mutation_permit(
        RelayerMutationMode::Live,
        RelayerMutationOperation::WalletCreate,
        owner,
        POLYGON_CHAIN_ID,
        FIXED_PERMIT_EXPIRY_UNIX,
    );

    let (url, handle) = spawn_server(vec![TestResponse::json(
        "503 Service Unavailable",
        "{}",
    )])
    .await;
    let client = mutation_test_client(url, FIXED_NOW_UNIX, true);
    let create_registry = intent_registry(Arc::new(InMemoryMutationIntentStore::default()));
    let create_error = create_registry
        .gate(&client)
        .submit_wallet_create(owner, &create_permit)
        .await
        .unwrap_err();
    assert!(matches!(
        create_error,
        RelayerError::Api { status: 503, .. }
    ));
    assert_eq!(
        create_registry
            .intent(owner, POLYGON_CHAIN_ID)
            .unwrap()
            .unwrap()
            .status(),
        MutationIntentStatus::AmbiguousNoId
    );
    assert!(create_registry
        .begin_intent(
            owner,
            POLYGON_CHAIN_ID,
            RelayerMutationOperation::WalletCreate,
        )
        .is_err());
    assert_eq!(handle.await.unwrap().len(), 1);

    let signer = execute_signer();
    let (url, handle) = spawn_server(vec![TestResponse::json(
        "503 Service Unavailable",
        "{}",
    )])
    .await;
    let client = mutation_test_client(url, FIXED_NOW_UNIX, true);
    let nonce_registry = intent_registry(Arc::new(InMemoryMutationIntentStore::default()));
    let nonce_error = nonce_registry
        .gate(&client)
        .execute_wallet_batch(
            execute_context(owner),
            execute_calls(),
            U256::from(EXECUTE_DEADLINE_UNIX),
            &signer,
            &read_permit(owner),
            &execute_permit(RelayerMutationMode::Live, owner),
        )
        .await
        .unwrap_err();
    assert!(matches!(nonce_error, RelayerError::Api { status: 503, .. }));
    assert_eq!(
        nonce_registry
            .intent(owner, POLYGON_CHAIN_ID)
            .unwrap()
            .unwrap()
            .status(),
        MutationIntentStatus::AmbiguousNoId
    );
    assert!(nonce_registry
        .begin_intent(
            owner,
            POLYGON_CHAIN_ID,
            RelayerMutationOperation::WalletBatch,
        )
        .is_err());
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "GET");

    let (url, handle) = spawn_server(vec![
        TestResponse::json("200 OK", json!({"nonce": "31"}).to_string()),
        TestResponse::json("503 Service Unavailable", "{}"),
    ])
    .await;
    let client = mutation_test_client(url, FIXED_NOW_UNIX, true);
    let submit_registry = intent_registry(Arc::new(InMemoryMutationIntentStore::default()));
    let submit_error = submit_registry
        .gate(&client)
        .execute_wallet_batch(
            execute_context(owner),
            execute_calls(),
            U256::from(EXECUTE_DEADLINE_UNIX),
            &signer,
            &read_permit(owner),
            &execute_permit(RelayerMutationMode::Live, owner),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        submit_error,
        RelayerError::Api { status: 503, .. }
    ));
    assert_eq!(
        submit_registry
            .intent(owner, POLYGON_CHAIN_ID)
            .unwrap()
            .unwrap()
            .status(),
        MutationIntentStatus::AmbiguousNoId
    );
    assert!(submit_registry
        .begin_intent(
            owner,
            POLYGON_CHAIN_ID,
            RelayerMutationOperation::WalletBatch,
        )
        .is_err());
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].method, "GET");
    assert_eq!(requests[1].method, "POST");
}

#[tokio::test]
async fn intent_gated_create_and_lifecycle_wrappers_preserve_outcomes_and_lease_policy() {
    let owner = address(WALLET_OWNER);
    let create_permit = mutation_permit(
        RelayerMutationMode::Live,
        RelayerMutationOperation::WalletCreate,
        owner,
        POLYGON_CHAIN_ID,
        FIXED_PERMIT_EXPIRY_UNIX,
    );

    let (url, handle) = spawn_server(vec![TestResponse::json(
        "200 OK",
        json!({"transactionID": "tx-gated-create", "state": "STATE_NEW"}).to_string(),
    )])
    .await;
    let client = mutation_test_client(url, FIXED_NOW_UNIX, true);
    let create_registry = intent_registry(Arc::new(InMemoryMutationIntentStore::default()));
    let receipt = expect_submitted(
        create_registry
            .gate(&client)
            .submit_wallet_create(owner, &create_permit)
            .await
            .unwrap(),
    );
    let create_record = create_registry
        .intent(owner, POLYGON_CHAIN_ID)
        .unwrap()
        .unwrap();
    assert_eq!(create_record.status(), MutationIntentStatus::Submitted);
    assert_eq!(create_record.operation(), RelayerMutationOperation::WalletCreate);
    assert_eq!(create_record.transaction_id(), Some("tx-gated-create"));
    assert_eq!(create_record.deadline_unix(), None);
    assert_eq!(
        create_record.payload_keccak256(),
        Some(receipt.payload_keccak256())
    );
    create_registry
        .record_terminal_failure(
            owner,
            POLYGON_CHAIN_ID,
            "tx-delayed-other",
            &RelayerError::TransactionFailed("synthetic".to_string()),
        )
        .unwrap();
    assert_eq!(
        create_registry
            .intent(owner, POLYGON_CHAIN_ID)
            .unwrap()
            .unwrap()
            .status(),
        MutationIntentStatus::Submitted
    );
    let reconciliation = RelayerError::reconciliation_required("synthetic unknown outcome");
    assert!(create_registry
        .record_terminal_failure(
            owner,
            POLYGON_CHAIN_ID,
            "tx-gated-create",
            &reconciliation,
        )
        .is_err());
    assert_eq!(
        create_registry
            .intent(owner, POLYGON_CHAIN_ID)
            .unwrap()
            .unwrap()
            .status(),
        MutationIntentStatus::Submitted
    );
    create_registry
        .record_terminal_failure(
            owner,
            POLYGON_CHAIN_ID,
            "tx-gated-create",
            &RelayerError::TransactionFailed("synthetic".to_string()),
        )
        .unwrap();
    assert_eq!(
        create_registry
            .intent(owner, POLYGON_CHAIN_ID)
            .unwrap()
            .unwrap()
            .status(),
        MutationIntentStatus::Failed
    );
    let next = create_registry
        .begin_intent(
            owner,
            POLYGON_CHAIN_ID,
            RelayerMutationOperation::WalletCreate,
        )
        .unwrap();
    drop(next);
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "POST");

    let (url, handle) = spawn_server(vec![TestResponse::json(
        "200 OK",
        json!({"deployed": true}).to_string(),
    )])
    .await;
    let client = mutation_test_client(url, FIXED_NOW_UNIX, true);
    let already_registry = intent_registry(Arc::new(InMemoryMutationIntentStore::default()));
    let status = already_registry
        .gate(&client)
        .ensure_deposit_wallet_deployment(
            owner,
            DepositWalletDeploymentPolicy::DeployIfMissing,
            &read_permit(owner),
            Some(&create_permit),
        )
        .await
        .unwrap();
    assert_eq!(status, DepositWalletDeploymentStatus::AlreadyDeployed);
    assert!(already_registry
        .intent(owner, POLYGON_CHAIN_ID)
        .unwrap()
        .is_none());
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_deployed_request(&requests[0]);

    let (url, handle) = spawn_server(vec![
        TestResponse::json("200 OK", json!({"deployed": false}).to_string()),
        TestResponse::json("200 OK", json!({"deployed": false}).to_string()),
    ])
    .await;
    let client = mutation_test_client(url, FIXED_NOW_UNIX, false);
    let predeployed_registry = intent_registry(Arc::new(InMemoryMutationIntentStore::default()));
    let error = predeployed_registry
        .gate(&client)
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
    assert!(predeployed_registry
        .intent(owner, POLYGON_CHAIN_ID)
        .unwrap()
        .is_none());
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 2);
    assert_deployed_request(&requests[0]);
    assert_deployed_request(&requests[1]);

    let (url, handle) = spawn_server(vec![
        TestResponse::json("200 OK", json!({"deployed": false}).to_string()),
        TestResponse::json("200 OK", json!({"deployed": false}).to_string()),
        TestResponse::json(
            "200 OK",
            json!({"transactionID": "tx-gated-lifecycle", "state": "STATE_NEW"}).to_string(),
        ),
    ])
    .await;
    let client = mutation_test_client(url, FIXED_NOW_UNIX, true);
    let lifecycle_registry = intent_registry(Arc::new(InMemoryMutationIntentStore::default()));
    let status = lifecycle_registry
        .gate(&client)
        .ensure_deposit_wallet_deployment(
            owner,
            DepositWalletDeploymentPolicy::DeployIfMissing,
            &read_permit(owner),
            Some(&create_permit),
        )
        .await
        .unwrap();
    let receipt = match status {
        DepositWalletDeploymentStatus::CreateSubmitted(receipt) => receipt,
        other => panic!("expected intent-gated lifecycle submit, got {other:?}"),
    };
    assert_eq!(receipt.transaction_id(), "tx-gated-lifecycle");
    let lifecycle_record = lifecycle_registry
        .intent(owner, POLYGON_CHAIN_ID)
        .unwrap()
        .unwrap();
    assert_eq!(lifecycle_record.status(), MutationIntentStatus::Submitted);
    assert_eq!(
        lifecycle_record.transaction_id(),
        Some("tx-gated-lifecycle")
    );
    assert_eq!(lifecycle_record.deadline_unix(), None);
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 3);
    assert_deployed_request(&requests[0]);
    assert_deployed_request(&requests[1]);
    assert_eq!(requests[2].method, "POST");
    assert_eq!(requests[2].path, SUBMIT_PATH);
}

#[test]
fn mutation_intent_serialization_and_debug_are_secret_free_and_redacted() {
    let owner = address(WALLET_OWNER);
    let store = Arc::new(InMemoryMutationIntentStore::default());
    let registry = intent_registry(store);
    let mut lease = registry
        .begin_intent(
            owner,
            POLYGON_CHAIN_ID,
            RelayerMutationOperation::WalletBatch,
        )
        .unwrap();
    lease
        .record_payload(&test_payload_keccak256(), Some(U256::from(123u64)))
        .unwrap();
    lease.record_submitted("tx-secret-free").unwrap();
    let record = registry
        .intent(owner, POLYGON_CHAIN_ID)
        .unwrap()
        .unwrap();
    let serialized = serde_json::to_string(&record).unwrap();
    let debug = format!("{record:?}");
    let fixture = fixture_value("wallet_submit_body.json");
    let raw_signature = fixture["signature"].as_str().unwrap();
    let raw_calldata = fixture["depositWalletParams"]["calls"][0]["data"]
        .as_str()
        .unwrap();
    let synthetic_private_key = format!("0x{}", hex::encode(SYNTHETIC_EXECUTE_SIGNER_KEY));

    for forbidden in [
        raw_signature,
        raw_calldata,
        API_KEY,
        "RELAYER_API_KEY",
        synthetic_private_key.as_str(),
    ] {
        assert!(!serialized.contains(forbidden));
        assert!(!debug.contains(forbidden));
    }
    assert!(serialized.contains(&test_payload_keccak256()));
    assert!(serialized.contains("tx-secret-free"));
    assert!(!debug.contains(WALLET_OWNER));
    assert!(!debug.contains("tx-secret-free"));
    assert!(debug.contains(&super::redaction::redacted_address(owner)));
    assert!(debug.contains("sha3:0x"));

    let before = record;
    let error = lease
        .record_observed_receipt(&intent_receipt(
            "tx-secret-free",
            RelayerTransactionState::Unknown(API_KEY.to_string()),
        ))
        .unwrap_err();
    assert!(!error.to_string().contains(API_KEY));
    assert_eq!(
        registry.intent(owner, POLYGON_CHAIN_ID).unwrap().unwrap(),
        before
    );
}

#[tokio::test]
async fn intent_gated_submit_recording_failure_returns_store_error_and_leaves_owner_locked() {
    let owner = address(WALLET_OWNER);
    let (url, handle) = spawn_server(vec![TestResponse::json(
        "200 OK",
        json!({"transactionID": "tx-accepted-store-failed", "state": "STATE_NEW"}).to_string(),
    )])
    .await;
    let client = mutation_test_client(url, FIXED_NOW_UNIX, true);
    let store = Arc::new(FailingSecondUpdateMutationIntentStore {
        inner: InMemoryMutationIntentStore::default(),
    });
    let registry = intent_registry(store);
    let permit = mutation_permit(
        RelayerMutationMode::Live,
        RelayerMutationOperation::WalletCreate,
        owner,
        POLYGON_CHAIN_ID,
        FIXED_PERMIT_EXPIRY_UNIX,
    );

    let error = registry
        .gate(&client)
        .submit_wallet_create(owner, &permit)
        .await
        .unwrap_err();
    assert!(error
        .to_string()
        .contains("synthetic mutation intent update failure"));
    assert!(!error.to_string().contains("tx-accepted-store-failed"));
    let record = registry
        .intent(owner, POLYGON_CHAIN_ID)
        .unwrap()
        .unwrap();
    assert_eq!(record.status(), MutationIntentStatus::Preparing);
    assert!(record.payload_keccak256().is_some());
    assert_eq!(record.transaction_id(), None);
    assert!(registry
        .begin_intent(
            owner,
            POLYGON_CHAIN_ID,
            RelayerMutationOperation::WalletCreate,
        )
        .is_err());
    let requests = handle.await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "POST");
}
