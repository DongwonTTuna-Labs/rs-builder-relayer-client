use super::*;

#[tokio::test]
    async fn get_wallet_nonce_sends_exact_path_and_parses_decimal_nonce() {
        let expected = fixture_value("wallet_nonce_request.json");
        let (url, handle) =
            spawn_server(vec![TestResponse::json("200 OK", json!({"nonce": "31"}).to_string())])
                .await;
        let client = test_client(url);
        let owner: Address = expected["address"].as_str().unwrap().parse().unwrap();

        let nonce = client.get_wallet_nonce(owner, wallet_nonce_read_permit_for(owner)).await.unwrap();

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
    async fn get_wallet_nonce_accepts_numeric_nonce_response() {
        let (url, handle) =
            spawn_server(vec![TestResponse::json("200 OK", json!({"nonce": 31}).to_string())])
                .await;
        let client = test_client(url);
        let owner = address(WALLET_CREATE_OWNER);

        let nonce = client.get_wallet_nonce(owner, wallet_nonce_read_permit_for(owner)).await.unwrap();

        assert_eq!(nonce, U256::from(31u64));
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
    }

#[tokio::test]
    async fn get_wallet_nonce_requires_owner_scoped_permit_before_http() {
        let url = DepositWalletRelayerUrl::loopback("http://127.0.0.1:1").unwrap();
        let client = test_client(url);
        let owner = address(WALLET_CREATE_OWNER);

        let error = client
            .get_wallet_nonce(owner, DepositWalletMutationGate::Deny)
            .await
            .unwrap_err();

        assert!(error_has_prefix(&error, MUTATION_BLOCKED_PREFIX));
    }

#[test]
    fn wallet_nonce_parser_accepts_u256_string_and_number_equivalently() {
        let raw = "18446744073709551616";
        let expected = U256::from_dec_str(raw).unwrap();
        let max = U256::MAX.to_string();

        let string_nonce =
            super::super::read::parse_wallet_nonce_value(json!(raw)).unwrap();
        let numeric_nonce = super::super::read::parse_wallet_nonce_value(
            serde_json::from_str::<serde_json::Value>(raw).unwrap(),
        )
        .unwrap();

        assert_eq!(string_nonce, expected);
        assert_eq!(numeric_nonce, expected);
        assert_eq!(
            super::super::read::parse_wallet_nonce_value(
                serde_json::from_str::<serde_json::Value>(&max).unwrap()
            )
            .unwrap(),
            U256::MAX
        );
    }

#[test]
    fn wallet_nonce_parser_rejects_invalid_boundaries() {
        let too_large = format!("{}0", U256::MAX);
        for value in [
            json!("not-decimal"),
            json!("-1"),
            serde_json::from_str::<serde_json::Value>("-1").unwrap(),
            serde_json::from_str::<serde_json::Value>("1.5").unwrap(),
            serde_json::from_str::<serde_json::Value>("1e3").unwrap(),
            json!(too_large.clone()),
            serde_json::from_str::<serde_json::Value>(&too_large).unwrap(),
        ] {
            assert!(super::super::read::parse_wallet_nonce_value(value).is_err());
        }
    }

#[tokio::test]
    async fn get_wallet_nonce_serializes_same_owner_mutations_until_response() {
        let owner = address(WALLET_CREATE_OWNER);
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
                TestResponse::json("200 OK", json!({"nonce": "31"}).to_string()),
            )
            .await;
            vec![request]
        });
        let url = DepositWalletRelayerUrl::loopback(&format!("http://{addr}")).unwrap();
        let client = test_client(url);
        let request_client = client.clone();
        let nonce_task = tokio::spawn(async move { request_client.get_wallet_nonce(owner, wallet_nonce_read_permit_for(owner)).await });
        request_seen_rx
            .await
            .expect("nonce request should reach test server");

        let second_nonce = client.get_wallet_nonce(owner, wallet_nonce_read_permit_for(owner)).await.unwrap_err();
        assert!(error_has_prefix(&second_nonce, MUTATION_BLOCKED_PREFIX));
        let blocked = match client.reserve_owner_submit(owner, "payload:nonce-race".to_string()) {
            Ok(_) => panic!("nonce read should block same-owner submit reservation"),
            Err(error) => error,
        };
        assert!(error_has_prefix(&blocked, MUTATION_BLOCKED_PREFIX));
        release_tx.send(()).unwrap();

        let nonce = nonce_task.await.unwrap().unwrap();
        assert_eq!(nonce, U256::from(31u64));
        client.ensure_owner_unblocked(owner).unwrap();
        let mut reservation = client
            .reserve_owner_submit(owner, "payload:after-nonce".to_string())
            .unwrap();
        reservation.clear().unwrap();
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].path.starts_with("/nonce?"));
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
    async fn get_transaction_accepts_array_alias_and_exact_item_limit() {
        let mut body = (0..MAX_TRANSACTION_RESPONSE_ITEMS)
            .map(|index| {
                json!({
                    "transactionID": format!("other-tx-{index}"),
                    "state": "STATE_FAILED",
                    "transactionHash": "0x38cbfbeae8fffa4e2b187ee5978d3ee9cafc53af0363ed90a35b7ea9016535d8"
                })
            })
            .collect::<Vec<_>>();
        body[MAX_TRANSACTION_RESPONSE_ITEMS - 1] = json!({
            "transactionId": "tx-array-alias",
            "state": "STATE_CONFIRMED",
            "transactionHash": "0x38cbfbeae8fffa4e2b187ee5978d3ee9cafc53af0363ed90a35b7ea9016535d8"
        });
        let (url, handle) =
            spawn_server(vec![TestResponse::json("200 OK", json!(body).to_string())]).await;
        let client = test_client(url);

        let receipt = client.get_transaction("tx-array-alias").await.unwrap();

        assert_eq!(receipt.transaction_id, "tx-array-alias");
        assert_eq!(receipt.state, RelayerTransactionState::Confirmed);
        let requests = handle.await.unwrap();
        assert_eq!(requests[0].path, "/transaction?id=tx-array-alias");
    }

#[test]
    fn transaction_array_parser_rejects_missing_duplicate_invalid_and_limit_cases() {
        let item = |transaction_id: String| {
            json!({
                "transactionID": transaction_id,
                "state": "STATE_CONFIRMED",
                "transactionHash": "0x38cbfbeae8fffa4e2b187ee5978d3ee9cafc53af0363ed90a35b7ea9016535d8"
            })
        };
        let target = "tx-array-boundary";
        let missing = json!([item("other-tx".to_string())]).to_string();
        let duplicate = json!([item(target.to_string()), item(target.to_string())]).to_string();
        let invalid = json!([item("bad\ntransaction".to_string())]).to_string();
        let oversized = json!(
            (0..=MAX_TRANSACTION_RESPONSE_ITEMS)
                .map(|index| item(format!("other-tx-{index}")))
                .collect::<Vec<_>>()
        )
        .to_string();

        for (label, body, retryable_absence, expected_message) in [
            ("missing", missing, true, "did not include requested transaction id"),
            ("duplicate", duplicate, false, "duplicate requested transaction id"),
            ("invalid", invalid, false, "invalid transactionID"),
            ("oversized", oversized, false, "more than"),
        ] {
            let parse_error = parse_transaction_response(target, body.as_bytes()).unwrap_err();
            assert_eq!(
                parse_error.retryable_absence, retryable_absence,
                "{label} array response retryable absence classification changed"
            );
            let error = parse_error.error;
            assert!(error.is_deposit_wallet_reconciliation_required());
            assert!(error.to_string().contains(expected_message), "{label}: {error}");
            assert!(!error.to_string().contains(target));
        }
    }

#[tokio::test]
    async fn get_transaction_is_read_only_and_does_not_clear_owner_blocks() {
        let owner = address(WALLET_CREATE_OWNER);
        let (url, handle) = spawn_server(vec![
            TestResponse::json(
                "200 OK",
                json!({"transactionID": "", "state": "STATE_NEW"}).to_string(),
            ),
            TestResponse::json(
                "200 OK",
                transaction_response("tx-read-only", "STATE_CONFIRMED"),
            ),
        ])
        .await;
        let client = test_client(url);

        let submit_error = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap_err();
        assert!(error_has_prefix(&submit_error, AMBIGUOUS_SUBMIT_PREFIX));
        let payload_hash = client.ambiguous_submit_block(owner).unwrap();

        let receipt = client.get_transaction("tx-read-only").await.unwrap();

        assert_eq!(receipt.transaction_id, "tx-read-only");
        assert_eq!(receipt.owner, Some(owner));
        assert_eq!(client.ambiguous_submit_block(owner), Some(payload_hash));
        let blocked = client.get_wallet_nonce(owner, wallet_nonce_read_permit_for(owner)).await.unwrap_err();
        assert!(error_has_prefix(&blocked, RECONCILIATION_REQUIRED_PREFIX));
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[1].path, "/transaction?id=tx-read-only");
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
        assert!(error.to_string().contains("id hash"));
        assert!(!error.to_string().contains("other-tx"));
        assert!(!error.to_string().contains("tx-array"));
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
        assert!(error.to_string().contains("id hash"));
        assert!(!error.to_string().contains("other-tx"));
        assert!(!error.to_string().contains("tx-array"));
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
        assert_eq!(
            validate_transaction_id("tx:abc/123+query=value").unwrap(),
            "tx:abc/123+query=value"
        );

        let too_long = "a".repeat(MAX_TRANSACTION_ID_LEN + 1);
        assert!(validate_transaction_id(&too_long).is_err());
        assert!(validate_transaction_id("").is_err());
        assert!(validate_transaction_id("tx abc").is_err());
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
        let rendered = format!("{receipt:?}");
        assert!(!rendered.contains("STATE_WEIRD"));
        assert!(rendered.contains("<unrecognized relayer state>"));

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

        let error = client.get_wallet_nonce(address(WALLET_CREATE_OWNER), wallet_nonce_read_permit_for(address(WALLET_CREATE_OWNER))).await.unwrap_err();
        assert!(matches!(error, RelayerError::Other(message) if message.contains("maximum size")));
        let _ = handle.await.unwrap();

        let (url, handle) = spawn_server(vec![TestResponse::json_without_content_length(
            "200 OK",
            "x".repeat(MAX_SUCCESS_BODY_BYTES + 1),
        )])
        .await;
        let client = test_client(url);

        let error = client
            .get_wallet_nonce(address(WALLET_CREATE_OWNER), wallet_nonce_read_permit_for(address(WALLET_CREATE_OWNER)))
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
        assert_eq!(receipt.owner, Some(address(WALLET_CREATE_OWNER)));
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
        let rendered = format!("{:?}", parsed.receipt);
        assert!(!rendered.contains("0x6e0c80c90ea6c15917308f820eac91ce2724b5b5"));
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
                .get_wallet_nonce(address(WALLET_CREATE_OWNER), wallet_nonce_read_permit_for(address(WALLET_CREATE_OWNER)))
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
        assert_eq!(receipt.owner, Some(address(WALLET_CREATE_OWNER)));
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 2);
    }
