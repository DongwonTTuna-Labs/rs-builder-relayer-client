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
        OwnerMutationBlock, OwnerMutationState, OwnerTransactionRecord, OwnerTransactionSource,
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
        RelayerKeyAuth::new(API_KEY, address(API_KEY_ADDRESS))
    }

    fn mutation_permit() -> DepositWalletMutationGate {
        mutation_permit_for(address(WALLET_CREATE_OWNER))
    }

    fn mutation_permit_for(owner: Address) -> DepositWalletMutationGate {
        DepositWalletMutationGate::Permit(mutation_permit_token_for(
            owner,
        ))
    }

    fn mutation_permit_token_for(owner: Address) -> DepositWalletMutationPermit {
        DepositWalletMutationPermit::from_owner_serialization_evidence(
            "mocked unit-test relayer call",
            owner_serialization_evidence_for(owner),
        )
        .unwrap()
    }

    fn owner_serialization_evidence_for(owner: Address) -> DepositWalletOwnerSerializationEvidence {
        DepositWalletOwnerSerializationEvidence::new(
            owner,
            "unit-test owner serialization guard",
            format!("unit-test-owner-lease-{owner:?}"),
            1_600_000_000,
            4_000_000_000,
        )
        .unwrap()
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

    #[test]
    fn production_url_rejects_unsafe_endpoints() {
        let invalid = [
            "http://relayer-v2.polymarket.com",
            "https://user@relayer-v2.polymarket.com",
            "https://relayer-v2.polymarket.com?api_key=leak",
            "https://relayer-v2.polymarket.com#fragment",
            "https://relayer-v2.polymarket.com/token-like-path",
            "https://relayer-v2.polymarket.com:444",
            "https://example.com",
            "https://relayer-v2.polymarket.com.evil.example",
        ];

        for raw in invalid {
            let error = DepositWalletRelayerUrl::parse(raw)
                .expect_err("unsafe relayer URL should be rejected");
            assert!(error_has_prefix(&error, INVALID_RELAYER_URL_PREFIX));
        }

        DepositWalletRelayerUrl::parse("https://relayer-v2.polymarket.com").unwrap();
        DepositWalletRelayerUrl::parse("https://relayer-v2.polymarket.com:443").unwrap();
    }

    #[test]
    fn mocked_loopback_url_allows_only_local_mock_servers() {
        for raw in ["http://127.0.0.1:8080", "http://localhost:8080", "https://[::1]:8443"] {
            DepositWalletRelayerUrl::loopback(raw).unwrap();
        }

        for raw in [
            "ftp://127.0.0.1:8080",
            "http://user@127.0.0.1:8080",
            "http://127.0.0.1:8080/path",
            "http://127.0.0.1:8080?token=leak",
            "https://relayer-v2.polymarket.com",
            "http://192.168.0.1:8080",
        ] {
            let error = DepositWalletRelayerUrl::loopback(raw)
                .expect_err("unsafe mock relayer URL should be rejected");
            assert!(error_has_prefix(&error, INVALID_RELAYER_URL_PREFIX));
        }
    }

    #[test]
    fn production_relayer_client_rejects_non_polygon_contract_config() {
        let url = DepositWalletRelayerUrl::parse("https://relayer-v2.polymarket.com").unwrap();
        let amoy_config = deposit_wallet_contract_config(80002).unwrap();
        let error = DepositWalletRelayerClient::new(url, relayer_auth(), amoy_config).unwrap_err();

        assert!(error_has_prefix(&error, INVALID_RELAYER_URL_PREFIX));
        assert!(error.to_string().contains("Polygon"));
    }

    #[test]
    fn auth_debug_redacts_secret_bearing_fields() {
        let auth = relayer_auth();
        let rendered = format!("{auth:?}");
        assert!(!rendered.contains(API_KEY));
        assert!(!rendered.contains(&to_checksum(&address(API_KEY_ADDRESS), None)));
        let headers = auth.headers().unwrap();
        assert!(headers.get("RELAYER_API_KEY").unwrap().is_sensitive());
        assert!(headers
            .get("RELAYER_API_KEY_ADDRESS")
            .unwrap()
            .is_sensitive());
        let rendered_headers = format!("{headers:?}");
        assert!(!rendered_headers.contains(API_KEY));
        assert!(!rendered_headers.contains(&to_checksum(&address(API_KEY_ADDRESS), None)));

        let builder = BuilderConfig {
            key: "builder-key-secret".to_string(),
            secret: "builder-hmac-secret".to_string(),
            passphrase: "builder-passphrase-secret".to_string(),
        };
        let rendered = format!("{:?}", AuthMethod::Builder(builder));
        assert!(!rendered.contains("builder-key-secret"));
        assert!(!rendered.contains("builder-hmac-secret"));
        assert!(!rendered.contains("builder-passphrase-secret"));

        let rendered = format!(
            "{:?}",
            AuthMethod::relayer_key(API_KEY, &to_checksum(&address(API_KEY_ADDRESS), None))
        );
        assert!(!rendered.contains(API_KEY));
        assert!(!rendered.contains(&to_checksum(&address(API_KEY_ADDRESS), None)));
    }

    #[tokio::test]
    async fn api_error_messages_do_not_echo_secret_material() {
        let signed = signed_wallet_batch();
        let body = format!(
            "bad request echoed {} {} Authorization: bearer value",
            API_KEY,
            fixture_value("wallet_batch_eip712.json")["ownerSignature"]
                .as_str()
                .unwrap()
        );
        let (url, handle) =
            spawn_server(vec![TestResponse::json("400 Bad Request", body)]).await;
        let client = test_client(url);

        let error = client.get_wallet_nonce(signed.owner()).await.unwrap_err();
        let rendered = error.to_string();
        assert!(!rendered.contains(API_KEY));
        assert!(!rendered.contains(
            fixture_value("wallet_batch_eip712.json")["ownerSignature"]
                .as_str()
                .unwrap()
        ));
        assert!(!rendered.contains("bearer value"));

        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
    }

    #[tokio::test]
    async fn transport_errors_do_not_echo_full_nonce_url() {
        let (url, handle) = spawn_reset_server().await;
        let client = test_client_with_auth_clock_timeout(
            url,
            relayer_auth(),
            1_700_000_000,
            Duration::from_millis(500),
        );
        let owner = address(WALLET_CREATE_OWNER);

        let error = client.get_wallet_nonce(owner).await.unwrap_err();
        let rendered = error.to_string();

        assert!(!rendered.contains("/nonce"));
        assert!(!rendered.contains(&to_checksum(&owner, None)));
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].path.contains("/nonce?address="));
    }

    #[tokio::test]
    async fn success_body_read_errors_do_not_echo_full_nonce_url() {
        let (url, handle) = spawn_truncated_success_body_server().await;
        let client = test_client_with_auth_clock_timeout(
            url,
            relayer_auth(),
            1_700_000_000,
            Duration::from_millis(100),
        );
        let owner = address(WALLET_CREATE_OWNER);

        let error = client.get_wallet_nonce(owner).await.unwrap_err();
        let rendered = error.to_string();

        assert!(!rendered.contains("/nonce"));
        assert!(!rendered.contains(&to_checksum(&owner, None)));
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].path.contains("/nonce?address="));
    }

    #[tokio::test]
    async fn non_success_error_body_drain_does_not_delay_caller() {
        let (url, handle, headers_sent, release_server) =
            spawn_controlled_error_body_server().await;
        let client = test_client_with_auth_clock_timeout(
            url,
            relayer_auth(),
            1_700_000_000,
            Duration::from_secs(1),
        );
        let owner = address(WALLET_CREATE_OWNER);

        let client_task = tokio::spawn(async move { client.get_wallet_nonce(owner).await });
        headers_sent
            .await
            .expect("server should send non-success headers");

        let result = tokio::time::timeout(TEST_SERVER_TIMEOUT, client_task)
        .await
        .expect("non-success status should return while error body is still held")
        .expect("client task should not panic");
        assert!(matches!(result.unwrap_err(), RelayerError::Api { status: 500, .. }));

        let _ = release_server.send(());
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
    }

    #[tokio::test]
    async fn background_error_body_drain_limit_drops_excess_drains() {
        let (url, handle, headers_sent, release_server) =
            spawn_controlled_error_body_server().await;
        let client = test_client_with_auth_clock_timeout(
            url,
            relayer_auth(),
            1_700_000_000,
            Duration::from_secs(1),
        );
        let _held_permits = client.hold_error_body_drain_permits_for_test();
        let owner = address(WALLET_CREATE_OWNER);

        let client_for_request = client.clone();
        let client_task = tokio::spawn(async move { client_for_request.get_wallet_nonce(owner).await });
        headers_sent
            .await
            .expect("server should send non-success headers");
        let result = tokio::time::timeout(TEST_SERVER_TIMEOUT, client_task)
            .await
            .expect("non-success response should return while all drain permits are held")
            .expect("client task should not panic");
        assert!(matches!(result.unwrap_err(), RelayerError::Api { status: 500, .. }));
        assert_eq!(client.dropped_error_body_drains_for_test(), 1);

        let _ = release_server.send(());
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
    }

    #[tokio::test]
    async fn default_deny_gate_runs_before_auth_or_http() {
        let url = DepositWalletRelayerUrl::loopback("http://127.0.0.1:1").unwrap();
        let bad_auth = RelayerKeyAuth::new("invalid\nheader", address(API_KEY_ADDRESS));
        let client =
            test_client_with_auth_clock_timeout(url, bad_auth, 1_700_000_000, Duration::from_secs(1));

        let error = client
            .submit_wallet_create(address(WALLET_CREATE_OWNER), DepositWalletMutationGate::Deny)
            .await
            .unwrap_err();

        assert!(error_has_prefix(&error, MUTATION_BLOCKED_PREFIX));

        let error = client
            .submit_wallet_create(
                address(WALLET_CREATE_OWNER),
                DepositWalletMutationGate::Permit(unchecked_mutation_permit(
                    address(WALLET_CREATE_OWNER),
                    " ",
                    owner_serialization_evidence_for(address(WALLET_CREATE_OWNER)),
                )),
            )
            .await
            .unwrap_err();

        assert!(error_has_prefix(&error, MUTATION_BLOCKED_PREFIX));

        let error = client
            .submit_wallet_create(
                address(WALLET_CREATE_OWNER),
                DepositWalletMutationGate::Permit(unchecked_mutation_permit(
                    address(WALLET_CREATE_OWNER),
                    "mocked unit-test relayer call",
                    DepositWalletOwnerSerializationEvidence::new(
                        address(WALLET_CREATE_OWNER),
                        "unit-test expired owner serialization guard",
                        "expired-owner-lease",
                        1,
                        2,
                    )
                    .unwrap(),
                )),
            )
            .await
            .unwrap_err();

        assert!(error_has_prefix(&error, MUTATION_BLOCKED_PREFIX));

        let error = client
            .submit_signed_wallet_batch(signed_wallet_batch(), DepositWalletMutationGate::Deny)
            .await
            .unwrap_err();

        assert!(error_has_prefix(&error, MUTATION_BLOCKED_PREFIX));

        let error = client
            .submit_wallet_create(
                address(WALLET_CREATE_OWNER),
                DepositWalletMutationGate::Permit(unchecked_mutation_permit(
                    Address::zero(),
                    "mocked unit-test relayer call",
                    owner_serialization_evidence_for(Address::zero()),
                )),
            )
            .await
            .unwrap_err();

        assert!(error_has_prefix(&error, MUTATION_BLOCKED_PREFIX));

        let signed = signed_wallet_batch();
        let owner = signed.owner();
        let error = client
            .submit_signed_wallet_batch(
                signed,
                DepositWalletMutationGate::Permit(unchecked_mutation_permit(
                    owner,
                    "",
                    owner_serialization_evidence_for(owner),
                )),
            )
            .await
            .unwrap_err();

        assert!(error_has_prefix(&error, MUTATION_BLOCKED_PREFIX));
    }

    #[tokio::test]
    async fn production_submit_uses_gate_before_auth_or_http() {
        let url = DepositWalletRelayerUrl::parse("https://relayer-v2.polymarket.com").unwrap();
        let bad_auth = RelayerKeyAuth::new("invalid\nheader", address(API_KEY_ADDRESS));
        let client =
            test_client_with_auth_clock_timeout(url, bad_auth, 1_700_000_000, Duration::from_secs(1));

        let error = client
            .submit_wallet_create(address(WALLET_CREATE_OWNER), DepositWalletMutationGate::Deny)
            .await
            .unwrap_err();

        assert!(error_has_prefix(&error, MUTATION_BLOCKED_PREFIX));

        let error = client
            .submit_wallet_create(address(WALLET_CREATE_OWNER), mutation_permit())
            .await
            .unwrap_err();

        assert!(matches!(error, RelayerError::AuthError(_)));

        let signed = signed_wallet_batch();
        let owner = signed.owner();
        let error = client
            .submit_signed_wallet_batch(signed, mutation_permit_for(owner))
            .await
            .unwrap_err();

        assert!(matches!(error, RelayerError::AuthError(_)));
    }

    #[tokio::test]
    async fn submit_auth_failure_clears_owner_reservation_after_preflight() {
        let owner = address(WALLET_CREATE_OWNER);
        let url = DepositWalletRelayerUrl::loopback("http://127.0.0.1:1").unwrap();
        let bad_auth = RelayerKeyAuth::new("invalid\nheader", address(API_KEY_ADDRESS));
        let client =
            test_client_with_auth_clock_timeout(url, bad_auth, 1_700_000_000, Duration::from_secs(1));

        let error = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap_err();

        assert!(matches!(error, RelayerError::AuthError(_)));
        assert!(client.ambiguous_submit_block(owner).is_none());
        client.ensure_owner_unblocked(owner).unwrap();
        let mut retry_reservation = client
            .reserve_owner_submit(owner, "payload:retry-after-auth-error".to_string())
            .unwrap();
        retry_reservation.clear().unwrap();
    }

    #[test]
    fn mutation_permit_debug_redacts_approval_evidence() {
        let owner = address(WALLET_CREATE_OWNER);
        let permit = DepositWalletMutationPermit::from_owner_serialization_evidence(
            "ticket-123 caller lock",
            DepositWalletOwnerSerializationEvidence::new(
                owner,
                "unit-test caller lock",
                "owner-lock-key-456",
                1_600_000_000,
                1_800_000_000,
            )
            .unwrap(),
        )
        .unwrap();
        let rendered_permit = format!("{permit:?}");
        let rendered_gate = format!("{:?}", DepositWalletMutationGate::Permit(permit));

        assert!(rendered_permit.contains("DepositWalletMutationPermit"));
        assert!(rendered_permit.contains("<redacted>"));
        assert!(!rendered_permit.contains("ticket-123"));
        assert!(!rendered_permit.contains("owner-lock-key-456"));
        assert!(!rendered_permit.contains("unit-test caller lock"));
        assert!(!rendered_gate.contains("ticket-123"));
        assert!(!rendered_gate.contains("owner-lock-key-456"));
        assert!(!rendered_gate.contains("unit-test caller lock"));
    }

    #[tokio::test]
    async fn get_wallet_nonce_sends_exact_path_and_parses_decimal_nonce() {
        let expected = fixture_value("wallet_nonce_request.json");
        let (url, handle) =
            spawn_server(vec![TestResponse::json("200 OK", json!({"nonce": "31"}).to_string())])
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
    }

    #[tokio::test]
    async fn submit_wallet_create_sends_fixture_body_with_explicit_permit() {
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "200 OK",
            transaction_response("tx-create", "STATE_NEW"),
        )])
        .await;
        let client = test_client(url);

        let receipt = client
            .submit_wallet_create(address(WALLET_CREATE_OWNER), mutation_permit())
            .await
            .unwrap();

        assert_eq!(receipt.transaction_id, "tx-create");
        assert_eq!(receipt.state, RelayerTransactionState::New);
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, "POST");
        assert_eq!(requests[0].path, SUBMIT_PATH);
        assert_eq!(requests[0].header("RELAYER_API_KEY"), Some(API_KEY));
        assert_eq!(
            requests[0].header("content-type"),
            Some("application/json")
        );
        assert_eq!(
            serde_json::from_str::<Value>(&requests[0].body).unwrap(),
            fixture_value("wallet_create_submit_body.json")
        );
    }

    #[tokio::test]
    async fn submit_pending_states_keep_owner_block_until_terminal_poll() {
        let owner = address(WALLET_CREATE_OWNER);
        for state in ["STATE_EXECUTED", "STATE_MINED"] {
            let transaction_id = format!("tx-{}", state.to_ascii_lowercase());
            let (url, handle) = spawn_server(vec![TestResponse::json(
                "200 OK",
                transaction_response(&transaction_id, state),
            )])
            .await;
            let client = test_client(url);

            let receipt = client
                .submit_wallet_create(owner, mutation_permit())
                .await
                .unwrap();

            assert_eq!(receipt.transaction_id, transaction_id);
            assert!(matches!(
                receipt.state,
                RelayerTransactionState::Executed | RelayerTransactionState::Mined
            ));
            {
                let state = client.mutation_state().unwrap();
                match state.owner_blocks.get(&owner) {
                    Some(OwnerMutationBlock::InFlight {
                        transaction_id: Some(blocked_transaction_id),
                        ..
                    }) => assert_eq!(blocked_transaction_id, &transaction_id),
                    block => panic!("expected in-flight owner block, got {block:?}"),
                }
            }
            let blocked = client.get_wallet_nonce(owner).await.unwrap_err();
            assert!(error_has_prefix(&blocked, RECONCILIATION_REQUIRED_PREFIX));
            let requests = handle.await.unwrap();
            assert_eq!(requests.len(), 1);
            assert_eq!(requests[0].path, SUBMIT_PATH);
        }
    }

    #[tokio::test]
    async fn submit_signed_wallet_batch_sends_fixture_body_with_explicit_permit() {
        let signed = signed_wallet_batch();
        let owner = signed.owner();
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "200 OK",
            json!({"nonce": signed.nonce().to_string()}).to_string(),
        ), TestResponse::json(
            "200 OK",
            transaction_response("tx-wallet", "STATE_NEW"),
        )])
        .await;
        let client = test_client(url);

        let receipt = client
            .submit_signed_wallet_batch(signed, mutation_permit_for(owner))
            .await
            .unwrap();

        assert_eq!(receipt.transaction_id, "tx-wallet");
        assert_eq!(receipt.state, RelayerTransactionState::New);
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].method, "GET");
        assert_eq!(
            requests[0].path,
            format!("/nonce?address={}&type=WALLET", to_checksum(&owner, None))
        );
        assert_eq!(requests[0].header("RELAYER_API_KEY"), Some(API_KEY));
        assert_eq!(
            requests[0].header("RELAYER_API_KEY_ADDRESS"),
            Some(to_checksum(&address(API_KEY_ADDRESS), None).as_str())
        );
        assert_ne!(
            requests[0].header("RELAYER_API_KEY_ADDRESS"),
            Some(to_checksum(&owner, None).as_str())
        );
        assert_eq!(requests[1].method, "POST");
        assert_eq!(requests[1].path, SUBMIT_PATH);
        assert_eq!(
            requests[1].header("content-type"),
            Some("application/json")
        );
        assert_eq!(
            serde_json::from_str::<Value>(&requests[1].body).unwrap(),
            fixture_value("wallet_signed_submit_body.json")
        );
    }

    #[tokio::test]
    async fn signed_submit_post_api_failures_record_ambiguous_block() {
        for (status, expected) in [
            ("400 Bad Request", Some(400u16)),
            ("429 Too Many Requests", None),
        ] {
            let signed = signed_wallet_batch();
            let owner = signed.owner();
            let (url, handle) = spawn_server(vec![
                TestResponse::json("200 OK", json!({"nonce": signed.nonce().to_string()}).to_string()),
                TestResponse::json(status, "{}"),
            ])
            .await;
            let client = test_client(url);

            let error = client
                .submit_signed_wallet_batch(signed, mutation_permit_for(owner))
                .await
                .unwrap_err();

            assert!(error_has_prefix(&error, AMBIGUOUS_SUBMIT_PREFIX));
            if let Some(status) = expected {
                assert!(error.to_string().contains(&status.to_string()));
            } else {
                assert!(error.to_string().contains("429"));
            }
            assert!(client.ambiguous_submit_block(owner).is_some());
            let blocked = client
                .submit_signed_wallet_batch(signed_wallet_batch(), mutation_permit_for(owner))
                .await
                .unwrap_err();
            assert!(error_has_prefix(&blocked, RECONCILIATION_REQUIRED_PREFIX));
            let requests = handle.await.unwrap();
            assert_eq!(requests.len(), 2);
            assert_eq!(requests[0].method, "GET");
            assert_eq!(requests[1].method, "POST");
        }
    }

    #[tokio::test]
    async fn signed_submit_post_transport_failure_records_ambiguous_block() {
        let signed = signed_wallet_batch();
        let owner = signed.owner();
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test server should bind");
        let addr = listener.local_addr().unwrap();
        let nonce = signed.nonce();
        let handle = tokio::spawn(async move {
            let mut requests = Vec::new();
            let (mut stream, _) = tokio::time::timeout(TEST_SERVER_TIMEOUT, listener.accept())
                .await
                .expect("server accept should not hang")
                .expect("server should accept");
            requests.push(read_request(&mut stream).await);
            write_response(
                &mut stream,
                TestResponse::json("200 OK", json!({"nonce": nonce.to_string()}).to_string()),
            )
            .await;

            let (mut stream, _) = tokio::time::timeout(TEST_SERVER_TIMEOUT, listener.accept())
                .await
                .expect("server accept should not hang")
                .expect("server should accept");
            requests.push(read_request(&mut stream).await);
            drop(stream);
            requests
        });
        let url = DepositWalletRelayerUrl::loopback(&format!("http://{addr}")).unwrap();
        let client = test_client(url);

        let error = client
            .submit_signed_wallet_batch(signed, mutation_permit_for(owner))
            .await
            .unwrap_err();

        assert!(error_has_prefix(&error, AMBIGUOUS_SUBMIT_PREFIX));
        assert!(client.ambiguous_submit_block(owner).is_some());
        let blocked = client.get_wallet_nonce(owner).await.unwrap_err();
        assert!(error_has_prefix(&blocked, RECONCILIATION_REQUIRED_PREFIX));
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[1].path, SUBMIT_PATH);
    }

    #[tokio::test]
    async fn signed_submit_post_oversized_success_records_ambiguous_block() {
        let signed = signed_wallet_batch();
        let owner = signed.owner();
        let (url, handle) = spawn_server(vec![
            TestResponse::json("200 OK", json!({"nonce": signed.nonce().to_string()}).to_string()),
            TestResponse::json_without_content_length(
                "200 OK",
                "x".repeat(MAX_SUCCESS_BODY_BYTES + 1),
            ),
        ])
        .await;
        let client = test_client(url);

        let error = client
            .submit_signed_wallet_batch(signed, mutation_permit_for(owner))
            .await
            .unwrap_err();

        assert!(error_has_prefix(&error, AMBIGUOUS_SUBMIT_PREFIX));
        assert!(client.ambiguous_submit_block(owner).is_some());
        let blocked = client.get_wallet_nonce(owner).await.unwrap_err();
        assert!(error_has_prefix(&blocked, RECONCILIATION_REQUIRED_PREFIX));
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[1].path, SUBMIT_PATH);
    }

    #[tokio::test]
    async fn submit_signed_wallet_batch_rejects_stale_nonce_before_post() {
        let signed = signed_wallet_batch();
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "200 OK",
            json!({"nonce": (signed.nonce() + U256::one()).to_string()}).to_string(),
        )])
        .await;
        let client = test_client(url);
        let owner = signed.owner();

        let error = client
            .submit_signed_wallet_batch(signed, mutation_permit_for(owner))
            .await
            .unwrap_err();

        assert!(matches!(error, RelayerError::Signing(message) if message.contains("nonce")));
        assert!(client.ambiguous_submit_block(owner).is_none());
        client.ensure_owner_unblocked(owner).unwrap();
        let mut retry_reservation = client
            .reserve_owner_submit(owner, "payload:retry-after-stale-nonce".to_string())
            .unwrap();
        retry_reservation.clear().unwrap();
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, "GET");
    }

    #[tokio::test]
    async fn signed_wallet_local_request_build_failure_clears_owner_reservation() {
        let signed = signed_wallet_batch();
        let owner = signed.owner();
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "200 OK",
            json!({"nonce": signed.nonce().to_string()}).to_string(),
        )])
        .await;
        let clock: Arc<dyn DepositWalletClock> = Arc::new(FixedClock { now: 1_700_000_000 });
        let sleeper: Arc<dyn DepositWalletSleeper> = Arc::new(RecordingSleeper::default());
        let client = DepositWalletRelayerClient::from_parts(
            reqwest_client(Duration::from_secs(2)),
            url,
            relayer_auth(),
            deposit_wallet_contract_config(80002).unwrap(),
            clock,
            sleeper,
        );

        let error = client
            .submit_signed_wallet_batch(signed, mutation_permit_for(owner))
            .await
            .unwrap_err();

        assert!(matches!(error, RelayerError::Signing(_)));
        assert!(client.ambiguous_submit_block(owner).is_none());
        client.ensure_owner_unblocked(owner).unwrap();
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, "GET");
    }

    #[tokio::test]
    async fn signed_wallet_batch_rechecks_deadline_after_nonce_lookup_before_post() {
        let signed = signed_wallet_batch();
        let deadline = signed.deadline().as_u64();
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "200 OK",
            json!({"nonce": signed.nonce().to_string()}).to_string(),
        )])
        .await;
        let clock: Arc<dyn DepositWalletClock> = Arc::new(SequenceClock::new([
            1_700_000_000,
            1_700_000_000,
            deadline,
        ]));
        let sleeper: Arc<dyn DepositWalletSleeper> = Arc::new(RecordingSleeper::default());
        let client = DepositWalletRelayerClient::from_parts(
            reqwest_client(Duration::from_secs(2)),
            url,
            relayer_auth(),
            deposit_wallet_contract_config(137).unwrap(),
            clock,
            sleeper,
        );
        let owner = signed.owner();

        let error = client
            .submit_signed_wallet_batch(signed, mutation_permit_for(owner))
            .await
            .unwrap_err();

        assert!(matches!(error, RelayerError::Signing(message) if message.contains("expired")));
        assert!(client.ambiguous_submit_block(owner).is_none());
        client.ensure_owner_unblocked(owner).unwrap();
        let mut retry_reservation = client
            .reserve_owner_submit(owner, "payload:retry-after-deadline-error".to_string())
            .unwrap();
        retry_reservation.clear().unwrap();
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, "GET");
    }

    #[tokio::test]
    async fn expired_signed_wallet_batch_fails_before_auth_or_http() {
        let url = DepositWalletRelayerUrl::loopback("http://127.0.0.1:1").unwrap();
        let bad_auth = RelayerKeyAuth::new("invalid\nheader", address(API_KEY_ADDRESS));
        let client =
            test_client_with_auth_clock_timeout(url, bad_auth, 2_000_000_000, Duration::from_secs(1));

        let signed = signed_wallet_batch();
        let owner = signed.owner();
        let error = client
            .submit_signed_wallet_batch(signed, mutation_permit_for(owner))
            .await
            .unwrap_err();

        assert!(matches!(error, RelayerError::Signing(message) if message.contains("expired")));
    }

    #[tokio::test]
    async fn partial_submit_response_records_owner_scoped_ambiguous_block_until_cleared() {
        let owner = address(WALLET_CREATE_OWNER);
        let (url, handle) = spawn_server(vec![
            TestResponse::json(
                "200 OK",
                json!({"transactionID": "", "state": "STATE_NEW"}).to_string(),
            ),
            TestResponse::json("200 OK", json!({"nonce": "32"}).to_string()),
        ])
        .await;
        let client = test_client(url);

        let error = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap_err();
        assert!(error_has_prefix(&error, AMBIGUOUS_SUBMIT_PREFIX));
        assert!(client.ambiguous_submit_block(owner).is_some());

        let blocked = client.get_wallet_nonce(owner).await.unwrap_err();
        assert!(error_has_prefix(&blocked, RECONCILIATION_REQUIRED_PREFIX));

        let error = client
            .clear_ambiguous_submit_after_manual_reconciliation(
                owner,
                unchecked_mutation_permit(
                    owner,
                    " ",
                    owner_serialization_evidence_for(owner),
                ),
            )
            .unwrap_err();
        assert!(error_has_prefix(&error, MUTATION_BLOCKED_PREFIX));
        assert!(client.ambiguous_submit_block(owner).is_some());
        let blocked = client.get_wallet_nonce(owner).await.unwrap_err();
        assert!(error_has_prefix(&blocked, RECONCILIATION_REQUIRED_PREFIX));

        let error = client
            .clear_ambiguous_submit_after_manual_reconciliation(
                owner,
                unchecked_mutation_permit(
                    owner,
                    "checked mocked relayer state",
                    DepositWalletOwnerSerializationEvidence::new(
                        owner,
                        "unit-test expired owner serialization guard",
                        "expired-owner-lease-for-clear",
                        1,
                        2,
                    )
                    .unwrap(),
                ),
            )
            .unwrap_err();
        assert!(error_has_prefix(&error, MUTATION_BLOCKED_PREFIX));
        assert!(client.ambiguous_submit_block(owner).is_some());

        let error = client
            .clear_ambiguous_submit_after_manual_reconciliation(
                owner,
                unchecked_mutation_permit(
                    Address::from_low_u64_be(99),
                    "checked mocked relayer state",
                    owner_serialization_evidence_for(Address::from_low_u64_be(99)),
                ),
            )
            .unwrap_err();
        assert!(error_has_prefix(&error, MUTATION_BLOCKED_PREFIX));
        assert!(client.ambiguous_submit_block(owner).is_some());

        client
            .clear_ambiguous_submit_after_manual_reconciliation(
                owner,
                mutation_permit_token_for(owner),
            )
            .unwrap();
        let nonce = client.get_wallet_nonce(owner).await.unwrap();
        assert_eq!(nonce, U256::from(32u64));

        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].path, SUBMIT_PATH);
        assert!(requests[1].path.contains("/nonce?address="));
    }

    #[tokio::test]
    async fn manual_clear_rejects_active_submit_before_response() {
        let owner = address(WALLET_CREATE_OWNER);
        let client = test_client(DepositWalletRelayerUrl::loopback("http://127.0.0.1:1").unwrap());
        let payload_hash = "test-active-submit-payload".to_string();
        let _reservation = client.reserve_owner_submit(owner, payload_hash.clone()).unwrap();

        let error = client
            .clear_ambiguous_submit_after_manual_reconciliation(
                owner,
                mutation_permit_token_for(owner),
            )
            .unwrap_err();

        assert!(error_has_prefix(&error, MUTATION_BLOCKED_PREFIX));
        {
            let state = client.mutation_state().unwrap();
            match state.owner_blocks.get(&owner) {
                Some(OwnerMutationBlock::InFlight {
                    payload_hash: current_payload_hash,
                    transaction_id: None,
                }) => assert_eq!(current_payload_hash, &payload_hash),
                block => panic!("expected active in-flight owner block, got {block:?}"),
            }
        }
        let blocked = client.get_wallet_nonce(owner).await.unwrap_err();
        assert!(error_has_prefix(&blocked, RECONCILIATION_REQUIRED_PREFIX));
    }

    #[tokio::test]
    async fn submit_transport_failure_records_ambiguous_block_and_blocks_duplicate_submit() {
        let owner = address(WALLET_CREATE_OWNER);
        let (url, handle) = spawn_reset_server().await;
        let client = test_client(url);

        let error = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap_err();

        assert!(error_has_prefix(&error, AMBIGUOUS_SUBMIT_PREFIX));
        assert!(client.ambiguous_submit_block(owner).is_some());

        let duplicate = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap_err();
        assert!(error_has_prefix(&duplicate, RECONCILIATION_REQUIRED_PREFIX));

        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].path, SUBMIT_PATH);
    }

    #[tokio::test]
    async fn signed_submit_nonce_failure_clears_owner_reservation() {
        let signed = signed_wallet_batch();
        let owner = signed.owner();
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "400 Bad Request",
            json!({"error": "nonce unavailable"}).to_string(),
        )])
        .await;
        let client = test_client(url);

        let error = client
            .submit_signed_wallet_batch(signed, mutation_permit_for(owner))
            .await
            .unwrap_err();

        assert!(matches!(error, RelayerError::Api { status: 400, .. }));
        assert!(client.ambiguous_submit_block(owner).is_none());
        client.ensure_owner_unblocked(owner).unwrap();
        let mut retry_reservation = client
            .reserve_owner_submit(owner, "payload:retry-after-nonce-error".to_string())
            .unwrap();
        retry_reservation.clear().unwrap();
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].path.contains("/nonce?address="));
    }

    #[test]
    fn owner_submit_reservation_blocks_duplicate_and_clears_cleanly() {
        let owner = address(WALLET_CREATE_OWNER);
        let client =
            test_client(DepositWalletRelayerUrl::loopback("http://127.0.0.1:1").unwrap());
        let mut reservation = client
            .reserve_owner_submit(owner, "payload:unit-test".to_string())
            .unwrap();

        let second = match client.reserve_owner_submit(owner, "payload:duplicate".to_string()) {
            Ok(_) => panic!("duplicate owner reservation should fail"),
            Err(error) => error,
        };
        assert!(error_has_prefix(&second, RECONCILIATION_REQUIRED_PREFIX));
        assert!(client.ensure_owner_unblocked(owner).is_err());

        reservation.clear().unwrap();
        client.ensure_owner_unblocked(owner).unwrap();
    }

    #[test]
    fn dropped_pre_submit_owner_reservation_clears_owner_block() {
        let signed = signed_wallet_batch();
        let owner = signed.owner();
        let client =
            test_client(DepositWalletRelayerUrl::loopback("http://127.0.0.1:1").unwrap());
        let payload_hash = signed_digest_payload_hash(signed.digest());
        let reservation = client
            .reserve_owner_submit(owner, payload_hash.clone())
            .unwrap();

        let duplicate = match client.reserve_owner_submit(owner, "payload:duplicate".to_string()) {
            Ok(_) => panic!("duplicate owner reservation should fail"),
            Err(error) => error,
        };
        assert!(error_has_prefix(&duplicate, RECONCILIATION_REQUIRED_PREFIX));

        drop(reservation);
        assert!(client.ambiguous_submit_block(owner).is_none());
        client.ensure_owner_unblocked(owner).unwrap();
    }

    #[test]
    fn dropped_post_submit_owner_reservation_records_ambiguous_block() {
        let signed = signed_wallet_batch();
        let owner = signed.owner();
        let client =
            test_client(DepositWalletRelayerUrl::loopback("http://127.0.0.1:1").unwrap());
        let payload_hash = signed_digest_payload_hash(signed.digest());
        let mut reservation = client
            .reserve_owner_submit(owner, payload_hash.clone())
            .unwrap();
        reservation.arm_ambiguous_on_drop();

        drop(reservation);
        assert_eq!(client.ambiguous_submit_block(owner), Some(payload_hash));
    }

    #[test]
    fn owner_mutation_state_rejects_new_entries_after_capacity() {
        let client =
            test_client(DepositWalletRelayerUrl::loopback("http://127.0.0.1:1").unwrap());
        {
            let mut state = client.mutation_state().unwrap();
            for index in 0..MAX_OWNER_MUTATION_RECORDS {
                let owner = Address::from_low_u64_be(index as u64 + 1);
                let payload_hash = format!("payload-{index}");
                state.owner_blocks.insert(
                    owner,
                    OwnerMutationBlock::Ambiguous {
                        payload_hash: payload_hash.clone(),
                    },
                );
                state.transaction_owners.insert(
                    format!("tx-{index}"),
                    OwnerTransactionRecord {
                        owner,
                        payload_hash,
                        source: OwnerTransactionSource::LocalSubmit,
                    },
                );
            }
        }

        let error = client
            .reserve_owner_submit(Address::from_low_u64_be(10_000), "payload-overflow".to_string())
            .err()
            .expect("new owner reservation should respect capacity");

        assert!(error_has_prefix(&error, MUTATION_BLOCKED_PREFIX));
    }

    #[test]
    fn owner_mutation_state_rejects_new_transactions_after_transaction_capacity() {
        let client =
            test_client(DepositWalletRelayerUrl::loopback("http://127.0.0.1:1").unwrap());
        let owner = address(WALLET_CREATE_OWNER);
        {
            let mut state = client.mutation_state().unwrap();
            for index in 0..MAX_OWNER_MUTATION_RECORDS {
                state.transaction_owners.insert(
                    format!("tx-{index}"),
                    OwnerTransactionRecord {
                        owner: Address::from_low_u64_be(index as u64 + 1),
                        payload_hash: format!("payload-{index}"),
                        source: OwnerTransactionSource::LocalSubmit,
                    },
                );
            }
        }

        let error = client
            .record_transaction_owner("tx-overflow", owner, "payload-overflow".to_string())
            .expect_err("new transaction owner should respect transaction capacity");

        assert!(error_has_prefix(&error, MUTATION_BLOCKED_PREFIX));
        client.ensure_owner_unblocked(owner).unwrap();
    }

    #[test]
    fn owner_submit_reservation_rejects_full_transaction_map_before_post() {
        let client =
            test_client(DepositWalletRelayerUrl::loopback("http://127.0.0.1:1").unwrap());
        let owner = address(WALLET_CREATE_OWNER);
        {
            let mut state = client.mutation_state().unwrap();
            for index in 0..MAX_OWNER_MUTATION_RECORDS {
                state.transaction_owners.insert(
                    format!("tx-{index}"),
                    OwnerTransactionRecord {
                        owner: Address::from_low_u64_be(index as u64 + 1),
                        payload_hash: format!("payload-{index}"),
                        source: OwnerTransactionSource::LocalSubmit,
                    },
                );
            }
        }

        let error = client
            .reserve_owner_submit(owner, "payload-preflight".to_string())
            .err()
            .expect("submit reservation should reject full transaction map before HTTP");

        assert!(error_has_prefix(&error, MUTATION_BLOCKED_PREFIX));
        client.ensure_owner_unblocked(owner).unwrap();
    }

    #[test]
    fn owner_mutation_state_rejects_transaction_owner_mapping_conflicts() {
        let client =
            test_client(DepositWalletRelayerUrl::loopback("http://127.0.0.1:1").unwrap());
        let owner = address(WALLET_CREATE_OWNER);
        let other_owner = address("0x0000000000000000000000000000000000000001");

        client
            .record_inflight_transaction(
                owner,
                "payload-original".to_string(),
                "tx-reused".to_string(),
            )
            .unwrap();

        let error = client
            .record_inflight_transaction(
                other_owner,
                "payload-other".to_string(),
                "tx-reused".to_string(),
            )
            .expect_err("transaction id reuse across owners should require reconciliation");

        assert!(error_has_prefix(&error, RECONCILIATION_REQUIRED_PREFIX));
        client.ensure_owner_unblocked(other_owner).unwrap();
        let state = client.mutation_state().unwrap();
        let record = state.transaction_owners.get("tx-reused").unwrap();
        assert_eq!(record.owner, owner);
        assert_eq!(record.payload_hash, "payload-original");
    }

    #[tokio::test]
    async fn post_api_failures_record_ambiguous_submit_including_quota() {
        let owner = address(WALLET_CREATE_OWNER);
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "400 Bad Request",
            format!("request echoed {API_KEY}"),
        )])
        .await;
        let client = test_client(url);

        let error = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap_err();

        assert!(error_has_prefix(&error, AMBIGUOUS_SUBMIT_PREFIX));
        assert!(client.ambiguous_submit_block(owner).is_some());
        let _ = handle.await.unwrap();

        let (url, handle) = spawn_server(vec![TestResponse::json("429 Too Many Requests", "{}")
            .with_header("retry-after", "7")])
        .await;
        let client = test_client(url);
        let error = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap_err();

        assert!(error_has_prefix(&error, AMBIGUOUS_SUBMIT_PREFIX));
        assert!(client.ambiguous_submit_block(owner).is_some());
        let duplicate = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap_err();
        assert!(error_has_prefix(&duplicate, RECONCILIATION_REQUIRED_PREFIX));
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
    }

    #[tokio::test]
    async fn unexpected_post_statuses_record_ambiguous_submit() {
        let owner = address(WALLET_CREATE_OWNER);

        for status in ["307 Temporary Redirect", "409 Conflict", "425 Too Early"] {
            let (url, handle) = spawn_server(vec![TestResponse::json(status, "{}")]).await;
            let client = test_client(url);

            let error = client
                .submit_wallet_create(owner, mutation_permit())
                .await
                .unwrap_err();

            assert!(
                error_has_prefix(&error, AMBIGUOUS_SUBMIT_PREFIX),
                "expected ambiguous submit for {status}, got {error:?}"
            );
            assert!(client.ambiguous_submit_block(owner).is_some());
            let duplicate = client
                .submit_wallet_create(owner, mutation_permit())
                .await
                .unwrap_err();
            assert!(error_has_prefix(&duplicate, RECONCILIATION_REQUIRED_PREFIX));
            let requests = handle.await.unwrap();
            assert_eq!(requests.len(), 1);
        }
    }

    #[tokio::test]
    async fn non_success_status_survives_truncated_error_body() {
        let (url, handle) = spawn_truncated_error_body_server("400 Bad Request").await;
        let client = test_client(url);

        let error = tokio::time::timeout(
            Duration::from_secs(1),
            client.get_wallet_nonce(address(WALLET_CREATE_OWNER)),
        )
        .await
        .expect("truncated 400 drain should not wait for the client timeout")
            .unwrap_err();

        assert!(matches!(error, RelayerError::Api { status: 400, .. }));
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].path.contains("/nonce?address="));

        let (url, handle) = spawn_truncated_error_body_server("429 Too Many Requests").await;
        let client = test_client(url);
        let error = tokio::time::timeout(
            Duration::from_secs(1),
            client.get_wallet_nonce(address(WALLET_CREATE_OWNER)),
        )
        .await
        .expect("truncated 429 drain should not wait for the client timeout")
            .unwrap_err();

        assert!(matches!(error, RelayerError::QuotaExhausted));
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].path.contains("/nonce?address="));
    }

    #[tokio::test]
    async fn retry_after_summary_accepts_numeric_and_http_date_values() {
        let owner = address(WALLET_CREATE_OWNER);
        let retry_at = httpdate::fmt_http_date(SystemTime::now() + Duration::from_secs(60));
        let (url, handle) = spawn_server(vec![
            TestResponse::json("503 Service Unavailable", "{}").with_header("retry-after", "7"),
            TestResponse::json("503 Service Unavailable", "{}")
                .with_header("retry-after", &retry_at),
            TestResponse::json("503 Service Unavailable", "{}")
                .with_header("retry-after", "soon"),
        ])
        .await;
        let client = test_client(url);

        let error = client.get_wallet_nonce(owner).await.unwrap_err();
        assert!(matches!(error, RelayerError::Api { status: 503, .. }));
        assert!(error.to_string().contains("retry after 7s"));

        let error = client.get_wallet_nonce(owner).await.unwrap_err();
        assert!(matches!(error, RelayerError::Api { status: 503, .. }));
        assert!(error.to_string().contains("retry after "));

        let error = client.get_wallet_nonce(owner).await.unwrap_err();
        assert!(matches!(error, RelayerError::Api { status: 503, .. }));
        assert!(!error.to_string().contains("retry after"));

        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 3);
    }

    #[test]
    fn retry_after_duration_at_parses_http_dates_deterministically() {
        let now = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, HeaderValue::from_static("11"));
        assert_eq!(
            retry_after_duration_at(&headers, now),
            Some(Duration::from_secs(11))
        );

        let future = httpdate::fmt_http_date(now + Duration::from_secs(42));
        headers.insert(RETRY_AFTER, HeaderValue::from_str(&future).unwrap());
        assert_eq!(
            retry_after_duration_at(&headers, now),
            Some(Duration::from_secs(42))
        );

        let past = httpdate::fmt_http_date(now - Duration::from_secs(42));
        headers.insert(RETRY_AFTER, HeaderValue::from_str(&past).unwrap());
        assert_eq!(
            retry_after_duration_at(&headers, now),
            Some(Duration::ZERO)
        );

        headers.insert(RETRY_AFTER, HeaderValue::from_static("soon"));
        assert_eq!(retry_after_duration_at(&headers, now), None);
    }

    #[tokio::test]
    async fn server_timeout_status_records_ambiguous_submit() {
        let owner = address(WALLET_CREATE_OWNER);
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "504 Gateway Timeout",
            "{}",
        )])
        .await;
        let client = test_client(url);

        let error = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap_err();

        assert!(error_has_prefix(&error, AMBIGUOUS_SUBMIT_PREFIX));
        assert!(client.ambiguous_submit_block(owner).is_some());
        let duplicate = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap_err();
        assert!(error_has_prefix(&duplicate, RECONCILIATION_REQUIRED_PREFIX));
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
    }

    #[tokio::test]
    async fn request_timeout_status_records_ambiguous_submit_and_blocks_duplicate() {
        let owner = address(WALLET_CREATE_OWNER);
        let (url, handle) = spawn_server(vec![TestResponse::json("408 Request Timeout", "{}")])
            .await;
        let client = test_client(url);

        let error = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap_err();

        assert!(error_has_prefix(&error, AMBIGUOUS_SUBMIT_PREFIX));
        assert!(client.ambiguous_submit_block(owner).is_some());
        let duplicate = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap_err();
        assert!(error_has_prefix(&duplicate, RECONCILIATION_REQUIRED_PREFIX));
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
    }

    #[tokio::test]
    async fn server_error_status_records_ambiguous_submit_and_blocks_duplicate() {
        let owner = address(WALLET_CREATE_OWNER);
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "500 Internal Server Error",
            "{}",
        )])
        .await;
        let client = test_client(url);

        let error = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap_err();

        assert!(error_has_prefix(&error, AMBIGUOUS_SUBMIT_PREFIX));
        assert!(client.ambiguous_submit_block(owner).is_some());
        let duplicate = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap_err();
        assert!(error_has_prefix(&duplicate, RECONCILIATION_REQUIRED_PREFIX));
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
    }

    #[tokio::test]
    async fn oversized_submit_success_response_records_ambiguous_submit() {
        let owner = address(WALLET_CREATE_OWNER);
        let (url, handle) = spawn_server(vec![TestResponse::json_without_content_length(
            "200 OK",
            "x".repeat(MAX_SUCCESS_BODY_BYTES + 1),
        )])
        .await;
        let client = test_client(url);

        let error = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap_err();

        assert!(error_has_prefix(&error, AMBIGUOUS_SUBMIT_PREFIX));
        assert!(client.ambiguous_submit_block(owner).is_some());
        let duplicate = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap_err();
        assert!(error_has_prefix(&duplicate, RECONCILIATION_REQUIRED_PREFIX));
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
    }

    #[tokio::test]
    async fn submit_unknown_state_records_reconciliation_block() {
        let owner = address(WALLET_CREATE_OWNER);
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "200 OK",
            transaction_response("tx-weird", "STATE_WEIRD\nforged"),
        )])
        .await;
        let client = test_client(url);

        let error = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap_err();
        let rendered = error.to_string();

        assert!(error_has_prefix(&error, RECONCILIATION_REQUIRED_PREFIX));
        assert!(!rendered.contains('\n'));
        assert!(!rendered.contains("STATE_WEIRD"));
        assert!(rendered.contains("<unrecognized relayer state>"));
        assert!(client.ambiguous_submit_block(owner).is_some());
        let _ = handle.await.unwrap();
    }

    #[tokio::test]
    async fn submit_preflight_rejects_full_transaction_map_before_http() {
        let owner = address(WALLET_CREATE_OWNER);
        let url = DepositWalletRelayerUrl::loopback("http://127.0.0.1:1").unwrap();
        let client = test_client(url);
        {
            let mut state = client.mutation_state().unwrap();
            for index in 0..MAX_OWNER_MUTATION_RECORDS {
                state.transaction_owners.insert(
                    format!("tx-existing-{index}"),
                    OwnerTransactionRecord {
                        owner: Address::from_low_u64_be(index as u64 + 1),
                        payload_hash: format!("payload-{index}"),
                        source: OwnerTransactionSource::LocalSubmit,
                    },
                );
            }
        }

        let error = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap_err();

        assert!(error_has_prefix(&error, MUTATION_BLOCKED_PREFIX));
        assert!(client.ambiguous_submit_block(owner).is_none());
        client.ensure_owner_unblocked(owner).unwrap();
    }

    #[tokio::test]
    async fn redirects_are_not_followed_and_target_gets_no_auth() {
        let target_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        target_listener.set_nonblocking(true).unwrap();
        let target_addr = target_listener.local_addr().unwrap();

        let (url, handle) = spawn_server(vec![TestResponse::redirect(format!(
            "http://{target_addr}/redirect-target"
        ))])
        .await;
        let client =
            DepositWalletRelayerClient::new(url, relayer_auth(), deposit_wallet_contract_config(137).unwrap())
                .unwrap();

        let error = client
            .get_wallet_nonce(address(WALLET_CREATE_OWNER))
            .await
            .unwrap_err();

        assert!(matches!(error, RelayerError::Api { status: 307, .. }));
        let source_requests = handle.await.unwrap();
        assert_eq!(source_requests.len(), 1);
        assert_eq!(source_requests[0].header("RELAYER_API_KEY"), Some(API_KEY));

        match target_listener.accept() {
            Err(error) if error.kind() == ErrorKind::WouldBlock => {}
            Ok(_) => panic!("redirect target unexpectedly received a request"),
            Err(error) => panic!("unexpected redirect target accept error: {error}"),
        }
    }

    #[tokio::test]
    async fn get_transaction_accepts_array_response() {
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "200 OK",
            json!([
                {
                    "transactionID": "other-tx",
                    "state": "STATE_FAILED",
                    "transactionHash": "0x38cbfbeae8fffa4e2b187ee5978d3ee9cafc53af0363ed90a35b7ea9016535d8"
                },
                {
                    "transactionID": "tx-array",
                    "state": "STATE_CONFIRMED",
                    "transactionHash": "0x38cbfbeae8fffa4e2b187ee5978d3ee9cafc53af0363ed90a35b7ea9016535d8"
                }
            ])
            .to_string(),
        )])
        .await;
        let client = test_client(url);

        let receipt = client.get_transaction("tx-array").await.unwrap();

        assert_eq!(receipt.transaction_id, "tx-array");
        assert_eq!(receipt.state, RelayerTransactionState::Confirmed);
        assert_eq!(
            receipt.transaction_hash.as_deref(),
            Some("0x38cbfbeae8fffa4e2b187ee5978d3ee9cafc53af0363ed90a35b7ea9016535d8")
        );
        let requests = handle.await.unwrap();
        assert_eq!(requests[0].path, "/transaction?id=tx-array");
    }

    #[tokio::test]
    async fn get_transaction_rejects_array_response_with_trailing_bytes() {
        let mut body = json!([
            {
                "transactionID": "tx-array",
                "state": "STATE_CONFIRMED",
                "transactionHash": "0x38cbfbeae8fffa4e2b187ee5978d3ee9cafc53af0363ed90a35b7ea9016535d8"
            }
        ])
        .to_string();
        body.push_str(" trailing");
        let (url, handle) = spawn_server(vec![TestResponse::json("200 OK", body)]).await;
        let client = test_client(url);

        let error = client.get_transaction("tx-array").await.unwrap_err();

        assert!(matches!(error, RelayerError::Other(_)));
        assert!(error.to_string().contains("trailing characters"));
        let requests = handle.await.unwrap();
        assert_eq!(requests[0].path, "/transaction?id=tx-array");
    }

    #[tokio::test]
    async fn get_transaction_rejects_duplicate_matching_response_ids() {
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "200 OK",
            json!([
                {
                    "transactionID": "tx-duplicate",
                    "state": "STATE_CONFIRMED",
                    "transactionHash": "0x38cbfbeae8fffa4e2b187ee5978d3ee9cafc53af0363ed90a35b7ea9016535d8",
                    "owner": WALLET_CREATE_OWNER
                },
                {
                    "transactionID": "tx-duplicate",
                    "state": "STATE_FAILED",
                    "transactionHash": "0x48cbfbeae8fffa4e2b187ee5978d3ee9cafc53af0363ed90a35b7ea9016535d8",
                    "owner": "0x0000000000000000000000000000000000000001"
                }
            ])
            .to_string(),
        )])
        .await;
        let client = test_client(url);

        let error = client.get_transaction("tx-duplicate").await.unwrap_err();

        assert!(error_has_prefix(&error, RECONCILIATION_REQUIRED_PREFIX));
        assert!(error.to_string().contains("duplicate"));
        let requests = handle.await.unwrap();
        assert_eq!(requests[0].path, "/transaction?id=tx-duplicate");
    }

    #[tokio::test]
    async fn get_transaction_rejects_mismatched_response_ids_and_invalid_request_ids() {
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "200 OK",
            transaction_response("other-tx", "STATE_CONFIRMED"),
        )])
        .await;
        let client = test_client(url);

        let error = client.get_transaction("tx-array").await.unwrap_err();
        assert!(error_has_prefix(&error, RECONCILIATION_REQUIRED_PREFIX));
        let _ = handle.await.unwrap();

        let (url, handle) = spawn_server(vec![TestResponse::json(
            "200 OK",
            json!([
                {
                    "transactionID": "other-tx",
                    "state": "STATE_CONFIRMED",
                    "transactionHash": "0x38cbfbeae8fffa4e2b187ee5978d3ee9cafc53af0363ed90a35b7ea9016535d8"
                }
            ])
            .to_string(),
        )])
        .await;
        let client = test_client(url);

        let error = client.get_transaction("tx-array").await.unwrap_err();
        assert!(error_has_prefix(&error, RECONCILIATION_REQUIRED_PREFIX));
        let _ = handle.await.unwrap();

        let (url, handle) = spawn_server(vec![TestResponse::json(
            "200 OK",
            transaction_response(" tx-array ", "STATE_CONFIRMED"),
        )])
        .await;
        let client = test_client(url);

        let error = client.get_transaction("tx-array").await.unwrap_err();
        assert!(matches!(error, RelayerError::Other(_)));
        let _ = handle.await.unwrap();

        let url = DepositWalletRelayerUrl::loopback("http://127.0.0.1:1").unwrap();
        let client = test_client(url);
        let error = client.get_transaction("bad\nid").await.unwrap_err();
        assert!(!error.to_string().contains('\n'));
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
        assert!(validate_transaction_id("tx/abc").is_err());
        assert!(validate_transaction_id("tx\nabc").is_err());
    }

    #[tokio::test]
    async fn get_transaction_preserves_unknown_state_wire_value_in_public_receipt() {
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "200 OK",
            transaction_response("tx-unknown", "STATE_WEIRD\nforged"),
        )])
        .await;
        let client = test_client(url);

        let receipt = client.get_transaction("tx-unknown").await.unwrap();

        match receipt.state {
            RelayerTransactionState::Unknown(raw) => {
                assert_eq!(raw, "STATE_WEIRD\nforged");
            }
            state => panic!("expected unknown state, got {state:?}"),
        }
        let _ = handle.await.unwrap();
    }

    #[tokio::test]
    async fn get_transaction_rejects_invalid_hash_and_oversized_success_body() {
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "200 OK",
            json!({
                "transactionID": "tx-bad-hash",
                "state": "STATE_CONFIRMED",
                "transactionHash": "bad\nhash"
            })
            .to_string(),
        )])
        .await;
        let client = test_client(url);

        let error = client.get_transaction("tx-bad-hash").await.unwrap_err();
        assert!(error_has_prefix(&error, RECONCILIATION_REQUIRED_PREFIX));
        assert!(!error.to_string().contains("bad\nhash"));
        let _ = handle.await.unwrap();

        let (url, handle) = spawn_server(vec![TestResponse::json(
            "200 OK",
            "x".repeat(MAX_SUCCESS_BODY_BYTES + 1),
        )])
        .await;
        let client = test_client(url);

        let error = client.get_wallet_nonce(address(WALLET_CREATE_OWNER)).await.unwrap_err();
        assert!(matches!(error, RelayerError::Other(message) if message.contains("maximum size")));
        let _ = handle.await.unwrap();

        let (url, handle) = spawn_server(vec![TestResponse::json_without_content_length(
            "200 OK",
            "x".repeat(MAX_SUCCESS_BODY_BYTES + 1),
        )])
        .await;
        let client = test_client(url);

        let error = client
            .get_wallet_nonce(address(WALLET_CREATE_OWNER))
            .await
            .unwrap_err();
        assert!(matches!(error, RelayerError::Other(message) if message.contains("maximum size")));
        let _ = handle.await.unwrap();
    }

    #[tokio::test]
    async fn get_transaction_accepts_large_metadata_under_transaction_limit() {
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "200 OK",
            json!({
                "transactionID": "tx-large-metadata",
                "state": "STATE_CONFIRMED",
                "transactionHash": "0x38cbfbeae8fffa4e2b187ee5978d3ee9cafc53af0363ed90a35b7ea9016535d8",
                "owner": WALLET_CREATE_OWNER,
                "metadata": "x".repeat(MAX_SUCCESS_BODY_BYTES + 1)
            })
            .to_string(),
        )])
        .await;
        let client = test_client(url);

        let receipt = client.get_transaction("tx-large-metadata").await.unwrap();

        assert_eq!(receipt.transaction_id, "tx-large-metadata");
        assert_eq!(receipt.state, RelayerTransactionState::Confirmed);
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
    }

    #[test]
    fn transaction_response_fixture_matches_official_owner_field() {
        let transaction_id = "0190b317-a1d3-7bec-9b91-eeb6dcd3a620";
        let fixture = fixture_text("wallet_transaction_response.json");

        let parsed = parse_transaction_response(transaction_id, fixture.as_bytes()).unwrap();

        assert_eq!(parsed.receipt.transaction_id, transaction_id);
        assert_eq!(parsed.receipt.state, RelayerTransactionState::Confirmed);
        assert_eq!(
            parsed.receipt.transaction_hash.as_deref(),
            Some("0x38cbfbeae8fffa4e2b187ee5978d3ee9cafc53af0363ed90a35b7ea9016535d8")
        );
        assert_eq!(
            parsed.owner,
            Some(address("0x6e0c80c90ea6c15917308f820eac91ce2724b5b5"))
        );
        assert_eq!(
            parsed.receipt.owner,
            Some(address("0x6e0c80c90ea6c15917308f820eac91ce2724b5b5"))
        );
    }

    #[tokio::test]
    async fn get_transaction_rejects_body_over_transaction_limit() {
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "200 OK",
            json!({
                "transactionID": "tx-too-large",
                "state": "STATE_CONFIRMED",
                "transactionHash": "0x38cbfbeae8fffa4e2b187ee5978d3ee9cafc53af0363ed90a35b7ea9016535d8",
                "owner": WALLET_CREATE_OWNER,
                "metadata": "x".repeat(MAX_TRANSACTION_SUCCESS_BODY_BYTES + 1)
            })
            .to_string(),
        )])
        .await;
        let client = test_client(url);

        let error = client.get_transaction("tx-too-large").await.unwrap_err();

        assert!(matches!(error, RelayerError::Other(message) if message.contains("maximum size")));
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
    }

    #[tokio::test]
    async fn get_wallet_nonce_rejects_malformed_success_payloads() {
        for body in ["not-json".to_string(), "{}".to_string(), json!({"nonce": "nan"}).to_string()]
        {
            let (url, handle) =
                spawn_server(vec![TestResponse::json("200 OK", body)]).await;
            let client = test_client(url);

            let error = client
                .get_wallet_nonce(address(WALLET_CREATE_OWNER))
                .await
                .unwrap_err();

            assert!(matches!(error, RelayerError::Other(_)));
            let requests = handle.await.unwrap();
            assert_eq!(requests.len(), 1);
            assert_eq!(
                requests[0].path,
                format!(
                    "/nonce?address={}&type=WALLET",
                    to_checksum(&address(WALLET_CREATE_OWNER), None)
                )
            );
        }
    }

    #[tokio::test]
    async fn get_transaction_rejects_oversized_array_response() {
        let body = (0..=MAX_TRANSACTION_RESPONSE_ITEMS)
            .map(|index| {
                json!({
                    "transactionID": format!("other-tx-{index}"),
                    "state": "STATE_CONFIRMED",
                    "transactionHash": "0x38cbfbeae8fffa4e2b187ee5978d3ee9cafc53af0363ed90a35b7ea9016535d8"
                })
            })
            .collect::<Vec<_>>();
        let (url, handle) =
            spawn_server(vec![TestResponse::json("200 OK", json!(body).to_string())]).await;
        let client = test_client(url);

        let error = client.get_transaction("tx-array").await.unwrap_err();
        assert!(error_has_prefix(&error, RECONCILIATION_REQUIRED_PREFIX));
        let _ = handle.await.unwrap();
    }

    #[tokio::test]
    async fn submit_terminal_states_clear_owner_block_or_return_terminal_error() {
        let owner = address(WALLET_CREATE_OWNER);

        for (transaction_id, state, expected_error) in [
            ("tx-invalid-now", "STATE_INVALID", Some("invalid")),
            ("tx-failed-now", "STATE_FAILED", Some("failed")),
        ] {
            let (url, handle) = spawn_server(vec![
                TestResponse::json("200 OK", transaction_response(transaction_id, state)),
                TestResponse::json("200 OK", json!({"nonce": "34"}).to_string()),
            ])
            .await;
            let client = test_client(url);

            let result = client.submit_wallet_create(owner, mutation_permit()).await;
            match expected_error {
                None => {
                    let receipt = result.unwrap();
                    assert_eq!(receipt.transaction_id, transaction_id);
                    assert_eq!(receipt.state, RelayerTransactionState::Confirmed);
                }
                Some("invalid") => {
                    assert!(matches!(result.unwrap_err(), RelayerError::TransactionInvalid(_)));
                }
                Some("failed") => {
                    assert!(matches!(result.unwrap_err(), RelayerError::TransactionFailed(_)));
                }
                _ => unreachable!(),
            }

            let nonce = client.get_wallet_nonce(owner).await.unwrap();
            assert_eq!(nonce, U256::from(34u64));
            let mut retry_reservation = client
                .reserve_owner_submit(owner, format!("payload:retry-after-{transaction_id}"))
                .unwrap();
            retry_reservation.clear().unwrap();
            let requests = handle.await.unwrap();
            assert_eq!(requests.len(), 2);
            assert_eq!(requests[0].path, SUBMIT_PATH);
            assert!(requests[1].path.contains("/nonce?address="));
        }
    }

    #[tokio::test]
    async fn immediate_confirmed_submit_requires_transaction_poll_reconciliation() {
        let owner = address(WALLET_CREATE_OWNER);
        let transaction_id = "tx-submit-confirmed";
        let (url, handle) = spawn_server(vec![
            TestResponse::json(
                "200 OK",
                transaction_response(transaction_id, "STATE_CONFIRMED"),
            ),
            TestResponse::json(
                "200 OK",
                transaction_response(transaction_id, "STATE_CONFIRMED"),
            ),
            TestResponse::json("200 OK", json!({"nonce": "41"}).to_string()),
        ])
        .await;
        let client = test_client(url);

        let error = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap_err();

        assert!(error_has_prefix(&error, RECONCILIATION_REQUIRED_PREFIX));
        assert!(client.ambiguous_submit_block(owner).is_some());
        let blocked = client.get_wallet_nonce(owner).await.unwrap_err();
        assert!(error_has_prefix(&blocked, RECONCILIATION_REQUIRED_PREFIX));

        let receipt = client
            .poll_transaction(
                transaction_id,
                DepositWalletPollPolicy::new(1, Duration::from_millis(100)).unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(receipt.transaction_id, transaction_id);
        assert_eq!(receipt.state, RelayerTransactionState::Confirmed);
        client.ensure_owner_unblocked(owner).unwrap();
        {
            let state = client.mutation_state().unwrap();
            assert!(!state.transaction_owners.contains_key(transaction_id));
        }
        let nonce = client.get_wallet_nonce(owner).await.unwrap();
        assert_eq!(nonce, U256::from(41u64));

        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[0].path, SUBMIT_PATH);
        assert_eq!(requests[1].path, format!("/transaction?id={transaction_id}"));
        assert!(requests[2].path.contains("/nonce?address="));
    }

    #[tokio::test]
    async fn pending_transaction_response_treats_empty_hash_as_unavailable() {
        let (url, handle) = spawn_server(vec![
            TestResponse::json(
                "200 OK",
                json!({
                    "transactionID": "tx-empty-hash",
                    "state": "STATE_NEW",
                    "transactionHash": ""
                })
                .to_string(),
            ),
            TestResponse::json(
                "200 OK",
                transaction_response("tx-empty-hash", "STATE_CONFIRMED"),
            ),
        ])
        .await;
        let client = test_client(url);

        let receipt = client
            .poll_transaction(
                "tx-empty-hash",
                DepositWalletPollPolicy::new(2, Duration::from_millis(100)).unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(receipt.state, RelayerTransactionState::Confirmed);
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 2);
    }

    #[tokio::test]
    async fn polling_keeps_mined_pending_until_confirmed_and_uses_injected_sleeper() {
        let (result, requests, sleeper, _policy) = poll_sequence(
            &[
                "STATE_NEW",
                "STATE_EXECUTED",
                "STATE_MINED",
                "STATE_CONFIRMED",
            ],
            4,
        )
        .await;

        let receipt = result.unwrap();
        assert_eq!(receipt.state, RelayerTransactionState::Confirmed);
        assert_eq!(requests.len(), 4);
        assert!(requests
            .iter()
            .all(|request| request.path == "/transaction?id=tx-123"));
        let sleeps = sleeper.sleeps();
        assert_eq!(sleeps.len(), 3);
        assert!((Duration::from_millis(100)..=Duration::from_millis(125)).contains(&sleeps[0]));
        assert!((Duration::from_millis(200)..=Duration::from_millis(250)).contains(&sleeps[1]));
        assert!((Duration::from_millis(400)..=Duration::from_millis(500)).contains(&sleeps[2]));
        assert_ne!(sleeps[0], Duration::from_millis(100));

        let (result, _, _, _) = poll_sequence(&["STATE_INVALID"], 1).await;
        assert!(matches!(result.unwrap_err(), RelayerError::TransactionInvalid(_)));

        let (result, _, _, _) = poll_sequence(&["STATE_FAILED"], 1).await;
        assert!(matches!(result.unwrap_err(), RelayerError::TransactionFailed(_)));

        let (result, _, _, _) = poll_sequence(&["STATE_STRANGE"], 1).await;
        let error = result.unwrap_err();
        let rendered = error.to_string();
        assert!(error_has_prefix(&error, RECONCILIATION_REQUIRED_PREFIX));
        assert!(!rendered.contains("STATE_STRANGE"));
        assert!(rendered.contains("<unrecognized relayer state>"));

        let (result, requests, sleeper, _policy) =
            poll_sequence(&["STATE_NEW", "STATE_NEW"], 2).await;
        assert!(matches!(result.unwrap_err(), RelayerError::Timeout));
        assert_eq!(requests.len(), 2);
        let sleeps = sleeper.sleeps();
        assert_eq!(sleeps.len(), 1);
        assert!((Duration::from_millis(100)..=Duration::from_millis(125)).contains(&sleeps[0]));
    }

    #[tokio::test]
    async fn transient_poll_fetch_error_retries_without_ambiguous_owner_block() {
        let owner = address(WALLET_CREATE_OWNER);
        let retry_at = httpdate::fmt_http_date(SystemTime::now() + Duration::from_secs(3600));
        let (url, handle) = spawn_server(vec![
            TestResponse::json("503 Service Unavailable", "{}")
                .with_header("retry-after", &retry_at),
            TestResponse::json(
                "200 OK",
                transaction_response("tx-transient", "STATE_CONFIRMED"),
            ),
        ])
        .await;
        let sleeper = Arc::new(RecordingSleeper::default());
        let client = test_client_with_sleeper(url, sleeper.clone());
        client
            .record_inflight_transaction(
                owner,
                "payload:transient-poll".to_string(),
                "tx-transient".to_string(),
            )
            .unwrap();

        let receipt = client
            .poll_owner_transaction(
                owner,
                "tx-transient",
                DepositWalletPollPolicy::new(2, Duration::from_millis(100)).unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(receipt.state, RelayerTransactionState::Confirmed);
        assert!(client.ambiguous_submit_block(owner).is_none());
        client.ensure_owner_unblocked(owner).unwrap();
        let sleeps = sleeper.sleeps();
        assert_eq!(sleeps, vec![MAX_POLL_INTERVAL]);
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 2);
    }

    #[tokio::test]
    async fn poll_policy_rejects_invalid_bounds_and_caps_backoff() {
        assert!(DepositWalletPollPolicy::new(0, Duration::from_secs(1)).is_err());
        assert!(DepositWalletPollPolicy::new(1, Duration::from_millis(99)).is_err());
        assert!(
            DepositWalletPollPolicy::new(MAX_POLL_ATTEMPTS + 1, Duration::from_secs(1)).is_err()
        );

        let policy = DepositWalletPollPolicy::new(1, Duration::from_secs(10)).unwrap();
        assert_eq!(policy.interval_for_attempt(4), MAX_POLL_INTERVAL);

        let invalid_literal = DepositWalletPollPolicy {
            max_attempts: 0,
            interval: Duration::from_secs(1),
        };
        let client =
            test_client(DepositWalletRelayerUrl::loopback("http://127.0.0.1:1").unwrap());
        let error = client
            .poll_transaction("tx-123", invalid_literal)
            .await
            .unwrap_err();
        assert!(matches!(error, RelayerError::Other(message) if message.contains("max attempts")));
    }

    #[tokio::test]
    async fn poll_timeout_keeps_owner_block_until_manual_reconciliation() {
        let owner = address(WALLET_CREATE_OWNER);
        let (url, handle) = spawn_server(vec![
            TestResponse::json("200 OK", transaction_response("tx-pending", "STATE_NEW")),
            TestResponse::json("200 OK", transaction_response("tx-pending", "STATE_NEW")),
            TestResponse::json("200 OK", transaction_response("tx-pending", "STATE_NEW")),
        ])
        .await;
        let client = test_client(url);

        let receipt = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap();
        assert_eq!(receipt.transaction_id, "tx-pending");

        let policy = DepositWalletPollPolicy::new(2, Duration::from_millis(100)).unwrap();
        let error = client
            .poll_transaction("tx-pending", policy)
            .await
            .unwrap_err();
        assert!(matches!(error, RelayerError::Timeout));

        let blocked = client.get_wallet_nonce(owner).await.unwrap_err();
        assert!(error_has_prefix(&blocked, RECONCILIATION_REQUIRED_PREFIX));

        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 3);
    }

    #[tokio::test]
    async fn owner_aware_repoll_can_continue_known_ambiguous_transaction() {
        let owner = address(WALLET_CREATE_OWNER);
        let (url, handle) = spawn_server(vec![
            TestResponse::json("200 OK", transaction_response("tx-repoll", "STATE_NEW")),
            TestResponse::json("200 OK", transaction_response("tx-repoll", "STATE_NEW")),
            TestResponse::json("200 OK", transaction_response("tx-repoll", "STATE_NEW")),
            TestResponse::json("200 OK", transaction_response("tx-repoll", "STATE_CONFIRMED")),
            TestResponse::json("200 OK", json!({"nonce": "38"}).to_string()),
        ])
        .await;
        let client = test_client(url);

        let receipt = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap();
        assert_eq!(receipt.transaction_id, "tx-repoll");

        let timeout = client
            .poll_owner_transaction(
                owner,
                "tx-repoll",
                DepositWalletPollPolicy::new(1, Duration::from_millis(100)).unwrap(),
            )
            .await
            .unwrap_err();
        assert!(matches!(timeout, RelayerError::Timeout));
        assert!(client.ambiguous_submit_block(owner).is_some());

        let receipt = client
            .poll_owner_transaction(
                owner,
                "tx-repoll",
                DepositWalletPollPolicy::new(2, Duration::from_millis(100)).unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(receipt.state, RelayerTransactionState::Confirmed);

        let nonce = client.get_wallet_nonce(owner).await.unwrap();
        assert_eq!(nonce, U256::from(38u64));

        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 5);
    }

    #[tokio::test]
    async fn owner_aware_poll_restores_recovered_block_after_local_clear() {
        let owner = address(WALLET_CREATE_OWNER);
        let (url, handle) = spawn_server(vec![
            TestResponse::json("200 OK", transaction_response("tx-cleared", "STATE_NEW")),
            TestResponse::json("200 OK", transaction_response("tx-cleared", "STATE_NEW")),
            TestResponse::json("200 OK", transaction_response("tx-cleared", "STATE_NEW")),
        ])
        .await;
        let sleeper = Arc::new(ClearingSleeper::default());
        let sleeper_trait: Arc<dyn DepositWalletSleeper> = sleeper.clone();
        let client = DepositWalletRelayerClient::from_parts(
            reqwest_client(Duration::from_secs(2)),
            url,
            relayer_auth(),
            deposit_wallet_contract_config(137).unwrap(),
            Arc::new(FixedClock { now: 1_700_000_000 }),
            sleeper_trait,
        );
        sleeper.attach_state(client.mutation_state.clone());

        let receipt = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap();
        assert_eq!(receipt.transaction_id, "tx-cleared");

        let error = client
            .poll_owner_transaction(
                owner,
                "tx-cleared",
                DepositWalletPollPolicy::new(2, Duration::from_millis(100)).unwrap(),
            )
            .await
            .unwrap_err();

        assert!(matches!(error, RelayerError::Timeout));
        assert!(client.ambiguous_submit_block(owner).is_some());
        let blocked = client.get_wallet_nonce(owner).await.unwrap_err();
        assert!(error_has_prefix(&blocked, RECONCILIATION_REQUIRED_PREFIX));

        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 3);
    }

    #[tokio::test]
    async fn owner_aware_poll_pending_without_local_evidence_blocks_owner_after_timeout() {
        let owner = address(WALLET_CREATE_OWNER);
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "200 OK",
            transaction_response("tx-recovered", "STATE_NEW"),
        )])
        .await;
        let client = test_client(url);

        let error = client
            .poll_owner_transaction_with_reconciliation_permit(
                owner,
                "tx-recovered",
                DepositWalletPollPolicy::new(1, Duration::from_millis(100)).unwrap(),
                mutation_permit_for(owner),
            )
            .await
            .unwrap_err();
        assert!(matches!(error, RelayerError::Timeout));

        assert!(client.ambiguous_submit_block(owner).is_some());
        let blocked = client.get_wallet_nonce(owner).await.unwrap_err();
        assert!(error_has_prefix(&blocked, RECONCILIATION_REQUIRED_PREFIX));

        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].path, "/transaction?id=tx-recovered");
    }

    #[tokio::test]
    async fn owner_aware_poll_allows_existing_inflight_transaction_to_reach_terminal_state() {
        let owner = address(WALLET_CREATE_OWNER);
        let (url, handle) = spawn_server(vec![
            TestResponse::json("200 OK", transaction_response("tx-owner-aware", "STATE_NEW")),
            TestResponse::json("200 OK", transaction_response("tx-owner-aware", "STATE_NEW")),
            TestResponse::json(
                "200 OK",
                transaction_response("tx-owner-aware", "STATE_CONFIRMED"),
            ),
            TestResponse::json("200 OK", json!({"nonce": "35"}).to_string()),
        ])
        .await;
        let client = test_client(url);

        let receipt = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap();
        assert_eq!(receipt.transaction_id, "tx-owner-aware");

        let receipt = client
            .poll_owner_transaction(
                owner,
                "tx-owner-aware",
                DepositWalletPollPolicy::new(2, Duration::from_millis(100)).unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(receipt.state, RelayerTransactionState::Confirmed);

        let nonce = client.get_wallet_nonce(owner).await.unwrap();
        assert_eq!(nonce, U256::from(35u64));
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 4);
    }

    #[tokio::test]
    async fn owner_aware_poll_requires_response_owner_before_recording_block() {
        let owner = address(WALLET_CREATE_OWNER);
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "200 OK",
            json!({
                "transactionID": "tx-no-owner",
                "state": "STATE_NEW",
                "transactionHash": "0x38cbfbeae8fffa4e2b187ee5978d3ee9cafc53af0363ed90a35b7ea9016535d8"
            })
            .to_string(),
        )])
        .await;
        let client = test_client(url);

        let error = client
            .poll_owner_transaction_with_reconciliation_permit(
                owner,
                "tx-no-owner",
                DepositWalletPollPolicy::new(1, Duration::from_millis(100)).unwrap(),
                mutation_permit_for(owner),
            )
            .await
            .unwrap_err();
        assert!(error_has_prefix(&error, RECONCILIATION_REQUIRED_PREFIX));
        assert!(client.ambiguous_submit_block(owner).is_some());
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);

        let (url, handle) = spawn_server(vec![TestResponse::json(
            "200 OK",
            json!({
                "transactionID": "tx-bad-owner",
                "state": "STATE_NEW",
                "transactionHash": "0x38cbfbeae8fffa4e2b187ee5978d3ee9cafc53af0363ed90a35b7ea9016535d8",
                "owner": "not-an-address"
            })
            .to_string(),
        )])
        .await;
        let client = test_client(url);

        let error = client
            .poll_owner_transaction_with_reconciliation_permit(
                owner,
                "tx-bad-owner",
                DepositWalletPollPolicy::new(1, Duration::from_millis(100)).unwrap(),
                mutation_permit_for(owner),
            )
            .await
            .unwrap_err();
        assert!(matches!(error, RelayerError::Other(_)));
        assert!(client.ambiguous_submit_block(owner).is_some());
        {
            let state = client.mutation_state().unwrap();
            assert!(state
                .transaction_owners
                .get("tx-bad-owner")
                .is_some_and(|record| record.owner == owner));
        }
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);

        let (url, handle) = spawn_server(vec![TestResponse::json(
            "200 OK",
            json!({
                "transactionID": "tx-known-bad-owner",
                "state": "STATE_NEW",
                "transactionHash": "0x38cbfbeae8fffa4e2b187ee5978d3ee9cafc53af0363ed90a35b7ea9016535d8",
                "owner": "not-an-address"
            })
            .to_string(),
        )])
        .await;
        let client = test_client(url);
        client
            .record_inflight_transaction(
                owner,
                "payload:known-bad-owner".to_string(),
                "tx-known-bad-owner".to_string(),
            )
            .unwrap();

        let error = client
            .poll_owner_transaction(
                owner,
                "tx-known-bad-owner",
                DepositWalletPollPolicy::new(1, Duration::from_millis(100)).unwrap(),
            )
            .await
            .unwrap_err();
        assert!(matches!(error, RelayerError::Other(_)));
        assert!(client.ambiguous_submit_block(owner).is_some());
        let blocked = client.get_wallet_nonce(owner).await.unwrap_err();
        assert!(error_has_prefix(&blocked, RECONCILIATION_REQUIRED_PREFIX));
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
    }

    #[tokio::test]
    async fn owner_aware_confirmed_without_hash_blocks_without_local_evidence() {
        let owner = address(WALLET_CREATE_OWNER);
        let transaction_id = "tx-confirmed-no-local-hash";
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "200 OK",
            json!({
                "transactionID": transaction_id,
                "state": "STATE_CONFIRMED",
                "owner": WALLET_CREATE_OWNER
            })
            .to_string(),
        )])
        .await;
        let client = test_client(url);

        let error = client
            .poll_owner_transaction_with_reconciliation_permit(
                owner,
                transaction_id,
                DepositWalletPollPolicy::new(1, Duration::from_millis(100)).unwrap(),
                mutation_permit_for(owner),
            )
            .await
            .unwrap_err();

        assert!(error_has_prefix(&error, RECONCILIATION_REQUIRED_PREFIX));
        assert!(client.ambiguous_submit_block(owner).is_some());
        {
            let state = client.mutation_state().unwrap();
            assert!(state
                .transaction_owners
                .get(transaction_id)
                .is_some_and(|record| record.owner == owner));
        }
        let blocked = client.get_wallet_nonce(owner).await.unwrap_err();
        assert!(error_has_prefix(&blocked, RECONCILIATION_REQUIRED_PREFIX));
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
    }

    #[tokio::test]
    async fn owner_aware_confirmed_invalid_hash_blocks_with_response_owner_evidence() {
        let owner = address(WALLET_CREATE_OWNER);
        let transaction_id = "tx-confirmed-bad-hash";
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "200 OK",
            json!({
                "transactionID": transaction_id,
                "state": "STATE_CONFIRMED",
                "transactionHash": "0xnot-a-transaction-hash",
                "owner": WALLET_CREATE_OWNER
            })
            .to_string(),
        )])
        .await;
        let client = test_client(url);

        let error = client
            .poll_owner_transaction_with_reconciliation_permit(
                owner,
                transaction_id,
                DepositWalletPollPolicy::new(1, Duration::from_millis(100)).unwrap(),
                mutation_permit_for(owner),
            )
            .await
            .unwrap_err();

        assert!(error_has_prefix(&error, RECONCILIATION_REQUIRED_PREFIX));
        assert!(client.ambiguous_submit_block(owner).is_some());
        {
            let state = client.mutation_state().unwrap();
            assert!(state
                .transaction_owners
                .get(transaction_id)
                .is_some_and(|record| record.owner == owner));
        }
        let blocked = client.get_wallet_nonce(owner).await.unwrap_err();
        assert!(error_has_prefix(&blocked, RECONCILIATION_REQUIRED_PREFIX));
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
    }

    #[tokio::test]
    async fn owner_aware_poll_requires_matching_response_owner() {
        let owner = address(WALLET_CREATE_OWNER);
        let other_owner = address("0x0000000000000000000000000000000000000001");
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "200 OK",
            json!({
                "transactionID": "tx-wrong-owner",
                "state": "STATE_NEW",
                "transactionHash": "0x38cbfbeae8fffa4e2b187ee5978d3ee9cafc53af0363ed90a35b7ea9016535d8",
                "owner": format!("{other_owner:?}")
            })
            .to_string(),
        )])
        .await;
        let client = test_client(url);

        let error = client
            .poll_owner_transaction_with_reconciliation_permit(
                owner,
                "tx-wrong-owner",
                DepositWalletPollPolicy::new(1, Duration::from_millis(100)).unwrap(),
                mutation_permit_for(owner),
            )
            .await
            .unwrap_err();
        assert!(error_has_prefix(&error, RECONCILIATION_REQUIRED_PREFIX));
        assert!(client.ambiguous_submit_block(owner).is_some());
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
    }

    #[tokio::test]
    async fn owner_aware_poll_without_evidence_requires_reconciliation_permit_before_http() {
        let owner = address(WALLET_CREATE_OWNER);
        let url = DepositWalletRelayerUrl::loopback("http://127.0.0.1:1").unwrap();
        let client = test_client(url);

        let error = client
            .poll_owner_transaction(
                owner,
                "tx-recovery-fetch-failed",
                DepositWalletPollPolicy::new(1, Duration::from_millis(100)).unwrap(),
            )
            .await
            .unwrap_err();

        assert!(error_has_prefix(&error, MUTATION_BLOCKED_PREFIX));
        assert!(client.ambiguous_submit_block(owner).is_none());
        client.ensure_owner_unblocked(owner).unwrap();
    }

    #[tokio::test]
    async fn owner_aware_recovery_permit_fetch_failure_blocks_owner() {
        let owner = address(WALLET_CREATE_OWNER);
        let (url, handle) = spawn_reset_server().await;
        let client = test_client(url);

        let error = client
            .poll_owner_transaction_with_reconciliation_permit(
                owner,
                "tx-recovery-fetch-failed",
                DepositWalletPollPolicy::new(1, Duration::from_millis(100)).unwrap(),
                mutation_permit_for(owner),
            )
            .await
            .unwrap_err();

        assert!(matches!(error, RelayerError::Http(_)));
        assert!(client.ambiguous_submit_block(owner).is_some());
        let blocked = client.get_wallet_nonce(owner).await.unwrap_err();
        assert!(error_has_prefix(&blocked, RECONCILIATION_REQUIRED_PREFIX));
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].path, "/transaction?id=tx-recovery-fetch-failed");
    }

    #[tokio::test]
    async fn owner_aware_recovery_permit_keeps_ambiguous_block_without_payload_identity() {
        let owner = address(WALLET_CREATE_OWNER);
        let transaction_id = "tx-recovered-from-ambiguous";
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "200 OK",
            transaction_response(transaction_id, "STATE_CONFIRMED"),
        )])
        .await;
        let client = test_client(url);
        client
            .record_ambiguous(owner, "payload:ambiguous-before-recovery".to_string())
            .unwrap();

        let error = client
            .poll_owner_transaction_with_reconciliation_permit(
                owner,
                transaction_id,
                DepositWalletPollPolicy::new(1, Duration::from_millis(100)).unwrap(),
                mutation_permit_for(owner),
            )
            .await
            .unwrap_err();

        assert!(error_has_prefix(&error, RECONCILIATION_REQUIRED_PREFIX));
        assert!(error.to_string().contains("ambiguous submit payload"));
        assert_eq!(
            client.ambiguous_submit_block(owner),
            Some("payload:ambiguous-before-recovery".to_string())
        );
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].path,
            "/transaction?id=tx-recovered-from-ambiguous"
        );
    }

    #[tokio::test]
    async fn owner_aware_recovery_permit_keeps_ambiguous_block_for_terminal_failures() {
        let owner = address(WALLET_CREATE_OWNER);
        for (transaction_id, state) in [
            ("tx-recovered-invalid", "STATE_INVALID"),
            ("tx-recovered-failed", "STATE_FAILED"),
        ] {
            let (url, handle) = spawn_server(vec![TestResponse::json(
                "200 OK",
                transaction_response(transaction_id, state),
            )])
            .await;
            let client = test_client(url);
            client
                .record_ambiguous(owner, "payload:ambiguous-before-recovery".to_string())
                .unwrap();

            let error = client
                .poll_owner_transaction_with_reconciliation_permit(
                    owner,
                    transaction_id,
                    DepositWalletPollPolicy::new(1, Duration::from_millis(100)).unwrap(),
                    mutation_permit_for(owner),
                )
                .await
                .unwrap_err();

            match state {
                "STATE_INVALID" => assert!(matches!(error, RelayerError::TransactionInvalid(_))),
                "STATE_FAILED" => assert!(matches!(error, RelayerError::TransactionFailed(_))),
                _ => unreachable!(),
            }
            assert_eq!(
                client.ambiguous_submit_block(owner),
                Some("payload:ambiguous-before-recovery".to_string())
            );
            let requests = handle.await.unwrap();
            assert_eq!(requests.len(), 1);
            assert_eq!(requests[0].path, format!("/transaction?id={transaction_id}"));
        }
    }

    #[tokio::test]
    async fn owner_aware_recovery_permit_blocks_same_owner_submit_while_polling() {
        let owner = address(WALLET_CREATE_OWNER);
        let transaction_id = "tx-recovery-race";
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test server should bind");
        let addr = listener.local_addr().unwrap();
        let (request_seen_tx, request_seen_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        let handle = tokio::spawn(async move {
            let (mut stream, _) = tokio::time::timeout(TEST_SERVER_TIMEOUT, listener.accept())
                .await
                .expect("server accept should not hang")
                .expect("server should accept");
            let request = read_request(&mut stream).await;
            let _ = request_seen_tx.send(());
            let _ = release_rx.await;
            write_response(
                &mut stream,
                TestResponse::json("200 OK", transaction_response(transaction_id, "STATE_NEW")),
            )
            .await;
            vec![request]
        });
        let url = DepositWalletRelayerUrl::loopback(&format!("http://{addr}")).unwrap();
        let client = test_client(url);
        let polling_client = client.clone();
        let poll = tokio::spawn(async move {
            polling_client
                .poll_owner_transaction_with_reconciliation_permit(
                    owner,
                    transaction_id,
                    DepositWalletPollPolicy::new(1, Duration::from_millis(100)).unwrap(),
                    mutation_permit_for(owner),
                )
                .await
        });
        request_seen_rx
            .await
            .expect("poll request should reach test server");

        let blocked = client.submit_wallet_create(owner, mutation_permit()).await;

        assert!(error_has_prefix(
            &blocked.unwrap_err(),
            RECONCILIATION_REQUIRED_PREFIX
        ));
        release_tx.send(()).unwrap();
        let poll_error = poll.await.unwrap().unwrap_err();
        assert!(matches!(poll_error, RelayerError::Timeout));
        assert!(client.ambiguous_submit_block(owner).is_some());
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].path, "/transaction?id=tx-recovery-race");
    }

    #[tokio::test]
    async fn owner_aware_poll_fetch_failure_blocks_known_owner_transaction() {
        let owner = address(WALLET_CREATE_OWNER);
        let transaction_id = "tx-known-fetch-failed";
        let (url, handle) = spawn_reset_server().await;
        let client = test_client(url);
        {
            let payload_hash = recovered_payload_hash(transaction_id);
            let mut state = client.mutation_state().unwrap();
            state.owner_blocks.insert(
                owner,
                OwnerMutationBlock::InFlight {
                    payload_hash: payload_hash.clone(),
                    transaction_id: Some(transaction_id.to_string()),
                },
            );
            state.transaction_owners.insert(
                transaction_id.to_string(),
                OwnerTransactionRecord {
                    owner,
                    payload_hash,
                    source: OwnerTransactionSource::LocalSubmit,
                },
            );
        }

        let error = client
            .poll_owner_transaction_with_reconciliation_permit(
                owner,
                transaction_id,
                DepositWalletPollPolicy::new(1, Duration::from_millis(100)).unwrap(),
                mutation_permit_for(owner),
            )
            .await
            .unwrap_err();

        assert!(matches!(error, RelayerError::Http(_)));
        assert!(client.ambiguous_submit_block(owner).is_some());
        let blocked = client.get_wallet_nonce(owner).await.unwrap_err();
        assert!(error_has_prefix(&blocked, RECONCILIATION_REQUIRED_PREFIX));
        client
            .clear_ambiguous_submit_after_manual_reconciliation(
                owner,
                mutation_permit_token_for(owner),
            )
            .unwrap();
        client.ensure_owner_unblocked(owner).unwrap();
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].path, "/transaction?id=tx-known-fetch-failed");
    }

    #[tokio::test]
    async fn owner_aware_poll_unknown_state_does_not_overwrite_existing_owner_block() {
        let owner = address(WALLET_CREATE_OWNER);
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "200 OK",
            transaction_response("tx-original", "STATE_NEW"),
        )])
        .await;
        let client = test_client(url);

        let receipt = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap();
        assert_eq!(receipt.transaction_id, "tx-original");

        let original_hash = {
            let state = client.mutation_state().unwrap();
            match state.owner_blocks.get(&owner) {
                Some(OwnerMutationBlock::InFlight {
                    payload_hash,
                    transaction_id: Some(transaction_id),
                }) => {
                    assert_eq!(transaction_id, "tx-original");
                    payload_hash.clone()
                }
                block => panic!("expected in-flight owner block, got {block:?}"),
            }
        };

        let error = client
            .poll_owner_transaction(
                owner,
                "tx-other",
                DepositWalletPollPolicy::new(1, Duration::from_millis(100)).unwrap(),
            )
            .await
            .unwrap_err();

        assert!(error_has_prefix(&error, RECONCILIATION_REQUIRED_PREFIX));
        {
            let state = client.mutation_state().unwrap();
            match state.owner_blocks.get(&owner) {
                Some(OwnerMutationBlock::InFlight {
                    payload_hash,
                    transaction_id: Some(transaction_id),
                }) => {
                    assert_eq!(transaction_id, "tx-original");
                    assert_eq!(payload_hash, &original_hash);
                }
                block => panic!("expected original in-flight owner block, got {block:?}"),
            }
            assert!(!state.transaction_owners.contains_key("tx-other"));
        }
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
    }

    #[tokio::test]
    async fn owner_aware_poll_unknown_state_without_local_evidence_blocks_owner() {
        let owner = address(WALLET_CREATE_OWNER);
        let transaction_id = "tx-no-local-evidence";
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "200 OK",
            transaction_response(transaction_id, "STATE_UNKNOWN_NEW"),
        )])
        .await;
        let client = test_client(url);

        let error = client
            .poll_owner_transaction_with_reconciliation_permit(
                owner,
                transaction_id,
                DepositWalletPollPolicy::new(1, Duration::from_millis(100)).unwrap(),
                mutation_permit_for(owner),
            )
            .await
            .unwrap_err();

        assert!(error_has_prefix(&error, RECONCILIATION_REQUIRED_PREFIX));
        {
            let state = client.mutation_state().unwrap();
            match state.owner_blocks.get(&owner) {
                Some(OwnerMutationBlock::Ambiguous {
                    payload_hash,
                }) => {
                    assert_eq!(payload_hash, &recovered_payload_hash(transaction_id));
                }
                block => panic!("expected recovered ambiguous owner block, got {block:?}"),
            }
            assert!(state
                .transaction_owners
                .get(transaction_id)
                .is_some_and(|record| record.owner == owner));
        }
        let blocked = client.get_wallet_nonce(owner).await.unwrap_err();
        assert!(error_has_prefix(&blocked, RECONCILIATION_REQUIRED_PREFIX));
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
    }

    #[tokio::test]
    async fn owner_aware_poll_pending_without_local_evidence_blocks_owner() {
        let owner = address(WALLET_CREATE_OWNER);
        let transaction_id = "tx-recovered-pending";
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "200 OK",
            transaction_response(transaction_id, "STATE_MINED"),
        )])
        .await;
        let client = test_client(url);

        let error = client
            .poll_owner_transaction_with_reconciliation_permit(
                owner,
                transaction_id,
                DepositWalletPollPolicy::new(1, Duration::from_millis(100)).unwrap(),
                mutation_permit_for(owner),
            )
            .await
            .unwrap_err();

        assert!(matches!(error, RelayerError::Timeout));
        {
            let state = client.mutation_state().unwrap();
            match state.owner_blocks.get(&owner) {
                Some(OwnerMutationBlock::Ambiguous {
                    payload_hash,
                }) => {
                    assert_eq!(payload_hash, &recovered_payload_hash(transaction_id));
                }
                block => panic!("expected recovered ambiguous owner block, got {block:?}"),
            }
            assert!(state
                .transaction_owners
                .get(transaction_id)
                .is_some_and(|record| record.owner == owner));
        }
        let blocked = client.get_wallet_nonce(owner).await.unwrap_err();
        assert!(error_has_prefix(&blocked, RECONCILIATION_REQUIRED_PREFIX));
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
    }

    #[tokio::test]
    async fn owner_aware_poll_unknown_state_records_reconciliation_block_after_owner_evidence() {
        let owner = address(WALLET_CREATE_OWNER);
        let transaction_id = "tx-unknown-owner";
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "200 OK",
            transaction_response(transaction_id, "STATE_UNKNOWN_NEW"),
        )])
        .await;
        let client = test_client(url);
        {
            let payload_hash = recovered_payload_hash(transaction_id);
            let mut state = client.mutation_state().unwrap();
            state.owner_blocks.insert(
                owner,
                OwnerMutationBlock::InFlight {
                    payload_hash: payload_hash.clone(),
                    transaction_id: Some(transaction_id.to_string()),
                },
            );
            state.transaction_owners.insert(
                transaction_id.to_string(),
                OwnerTransactionRecord {
                    owner,
                    payload_hash,
                    source: OwnerTransactionSource::LocalSubmit,
                },
            );
        }

        let error = client
            .poll_owner_transaction_with_reconciliation_permit(
                owner,
                transaction_id,
                DepositWalletPollPolicy::new(1, Duration::from_millis(100)).unwrap(),
                mutation_permit_for(owner),
            )
            .await
            .unwrap_err();

        assert!(error_has_prefix(&error, RECONCILIATION_REQUIRED_PREFIX));
        assert!(client.ambiguous_submit_block(owner).is_some());
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
    }

    #[tokio::test]
    async fn local_transaction_poll_unknown_state_keeps_owner_blocked_for_reconciliation() {
        let owner = address(WALLET_CREATE_OWNER);
        let (url, handle) = spawn_server(vec![
            TestResponse::json("200 OK", transaction_response("tx-local-unknown", "STATE_NEW")),
            TestResponse::json(
                "200 OK",
                transaction_response("tx-local-unknown", "STATE_UNKNOWN_NEW"),
            ),
        ])
        .await;
        let client = test_client(url);

        let receipt = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap();
        assert_eq!(receipt.transaction_id, "tx-local-unknown");

        let error = client
            .poll_transaction(
                "tx-local-unknown",
                DepositWalletPollPolicy::new(1, Duration::from_millis(100)).unwrap(),
            )
            .await
            .unwrap_err();

        assert!(error_has_prefix(&error, RECONCILIATION_REQUIRED_PREFIX));
        assert!(client.ambiguous_submit_block(owner).is_some());
        let blocked = client.get_wallet_nonce(owner).await.unwrap_err();
        assert!(error_has_prefix(&blocked, RECONCILIATION_REQUIRED_PREFIX));
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 2);
    }

    #[tokio::test]
    async fn local_transaction_poll_requires_response_owner_before_clearing_block() {
        let owner = address(WALLET_CREATE_OWNER);
        let other_owner = address("0x0000000000000000000000000000000000000001");
        let (url, handle) = spawn_server(vec![
            TestResponse::json("200 OK", transaction_response("tx-owner-mismatch", "STATE_NEW")),
            TestResponse::json(
                "200 OK",
                json!({
                    "transactionID": "tx-owner-mismatch",
                    "state": "STATE_CONFIRMED",
                    "transactionHash": "0x38cbfbeae8fffa4e2b187ee5978d3ee9cafc53af0363ed90a35b7ea9016535d8",
                    "owner": format!("{other_owner:?}")
                })
                .to_string(),
            ),
        ])
        .await;
        let client = test_client(url);

        let receipt = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap();
        assert_eq!(receipt.transaction_id, "tx-owner-mismatch");

        let error = client
            .poll_transaction(
                "tx-owner-mismatch",
                DepositWalletPollPolicy::new(1, Duration::from_millis(100)).unwrap(),
            )
            .await
            .unwrap_err();

        assert!(error_has_prefix(&error, RECONCILIATION_REQUIRED_PREFIX));
        assert!(client.ambiguous_submit_block(owner).is_some());
        let blocked = client.get_wallet_nonce(owner).await.unwrap_err();
        assert!(error_has_prefix(&blocked, RECONCILIATION_REQUIRED_PREFIX));
        client
            .clear_ambiguous_submit_after_manual_reconciliation(
                owner,
                mutation_permit_token_for(owner),
            )
            .unwrap();
        client.ensure_owner_unblocked(owner).unwrap();
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 2);
    }

    #[tokio::test]
    async fn local_transaction_poll_parse_error_marks_inflight_block_reconciliation_required() {
        let owner = address(WALLET_CREATE_OWNER);
        let (url, handle) = spawn_server(vec![
            TestResponse::json("200 OK", transaction_response("tx-malformed", "STATE_NEW")),
            TestResponse::json("200 OK", "{\"transactionID\":\"tx-malformed\""),
        ])
        .await;
        let client = test_client(url);

        let receipt = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap();
        assert_eq!(receipt.transaction_id, "tx-malformed");

        let error = client
            .poll_transaction(
                "tx-malformed",
                DepositWalletPollPolicy::new(1, Duration::from_millis(100)).unwrap(),
            )
            .await
            .unwrap_err();

        assert!(matches!(error, RelayerError::Other(_)));
        assert!(client.ambiguous_submit_block(owner).is_some());
        let blocked = client.get_wallet_nonce(owner).await.unwrap_err();
        assert!(error_has_prefix(&blocked, RECONCILIATION_REQUIRED_PREFIX));
        client
            .clear_ambiguous_submit_after_manual_reconciliation(
                owner,
                mutation_permit_token_for(owner),
            )
            .unwrap();
        client.ensure_owner_unblocked(owner).unwrap();
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 2);
    }

    #[tokio::test]
    async fn confirmed_poll_clears_owner_inflight_block() {
        let owner = address(WALLET_CREATE_OWNER);
        let (url, handle) = spawn_server(vec![
            TestResponse::json("200 OK", transaction_response("tx-confirmed", "STATE_NEW")),
            TestResponse::json(
                "200 OK",
                transaction_response("tx-confirmed", "STATE_CONFIRMED"),
            ),
            TestResponse::json("200 OK", json!({"nonce": "33"}).to_string()),
        ])
        .await;
        let client = test_client(url);

        let receipt = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap();
        assert_eq!(receipt.transaction_id, "tx-confirmed");
        let blocked = client.get_wallet_nonce(owner).await.unwrap_err();
        assert!(error_has_prefix(&blocked, RECONCILIATION_REQUIRED_PREFIX));

        let receipt = client
            .poll_transaction(
                "tx-confirmed",
                DepositWalletPollPolicy::new(1, Duration::from_millis(100)).unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(receipt.state, RelayerTransactionState::Confirmed);

        let nonce = client.get_wallet_nonce(owner).await.unwrap();
        assert_eq!(nonce, U256::from(33u64));

        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 3);
    }

    #[tokio::test]
    async fn confirmed_poll_without_transaction_hash_keeps_owner_block_for_reconciliation() {
        let owner = address(WALLET_CREATE_OWNER);
        let (url, handle) = spawn_server(vec![
            TestResponse::json("200 OK", transaction_response("tx-no-hash", "STATE_NEW")),
            TestResponse::json(
                "200 OK",
                json!({
                    "transactionID": "tx-no-hash",
                    "state": "STATE_CONFIRMED",
                    "owner": WALLET_CREATE_OWNER
                })
                .to_string(),
            ),
        ])
        .await;
        let client = test_client(url);

        let receipt = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap();
        assert_eq!(receipt.transaction_id, "tx-no-hash");

        let error = client
            .poll_transaction(
                "tx-no-hash",
                DepositWalletPollPolicy::new(1, Duration::from_millis(100)).unwrap(),
            )
            .await
            .unwrap_err();
        assert!(error_has_prefix(&error, RECONCILIATION_REQUIRED_PREFIX));
        assert!(client.ambiguous_submit_block(owner).is_some());

        let blocked = client.get_wallet_nonce(owner).await.unwrap_err();
        assert!(error_has_prefix(&blocked, RECONCILIATION_REQUIRED_PREFIX));

        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 2);
    }

    #[tokio::test]
    async fn terminal_error_poll_clears_owner_inflight_block() {
        let owner = address(WALLET_CREATE_OWNER);

        for (transaction_id, terminal_state, expected_error) in [
            (
                "tx-terminal-invalid",
                "STATE_INVALID",
                "transaction invalid",
            ),
            ("tx-terminal-failed", "STATE_FAILED", "transaction failed"),
        ] {
            let (url, handle) = spawn_server(vec![
                TestResponse::json("200 OK", transaction_response(transaction_id, "STATE_NEW")),
                TestResponse::json("200 OK", transaction_response(transaction_id, terminal_state)),
                TestResponse::json("200 OK", json!({"nonce": "37"}).to_string()),
            ])
            .await;
            let client = test_client(url);

            let receipt = client
                .submit_wallet_create(owner, mutation_permit())
                .await
                .unwrap();
            assert_eq!(receipt.transaction_id, transaction_id);
            assert!(client.get_wallet_nonce(owner).await.is_err());

            let error = client
                .poll_transaction(
                    transaction_id,
                    DepositWalletPollPolicy::new(1, Duration::from_millis(100)).unwrap(),
                )
                .await
                .unwrap_err();
            assert!(
                matches!(
                    (&expected_error, &error),
                    (&"transaction invalid", RelayerError::TransactionInvalid(_))
                        | (&"transaction failed", RelayerError::TransactionFailed(_))
                ),
                "expected {expected_error}, got {error:?}"
            );

            client.ensure_owner_unblocked(owner).unwrap();
            {
                let state = client.mutation_state().unwrap();
                assert!(!state.transaction_owners.contains_key(transaction_id));
            }
            let nonce = client.get_wallet_nonce(owner).await.unwrap();
            assert_eq!(nonce, U256::from(37u64));

            let requests = handle.await.unwrap();
            assert_eq!(requests.len(), 3);
        }
    }

    #[test]
    fn manual_clear_removes_only_matching_owner_transaction_records() {
        let owner = address(WALLET_CREATE_OWNER);
        let other_owner = address("0x0000000000000000000000000000000000000001");
        let client =
            test_client(DepositWalletRelayerUrl::loopback("http://127.0.0.1:1").unwrap());
        {
            let mut state = client.mutation_state().unwrap();
            state.owner_blocks.insert(
                owner,
                OwnerMutationBlock::Ambiguous {
                    payload_hash: "payload-owner".to_string(),
                },
            );
            state.transaction_owners.insert(
                "tx-owner-stale".to_string(),
                OwnerTransactionRecord {
                    owner,
                    payload_hash: "payload-owner".to_string(),
                    source: OwnerTransactionSource::LocalSubmit,
                },
            );
            state.transaction_owners.insert(
                "tx-other-live".to_string(),
                OwnerTransactionRecord {
                    owner: other_owner,
                    payload_hash: "payload-other".to_string(),
                    source: OwnerTransactionSource::LocalSubmit,
                },
            );
        }

        client
            .clear_ambiguous_submit_after_manual_reconciliation(
                owner,
                mutation_permit_token_for(owner),
            )
            .unwrap();

        client.ensure_owner_unblocked(owner).unwrap();
        let state = client.mutation_state().unwrap();
        assert!(!state.transaction_owners.contains_key("tx-owner-stale"));
        assert_eq!(
            state
                .transaction_owners
                .get("tx-other-live")
                .map(|record| record.owner),
            Some(other_owner)
        );
    }

    #[tokio::test]
    async fn manual_clear_rejects_known_inflight_submit_until_terminal_poll() {
        let owner = address(WALLET_CREATE_OWNER);
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "200 OK",
            transaction_response("tx-known", "STATE_NEW"),
        )])
        .await;
        let client = test_client(url);

        let receipt = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap();
        assert_eq!(receipt.transaction_id, "tx-known");

        let error = client
            .clear_ambiguous_submit_after_manual_reconciliation(
                owner,
                mutation_permit_token_for(owner),
            )
            .unwrap_err();
        assert!(error_has_prefix(&error, MUTATION_BLOCKED_PREFIX));

        let duplicate = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap_err();
        assert!(error_has_prefix(&duplicate, RECONCILIATION_REQUIRED_PREFIX));

        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
    }
