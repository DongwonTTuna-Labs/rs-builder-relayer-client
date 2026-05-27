    use std::collections::VecDeque;
    use std::io::ErrorKind;
    use std::sync::Arc;

    use ethers::types::Bytes;
    use serde_json::{json, Value};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::oneshot;
    use tokio::task::JoinHandle;

    use crate::auth::{AuthMethod, BuilderConfig};
    use crate::deposit_wallet::{
        deposit_wallet_contract_config, validate_deposit_wallet_batch_signature,
        DepositWalletBatchToSign, DepositWalletCall,
    };

    use super::*;
    use super::redaction::{recovered_payload_hash, signed_digest_payload_hash};
    use super::response::{parse_transaction_response, validate_transaction_id};
    use super::state::{
        OwnerMutationBlock, OwnerMutationState, OwnerTransactionRecord,
        OwnerTransactionSource, OwnerTransactionTerminalObservation,
    };
    use super::transport::retry_after_duration_at;

    const API_KEY: &str = "unit-test-relayer-api-key";
    const WALLET_CREATE_OWNER: &str = "0x6e0c80c90ea6c15917308F820Eac91Ce2724B5b5";
    const API_KEY_ADDRESS: &str = "0xA6Db23622C9EA7584D5c61C3e7497c80E2CE167B";
    const INVALID_RELAYER_URL_PREFIX: &str = "Invalid relayer URL:";
    const MUTATION_BLOCKED_PREFIX: &str = "Deposit-wallet mutation blocked:";
    const AMBIGUOUS_SUBMIT_PREFIX: &str = "Ambiguous deposit-wallet submit:";
    const RECONCILIATION_REQUIRED_PREFIX: &str = "Deposit-wallet reconciliation required:";
    const TEST_SERVER_TIMEOUT: Duration = Duration::from_secs(2);

    #[derive(Clone)]
    struct FixedClock {
        now: u64,
    }

    impl DepositWalletClock for FixedClock {
        fn now_unix_seconds(&self) -> u64 {
            self.now
        }
    }

    struct SequenceClock {
        values: Mutex<VecDeque<u64>>,
    }

    impl SequenceClock {
        fn new(values: impl IntoIterator<Item = u64>) -> Self {
            Self {
                values: Mutex::new(values.into_iter().collect()),
            }
        }
    }

    impl DepositWalletClock for SequenceClock {
        fn now_unix_seconds(&self) -> u64 {
            let mut values = self.values.lock().unwrap();
            if values.len() > 1 {
                values.pop_front().unwrap()
            } else {
                *values.front().unwrap()
            }
        }
    }

    #[derive(Default)]
    struct RecordingSleeper {
        sleeps: Mutex<Vec<Duration>>,
    }

    impl RecordingSleeper {
        fn sleeps(&self) -> Vec<Duration> {
            self.sleeps.lock().unwrap().clone()
        }
    }

    impl DepositWalletSleeper for RecordingSleeper {
        fn sleep<'a>(
            &'a self,
            duration: Duration,
        ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
            self.sleeps.lock().unwrap().push(duration);
            Box::pin(async {})
        }
    }

    #[derive(Default)]
    struct ClearingSleeper {
        sleeps: Mutex<Vec<Duration>>,
        state: Mutex<Option<Arc<Mutex<OwnerMutationState>>>>,
    }

    impl ClearingSleeper {
        fn attach_state(&self, state: Arc<Mutex<OwnerMutationState>>) {
            *self.state.lock().unwrap() = Some(state);
        }
    }

    impl DepositWalletSleeper for ClearingSleeper {
        fn sleep<'a>(
            &'a self,
            duration: Duration,
        ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
            self.sleeps.lock().unwrap().push(duration);
            let state = self.state.lock().unwrap().clone();
            if let Some(state) = state {
                let mut state = state.lock().unwrap();
                state.owner_blocks.clear();
                state.nonce_reads.clear();
                state.transaction_owners.clear();
            }
            Box::pin(async {})
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

        fn redirect(location: String) -> Self {
            Self {
                status: "307 Temporary Redirect",
                headers: vec![("location".to_string(), location)],
                include_content_length: true,
                body: String::new(),
            }
        }

        fn with_header(mut self, name: &str, value: &str) -> Self {
            self.headers.push((name.to_string(), value.to_string()));
            self
        }
    }

    fn address(raw: &str) -> Address {
        raw.parse().expect("test address should parse")
    }

    fn error_has_prefix(error: &RelayerError, prefix: &str) -> bool {
        matches!(error, RelayerError::Other(message) if message.starts_with(prefix))
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

    fn mutation_permit() -> DepositWalletMutationGate {
        mutation_permit_for(address(WALLET_CREATE_OWNER))
    }

    fn mutation_permit_for(owner: Address) -> DepositWalletMutationGate {
        DepositWalletMutationGate::Permit(mutation_permit_token_for(owner))
    }

    fn mutation_permit_token_for(owner: Address) -> DepositWalletMutationPermit {
        mutation_permit_token_for_action(owner, DepositWalletMutationAction::WalletCreate)
    }

    fn wallet_batch_mutation_permit_for(owner: Address) -> DepositWalletMutationGate {
        DepositWalletMutationGate::Permit(mutation_permit_token_for_action(
            owner,
            DepositWalletMutationAction::WalletBatch,
        ))
    }

    fn owner_recovery_poll_permit_token_for(owner: Address) -> DepositWalletMutationPermit {
        mutation_permit_token_for_action(owner, DepositWalletMutationAction::OwnerRecoveryPoll)
    }

    fn wallet_nonce_read_permit_for(owner: Address) -> DepositWalletMutationGate {
        DepositWalletMutationGate::Permit(mutation_permit_token_for_action(
            owner,
            DepositWalletMutationAction::WalletNonceRead,
        ))
    }

    fn manual_reconciliation_permit_token_for(owner: Address) -> DepositWalletMutationPermit {
        mutation_permit_token_for_action(owner, DepositWalletMutationAction::ManualReconciliation)
    }

    fn mutation_permit_token_for_action(
        owner: Address,
        action: DepositWalletMutationAction,
    ) -> DepositWalletMutationPermit {
        mutation_permit_token_for_scope(owner, mutation_scope(action))
    }

    fn mutation_permit_for_scope(
        owner: Address,
        scope: DepositWalletMutationScope,
    ) -> DepositWalletMutationGate {
        DepositWalletMutationGate::Permit(mutation_permit_token_for_scope(owner, scope))
    }

    fn mutation_permit_token_for_scope(
        owner: Address,
        scope: DepositWalletMutationScope,
    ) -> DepositWalletMutationPermit {
        mutation_permit_token_for_scope_times(owner, scope, 1_699_999_900, 1_700_000_200)
    }

    fn mutation_permit_token_for_scope_times(
        owner: Address,
        scope: DepositWalletMutationScope,
        acquired_at_unix_seconds: u64,
        expires_at_unix_seconds: u64,
    ) -> DepositWalletMutationPermit {
        DepositWalletMutationPermit::from_owner_serialization_evidence(
            "mocked unit-test relayer call",
            owner_serialization_evidence_for_scope_times(
                owner,
                scope,
                acquired_at_unix_seconds,
                expires_at_unix_seconds,
            ),
        )
        .unwrap()
    }

    fn owner_serialization_evidence_for(owner: Address) -> DepositWalletOwnerSerializationEvidence {
        owner_serialization_evidence_for_action(owner, DepositWalletMutationAction::WalletCreate)
    }

    fn owner_serialization_evidence_for_action(
        owner: Address,
        action: DepositWalletMutationAction,
    ) -> DepositWalletOwnerSerializationEvidence {
        owner_serialization_evidence_for_scope(owner, mutation_scope(action))
    }

    fn owner_serialization_evidence_for_scope(
        owner: Address,
        scope: DepositWalletMutationScope,
    ) -> DepositWalletOwnerSerializationEvidence {
        DepositWalletOwnerSerializationEvidence::new(
            owner,
            scope,
            "unit-test owner serialization guard",
            format!("unit-test-owner-lease-{owner:?}"),
            1_699_999_900,
            1_700_000_200,
        )
        .unwrap()
    }

    fn owner_serialization_evidence_for_scope_times(
        owner: Address,
        scope: DepositWalletMutationScope,
        acquired_at_unix_seconds: u64,
        expires_at_unix_seconds: u64,
    ) -> DepositWalletOwnerSerializationEvidence {
        DepositWalletOwnerSerializationEvidence::new(
            owner,
            scope,
            "unit-test owner serialization guard",
            format!("unit-test-owner-lease-{owner:?}"),
            acquired_at_unix_seconds,
            expires_at_unix_seconds,
        )
        .unwrap()
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

    fn unchecked_mutation_permit(
        owner: Address,
        reason: impl Into<String>,
        evidence: DepositWalletOwnerSerializationEvidence,
    ) -> DepositWalletMutationPermit {
        DepositWalletMutationPermit {
            owner,
            reason: reason.into(),
            owner_serialization_evidence: evidence,
        }
    }

    fn submit_reconciliation_evidence_for_payload(
        owner: Address,
        payload_hash: impl Into<String>,
    ) -> DepositWalletSubmitReconciliationEvidence {
        submit_reconciliation_evidence_for_payload_and_transaction(
            owner,
            payload_hash,
            "tx-manual-reconciliation",
        )
    }

    fn submit_reconciliation_evidence_for_payload_and_transaction(
        owner: Address,
        payload_hash: impl Into<String>,
        transaction_id: impl AsRef<str>,
    ) -> DepositWalletSubmitReconciliationEvidence {
        submit_reconciliation_evidence_for_payload_transaction_observation(
            owner,
            payload_hash,
            transaction_id,
            RelayerTransactionState::Failed,
            None::<&str>,
        )
    }

    fn submit_reconciliation_evidence_for_payload_transaction_observation(
        owner: Address,
        payload_hash: impl Into<String>,
        transaction_id: impl AsRef<str>,
        observed_state: RelayerTransactionState,
        transaction_hash: Option<impl AsRef<str>>,
    ) -> DepositWalletSubmitReconciliationEvidence {
        DepositWalletSubmitReconciliationEvidence::new(
            owner,
            mutation_scope(DepositWalletMutationAction::ManualReconciliation),
            "unit-test owner serialization guard",
            payload_hash,
            DepositWalletSubmitReconciliationObservation::new(
                transaction_id,
                observed_state,
                transaction_hash,
                "unit-test manual submit reconciliation",
                1_700_000_001,
            )
            .unwrap(),
        )
        .unwrap()
    }

    fn idless_submit_reconciliation_evidence_for_payload(
        owner: Address,
        payload_hash: impl Into<String>,
    ) -> DepositWalletIdlessSubmitReconciliationEvidence {
        DepositWalletIdlessSubmitReconciliationEvidence::new(
            owner,
            mutation_scope(DepositWalletMutationAction::ManualReconciliation),
            "unit-test owner serialization guard",
            payload_hash,
            "unit-test relayer audit found no accepted transaction for the ambiguous payload",
            1_700_000_001,
        )
        .unwrap()
    }

    fn submit_reconciliation_evidence_for(
        client: &DepositWalletRelayerClient,
        owner: Address,
    ) -> DepositWalletSubmitReconciliationEvidence {
        let payload_hash = client
            .ambiguous_submit_block(owner)
            .expect("test owner should have an ambiguous submit block");
        let transaction_id = {
            let mut state = client.mutation_state().expect("test state should be readable");
            let transaction_id = state
                .transaction_owners
                .iter()
                .find(|(_, record)| record.owner == owner && record.payload_hash == payload_hash)
                .map(|(transaction_id, _)| transaction_id.clone())
                .unwrap_or_else(|| "tx-manual-reconciliation".to_string());
            if state.transaction_owners.contains_key(&transaction_id) {
                state.terminal_observations.insert(
                    transaction_id.clone(),
                    OwnerTransactionTerminalObservation {
                        observed_state: RelayerTransactionState::Failed,
                        transaction_hash: None,
                    },
                );
            }
            transaction_id
        };
        submit_reconciliation_evidence_for_payload_and_transaction(owner, payload_hash, transaction_id)
    }

    fn reqwest_client(timeout: Duration) -> Client {
        Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(timeout)
            .build()
            .expect("test HTTP client should build")
    }

    fn test_client(base_url: DepositWalletRelayerUrl) -> DepositWalletRelayerClient {
        test_client_with_auth_clock_timeout(
            base_url,
            relayer_auth(),
            1_700_000_000,
            Duration::from_secs(2),
        )
    }

    fn test_client_with_auth_clock_timeout(
        base_url: DepositWalletRelayerUrl,
        auth: RelayerKeyAuth,
        now: u64,
        timeout: Duration,
    ) -> DepositWalletRelayerClient {
        let clock: Arc<dyn DepositWalletClock> = Arc::new(FixedClock { now });
        let sleeper: Arc<dyn DepositWalletSleeper> = Arc::new(RecordingSleeper::default());
        DepositWalletRelayerClient::from_parts(
            reqwest_client(timeout),
            base_url,
            auth,
            deposit_wallet_contract_config(137).unwrap(),
            clock,
            sleeper,
        )
    }

    fn test_client_with_sleeper(
        base_url: DepositWalletRelayerUrl,
        sleeper: Arc<RecordingSleeper>,
    ) -> DepositWalletRelayerClient {
        let clock: Arc<dyn DepositWalletClock> = Arc::new(FixedClock { now: 1_700_000_000 });
        let sleeper_trait: Arc<dyn DepositWalletSleeper> = sleeper;
        DepositWalletRelayerClient::from_parts(
            reqwest_client(Duration::from_secs(2)),
            base_url,
            relayer_auth(),
            deposit_wallet_contract_config(137).unwrap(),
            clock,
            sleeper_trait,
        )
    }

    fn signed_wallet_batch() -> SignedDepositWalletBatch {
        let fixture = fixture_value("wallet_batch_eip712.json");
        let calls = fixture["calls"]
            .as_array()
            .unwrap()
            .iter()
            .map(|call| DepositWalletCall {
                target: call["target"].as_str().unwrap().parse().unwrap(),
                value: U256::from_dec_str(call["value"].as_str().unwrap()).unwrap(),
                data: Bytes::from(
                    hex::decode(call["data"].as_str().unwrap().trim_start_matches("0x"))
                        .unwrap(),
                ),
            })
            .collect();
        let batch = DepositWalletBatchToSign {
            owner: fixture["owner"].as_str().unwrap().parse().unwrap(),
            nonce_owner: fixture["nonceOwner"].as_str().unwrap().parse().unwrap(),
            submit_from: fixture["submitFrom"].as_str().unwrap().parse().unwrap(),
            deposit_wallet: fixture["depositWallet"].as_str().unwrap().parse().unwrap(),
            chain_id: fixture["chainId"].as_u64().unwrap(),
            nonce: U256::from_dec_str(fixture["nonce"].as_str().unwrap()).unwrap(),
            deadline: U256::from_dec_str(fixture["deadline"].as_str().unwrap()).unwrap(),
            calls,
        };
        validate_deposit_wallet_batch_signature(
            batch,
            fixture["ownerSignature"].as_str().unwrap(),
        )
        .unwrap()
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

    async fn spawn_truncated_error_body_server(
        status: &'static str,
    ) -> (DepositWalletRelayerUrl, JoinHandle<Vec<CapturedRequest>>) {
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
            let wire = format!(
                "HTTP/1.1 {status}\r\nconnection: close\r\ncontent-type: application/json\r\ncontent-length: 1024\r\n\r\npartial"
            );
            stream
                .write_all(wire.as_bytes())
                .await
                .expect("partial response should write");
            vec![request]
        });

        (
            DepositWalletRelayerUrl::loopback(&format!("http://{addr}")).unwrap(),
            handle,
        )
    }

    async fn spawn_truncated_success_body_server() -> (
        DepositWalletRelayerUrl,
        JoinHandle<Vec<CapturedRequest>>,
    ) {
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
            let wire = "HTTP/1.1 200 OK\r\nconnection: close\r\ncontent-type: application/json\r\ncontent-length: 1024\r\n\r\n{\"nonce\":";
            stream
                .write_all(wire.as_bytes())
                .await
                .expect("partial response should write");
            vec![request]
        });

        (
            DepositWalletRelayerUrl::loopback(&format!("http://{addr}")).unwrap(),
            handle,
        )
    }

    async fn spawn_controlled_error_body_server() -> (
        DepositWalletRelayerUrl,
        JoinHandle<Vec<CapturedRequest>>,
        tokio::sync::oneshot::Receiver<()>,
        tokio::sync::oneshot::Sender<()>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test server should bind");
        let addr = listener.local_addr().unwrap();
        let (headers_sent_tx, headers_sent_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(async move {
            let (mut stream, _) = tokio::time::timeout(TEST_SERVER_TIMEOUT, listener.accept())
                .await
                .expect("server accept should not hang")
                .expect("server should accept");
            let request = read_request(&mut stream).await;
            let wire = "HTTP/1.1 500 Internal Server Error\r\nconnection: close\r\ncontent-type: application/json\r\ncontent-length: 1024\r\n\r\npartial";
            stream
                .write_all(wire.as_bytes())
                .await
                .expect("partial response should write");
            let _ = headers_sent_tx.send(());
            let _ = release_rx.await;
            vec![request]
        });

        (
            DepositWalletRelayerUrl::loopback(&format!("http://{addr}")).unwrap(),
            handle,
            headers_sent_rx,
            release_tx,
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

        let body =
            String::from_utf8(buffer[body_start..body_start + content_length].to_vec()).unwrap();

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

    fn transaction_response(transaction_id: &str, state: &str) -> String {
        json!({
            "transactionID": transaction_id,
            "state": state,
            "transactionHash": "0x38cbfbeae8fffa4e2b187ee5978d3ee9cafc53af0363ed90a35b7ea9016535d8",
            "owner": WALLET_CREATE_OWNER
        })
        .to_string()
    }

    async fn poll_sequence(
        states: &[&str],
        max_attempts: usize,
    ) -> (
        Result<DepositWalletTransactionReceipt>,
        Vec<CapturedRequest>,
        Arc<RecordingSleeper>,
        DepositWalletPollPolicy,
    ) {
        let responses = states
            .iter()
            .map(|state| TestResponse::json("200 OK", transaction_response("tx-123", state)))
            .collect::<Vec<_>>();
        let (url, handle) = spawn_server(responses).await;
        let sleeper = Arc::new(RecordingSleeper::default());
        let client = test_client_with_sleeper(url, sleeper.clone());
        let policy =
            DepositWalletPollPolicy::new(max_attempts, Duration::from_millis(100)).unwrap();
        let result = client.poll_transaction("tx-123", policy.clone()).await;
        let requests = handle.await.unwrap();
        (result, requests, sleeper, policy)
    }

mod endpoint;
mod submit;
mod state;
mod read;
mod poll;
