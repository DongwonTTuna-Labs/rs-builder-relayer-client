use super::*;

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
    async fn get_wallet_nonce_accepts_numeric_nonce_response() {
        let (url, handle) =
            spawn_server(vec![TestResponse::json("200 OK", json!({"nonce": 31}).to_string())])
                .await;
        let client = test_client(url);
        let owner = address(WALLET_CREATE_OWNER);

        let nonce = client.get_wallet_nonce(owner).await.unwrap();

        assert_eq!(nonce, U256::from(31u64));
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
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
        assert_eq!(receipt.owner, None);
        assert_eq!(client.ambiguous_submit_block(owner), Some(payload_hash));
        let blocked = client.get_wallet_nonce(owner).await.unwrap_err();
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
        assert_eq!(receipt.owner, None);
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
        assert_eq!(parsed.receipt.owner, None);
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
        assert_eq!(receipt.owner, None);
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 2);
    }
