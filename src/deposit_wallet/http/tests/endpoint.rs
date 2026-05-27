use super::*;

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

        let builder = BuilderConfig::new(
            "builder-key-secret",
            "builder-hmac-secret",
            "builder-passphrase-secret",
        );
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

        let error = client.get_wallet_nonce(signed.owner(), wallet_nonce_read_permit_for(signed.owner())).await.unwrap_err();
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

        let error = client.get_wallet_nonce(owner, wallet_nonce_read_permit_for(owner)).await.unwrap_err();
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

        let error = client.get_wallet_nonce(owner, wallet_nonce_read_permit_for(owner)).await.unwrap_err();
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

        let client_task = tokio::spawn(async move { client.get_wallet_nonce(owner, wallet_nonce_read_permit_for(owner)).await });
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
        let client_task = tokio::spawn(async move { client_for_request.get_wallet_nonce(owner, wallet_nonce_read_permit_for(owner)).await });
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
    async fn non_success_status_survives_truncated_error_body() {
        let (url, handle) = spawn_truncated_error_body_server("400 Bad Request").await;
        let client = test_client(url);

        let error = tokio::time::timeout(
            Duration::from_secs(1),
            client.get_wallet_nonce(address(WALLET_CREATE_OWNER), wallet_nonce_read_permit_for(address(WALLET_CREATE_OWNER))),
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
            client.get_wallet_nonce(address(WALLET_CREATE_OWNER), wallet_nonce_read_permit_for(address(WALLET_CREATE_OWNER))),
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

        let error = client.get_wallet_nonce(owner, wallet_nonce_read_permit_for(owner)).await.unwrap_err();
        assert!(matches!(error, RelayerError::Api { status: 503, .. }));
        assert!(error.to_string().contains("retry after 7s"));

        let error = client.get_wallet_nonce(owner, wallet_nonce_read_permit_for(owner)).await.unwrap_err();
        assert!(matches!(error, RelayerError::Api { status: 503, .. }));
        assert!(error.to_string().contains("retry after "));

        let error = client.get_wallet_nonce(owner, wallet_nonce_read_permit_for(owner)).await.unwrap_err();
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
    async fn redirects_are_not_followed_and_target_gets_no_auth() {
        let target_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        target_listener.set_nonblocking(true).unwrap();
        let target_addr = target_listener.local_addr().unwrap();

        let (url, handle) = spawn_server(vec![TestResponse::redirect(format!(
            "http://{target_addr}/redirect-target"
        ))])
        .await;
        let client = test_client(url);

        let error = client
            .get_wallet_nonce(address(WALLET_CREATE_OWNER), wallet_nonce_read_permit_for(address(WALLET_CREATE_OWNER)))
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
