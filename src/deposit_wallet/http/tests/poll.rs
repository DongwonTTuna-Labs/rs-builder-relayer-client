use super::*;

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
        assert!(client.ambiguous_submit_block(owner).is_none());
        client.ensure_owner_unblocked(owner).unwrap();
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
        assert!(client.ambiguous_submit_block(owner).is_none());
        client.ensure_owner_unblocked(owner).unwrap();
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
        assert!(client.ambiguous_submit_block(owner).is_none());
        client.ensure_owner_unblocked(owner).unwrap();
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
    async fn owner_aware_recovery_permit_fetch_failure_without_owner_evidence_does_not_block_owner() {
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
        assert!(client.ambiguous_submit_block(owner).is_none());
        client.ensure_owner_unblocked(owner).unwrap();
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

            assert!(error_has_prefix(&error, RECONCILIATION_REQUIRED_PREFIX));
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
    async fn owner_aware_recovery_permit_waits_for_response_owner_before_blocking_owner() {
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

        assert!(client.ambiguous_submit_block(owner).is_none());
        client.ensure_owner_unblocked(owner).unwrap();
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
                submit_reconciliation_evidence_for(&client, owner),
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
                submit_reconciliation_evidence_for(&client, owner),
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
                submit_reconciliation_evidence_for(&client, owner),
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
