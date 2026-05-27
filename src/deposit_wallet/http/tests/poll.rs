use super::*;

#[tokio::test]
    async fn polling_keeps_mined_pending_until_confirmed_and_uses_injected_sleeper() {
        let (result, requests, sleeper, policy) = poll_sequence(
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
        assert_eq!(
            sleeps,
            vec![
                policy.interval_for_transaction_attempt("tx-123", 0),
                policy.interval_for_transaction_attempt("tx-123", 1),
                policy.interval_for_transaction_attempt("tx-123", 2),
            ]
        );
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
        let (url, handle) = spawn_server(vec![
            TestResponse::json("503 Service Unavailable", "{}")
                .with_header("retry-after", "120"),
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
        assert_eq!(sleeps, vec![Duration::from_secs(120)]);
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 2);
    }

#[tokio::test]
    async fn owner_aware_poll_final_retryable_failure_marks_known_transaction_ambiguous() {
        let owner = address(WALLET_CREATE_OWNER);
        let transaction_id = "tx-final-retryable-failure";
        let payload_hash = "payload:final-retryable-failure";
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "429 Too Many Requests",
            "{}",
        )
        .with_header("retry-after", "1")])
        .await;
        let client = test_client(url);
        client
            .record_inflight_transaction(
                owner,
                payload_hash.to_string(),
                transaction_id.to_string(),
            )
            .unwrap();

        let error = client
            .poll_owner_transaction(
                owner,
                transaction_id,
                DepositWalletPollPolicy::new(1, Duration::from_millis(100)).unwrap(),
            )
            .await
            .unwrap_err();

        assert!(matches!(error, RelayerError::QuotaExhausted));
        assert_eq!(
            client.ambiguous_submit_block(owner),
            Some(payload_hash.to_string())
        );
        {
            let state = client.mutation_state().unwrap();
            assert!(matches!(
                state.owner_blocks.get(&owner),
                Some(OwnerMutationBlock::Ambiguous {
                    payload_hash: current,
                    ..
                }) if current == payload_hash
            ));
            assert!(state.transaction_owners.contains_key(transaction_id));
        }
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
    }

#[tokio::test]
    async fn transient_poll_429_retry_after_uses_larger_policy_or_server_delay() {
        for (transaction_id, retry_after, interval, expected_sleep) in [
            (
                "tx-retry-after-header",
                "1".to_string(),
                Duration::from_millis(100),
                Duration::from_secs(1),
            ),
            (
                "tx-retry-after-long",
                "120".to_string(),
                Duration::from_millis(100),
                Duration::from_secs(120),
            ),
            (
                "tx-retry-after-capped",
                "600".to_string(),
                Duration::from_millis(100),
                MAX_RETRY_AFTER_INTERVAL,
            ),
        ] {
            let owner = address(WALLET_CREATE_OWNER);
            let (url, handle) = spawn_server(vec![
                TestResponse::json("429 Too Many Requests", "{}")
                    .with_header("retry-after", &retry_after),
                TestResponse::json(
                    "200 OK",
                    transaction_response(transaction_id, "STATE_CONFIRMED"),
                ),
            ])
            .await;
            let sleeper = Arc::new(RecordingSleeper::default());
            let client = test_client_with_sleeper(url, sleeper.clone());
            client
                .record_inflight_transaction(
                    owner,
                    format!("payload:{transaction_id}"),
                    transaction_id.to_string(),
                )
                .unwrap();

            let receipt = client
                .poll_owner_transaction(
                    owner,
                    transaction_id,
                    DepositWalletPollPolicy::new(2, interval).unwrap(),
                )
                .await
                .unwrap();

            assert_eq!(receipt.state, RelayerTransactionState::Confirmed);
            assert_eq!(sleeper.sleeps(), vec![expected_sleep]);
            let requests = handle.await.unwrap();
            assert_eq!(requests.len(), 2);
        }

        let owner = address(WALLET_CREATE_OWNER);
        let transaction_id = "tx-retry-after-policy";
        let policy = DepositWalletPollPolicy::new(2, Duration::from_secs(2)).unwrap();
        let expected_sleep = policy.interval_for_transaction_attempt(transaction_id, 0);
        let (url, handle) = spawn_server(vec![
            TestResponse::json("429 Too Many Requests", "{}").with_header("retry-after", "1"),
            TestResponse::json(
                "200 OK",
                transaction_response(transaction_id, "STATE_CONFIRMED"),
            ),
        ])
        .await;
        let sleeper = Arc::new(RecordingSleeper::default());
        let client = test_client_with_sleeper(url, sleeper.clone());
        client
            .record_inflight_transaction(
                owner,
                "payload:retry-after-policy".to_string(),
                transaction_id.to_string(),
            )
            .unwrap();

        let receipt = client
            .poll_owner_transaction(owner, transaction_id, policy)
            .await
            .unwrap();

        assert_eq!(receipt.state, RelayerTransactionState::Confirmed);
        assert_eq!(sleeper.sleeps(), vec![expected_sleep]);
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 2);
    }

#[test]
    fn transaction_poll_jitter_is_deterministic_bounded_and_capped() {
        let base = Duration::from_millis(200);
        let first = super::super::poll::transaction_poll_jitter("tx-jitter-a", 0, base);

        assert_eq!(
            first,
            super::super::poll::transaction_poll_jitter("tx-jitter-a", 0, base)
        );
        assert!((Duration::from_millis(1)..=Duration::from_millis(50)).contains(&first));
        assert_eq!(
            super::super::poll::transaction_poll_jitter("tx-jitter-a", 0, Duration::ZERO),
            Duration::ZERO
        );
        let variants = [
            super::super::poll::transaction_poll_jitter("tx-jitter-a", 1, base),
            super::super::poll::transaction_poll_jitter("tx-jitter-b", 0, base),
            super::super::poll::transaction_poll_jitter("tx-jitter-c", 2, base),
        ];
        assert!(variants.iter().any(|candidate| *candidate != first));

        let capped = DepositWalletPollPolicy::new(5, MAX_POLL_INTERVAL).unwrap();
        assert_eq!(
            capped.interval_for_transaction_attempt("tx-jitter-capped", 1),
            MAX_POLL_INTERVAL
        );
    }

#[tokio::test]
    async fn poll_retries_absent_transaction_array_and_404_before_confirmed() {
        let (url, handle) = spawn_server(vec![
            TestResponse::json("200 OK", json!([]).to_string()),
            TestResponse::json("404 Not Found", "{}"),
            TestResponse::json(
                "200 OK",
                transaction_response("tx-delayed-visibility", "STATE_CONFIRMED"),
            ),
        ])
        .await;
        let sleeper = Arc::new(RecordingSleeper::default());
        let client = test_client_with_sleeper(url, sleeper.clone());
        let policy = DepositWalletPollPolicy::new(3, Duration::from_millis(100)).unwrap();

        let receipt = client
            .poll_transaction("tx-delayed-visibility", policy.clone())
            .await
            .unwrap();

        assert_eq!(receipt.state, RelayerTransactionState::Confirmed);
        assert_eq!(
            sleeper.sleeps(),
            vec![
                policy.interval_for_transaction_attempt("tx-delayed-visibility", 0),
                policy.interval_for_transaction_attempt("tx-delayed-visibility", 1),
            ]
        );
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 3);
        assert!(requests
            .iter()
            .all(|request| request.path == "/transaction?id=tx-delayed-visibility"));
    }

#[tokio::test]
    async fn poll_policy_rejects_invalid_bounds_and_caps_backoff() {
        assert!(DepositWalletPollPolicy::new(0, Duration::from_secs(1)).is_err());
        assert!(DepositWalletPollPolicy::new(1, Duration::from_millis(99)).is_err());
        assert!(
            DepositWalletPollPolicy::new(MAX_POLL_ATTEMPTS + 1, Duration::from_secs(1)).is_err()
        );

        let default_policy = DepositWalletPollPolicy::default();
        assert_eq!(default_policy.max_attempts, 5);
        assert_eq!(default_policy.interval, Duration::from_secs(1));

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
            .poll_owner_transaction(owner, "tx-pending", policy)
            .await
            .unwrap_err();
        assert!(error_has_prefix(&error, RECONCILIATION_REQUIRED_PREFIX));

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
        assert!(error_has_prefix(&timeout, RECONCILIATION_REQUIRED_PREFIX));
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
        assert_eq!(receipt.owner, Some(owner));

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

        assert!(error_has_prefix(&error, RECONCILIATION_REQUIRED_PREFIX));
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
                owner_recovery_poll_permit_for(owner),
            )
            .await
            .unwrap_err();
        assert!(error_has_prefix(&error, RECONCILIATION_REQUIRED_PREFIX));

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
                owner_recovery_poll_permit_for(owner),
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
                owner_recovery_poll_permit_for(owner),
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
        let blocked = client.get_wallet_nonce(owner).await.unwrap_err();
        assert!(error_has_prefix(&blocked, RECONCILIATION_REQUIRED_PREFIX));
        {
            let state = client.mutation_state().unwrap();
            assert!(matches!(
                state.owner_blocks.get(&owner),
                Some(OwnerMutationBlock::Ambiguous { payload_hash, .. })
                    if payload_hash == "payload:known-bad-owner"
            ));
        }
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
                owner_recovery_poll_permit_for(owner),
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
                owner_recovery_poll_permit_for(owner),
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
                owner_recovery_poll_permit_for(owner),
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
    async fn owner_aware_poll_does_not_use_owner_from_mismatched_transaction_response() {
        let owner = address(WALLET_CREATE_OWNER);
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "200 OK",
            json!({
                "transactionID": "other-tx",
                "state": "STATE_FAILED",
                "transactionHash": "0x38cbfbeae8fffa4e2b187ee5978d3ee9cafc53af0363ed90a35b7ea9016535d8",
                "owner": WALLET_CREATE_OWNER
            })
            .to_string(),
        )])
        .await;
        let client = test_client(url);

        let error = client
            .poll_owner_transaction_with_reconciliation_permit(
                owner,
                "tx-requested",
                DepositWalletPollPolicy::new(1, Duration::from_millis(100)).unwrap(),
                owner_recovery_poll_permit_for(owner),
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
    async fn owner_aware_poll_mismatched_transaction_response_keeps_known_owner_blocked() {
        let owner = address(WALLET_CREATE_OWNER);
        let transaction_id = "tx-requested-known";
        let (url, handle) = spawn_server(vec![
            TestResponse::json("200 OK", transaction_response(transaction_id, "STATE_NEW")),
            TestResponse::json(
                "200 OK",
                json!({
                    "transactionID": "other-tx",
                    "state": "STATE_CONFIRMED",
                    "transactionHash": "0x38cbfbeae8fffa4e2b187ee5978d3ee9cafc53af0363ed90a35b7ea9016535d8",
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
        assert_eq!(receipt.transaction_id, transaction_id);

        let error = client
            .poll_owner_transaction(
                owner,
                transaction_id,
                DepositWalletPollPolicy::new(1, Duration::from_millis(100)).unwrap(),
            )
            .await
            .unwrap_err();

        assert!(error_has_prefix(&error, RECONCILIATION_REQUIRED_PREFIX));
        let blocked = client.get_wallet_nonce(owner).await.unwrap_err();
        assert!(error_has_prefix(&blocked, RECONCILIATION_REQUIRED_PREFIX));
        {
            let state = client.mutation_state().unwrap();
            assert!(matches!(
                state.owner_blocks.get(&owner),
                Some(OwnerMutationBlock::Ambiguous { .. })
            ));
        }
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 2);
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
                owner_recovery_poll_permit_for(owner),
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
    async fn owner_aware_recovery_permit_parse_errors_record_only_matching_response_owner() {
        let owner = address(WALLET_CREATE_OWNER);
        let other_owner = address(API_KEY_ADDRESS);

        for (label, response_owner, should_record) in [
            ("matching", Some(owner), true),
            ("mismatch", Some(other_owner), false),
            ("missing", None, false),
        ] {
            let transaction_id = format!("tx-recovery-owner-{label}");
            let mut body = json!({
                "transactionID": transaction_id,
                "state": "STATE_CONFIRMED",
                "transactionHash": "not-a-transaction-hash",
            });
            if let Some(response_owner) = response_owner {
                body["owner"] = json!(to_checksum(&response_owner, None));
            }
            let (url, handle) =
                spawn_server(vec![TestResponse::json("200 OK", body.to_string())]).await;
            let client = test_client(url);

            let error = client
                .poll_owner_transaction_with_reconciliation_permit(
                    owner,
                    &transaction_id,
                    DepositWalletPollPolicy::new(1, Duration::from_millis(100)).unwrap(),
                    owner_recovery_poll_permit_for(owner),
                )
                .await
                .unwrap_err();

            assert!(error_has_prefix(&error, RECONCILIATION_REQUIRED_PREFIX));
            {
                let state = client.mutation_state().unwrap();
                assert_eq!(
                    state.transaction_owners.contains_key(&transaction_id),
                    should_record,
                    "{label} response owner should control recovered transaction recording"
                );
            }
            if should_record {
                assert!(client.ambiguous_submit_block(owner).is_some());
            } else {
                assert!(client.ambiguous_submit_block(owner).is_none());
                client.ensure_owner_unblocked(owner).unwrap();
            }
            let requests = handle.await.unwrap();
            assert_eq!(requests.len(), 1);
            assert_eq!(
                requests[0].path,
                format!("/transaction?id={transaction_id}")
            );
        }
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
                owner_recovery_poll_permit_for(owner),
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
    async fn owner_aware_recovery_permit_pending_without_payload_keeps_ambiguous_block() {
        let owner = address(WALLET_CREATE_OWNER);
        let transaction_id = "tx-recovered-pending-without-payload";
        let payload_hash = "payload:ambiguous-before-pending-recovery".to_string();
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "200 OK",
            transaction_response(transaction_id, "STATE_NEW"),
        )])
        .await;
        let client = test_client(url);
        client.record_ambiguous(owner, payload_hash.clone()).unwrap();

        let error = client
            .poll_owner_transaction_with_reconciliation_permit(
                owner,
                transaction_id,
                DepositWalletPollPolicy::new(1, Duration::from_millis(100)).unwrap(),
                owner_recovery_poll_permit_for(owner),
            )
            .await
            .unwrap_err();

        assert!(error_has_prefix(&error, RECONCILIATION_REQUIRED_PREFIX));
        assert_eq!(client.ambiguous_submit_block(owner), Some(payload_hash));
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
    }

#[tokio::test]
    async fn owner_aware_recovery_permit_requires_payload_identity_before_confirmed_success() {
        let owner = address(WALLET_CREATE_OWNER);
        let transaction_id = "tx-recovered-without-payload";
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "200 OK",
            transaction_response(transaction_id, "STATE_CONFIRMED"),
        )])
        .await;
        let client = test_client(url);

        let error = client
            .poll_owner_transaction_with_reconciliation_permit(
                owner,
                transaction_id,
                DepositWalletPollPolicy::new(1, Duration::from_millis(100)).unwrap(),
                owner_recovery_poll_permit_for(owner),
            )
            .await
            .unwrap_err();

        assert!(error_has_prefix(&error, RECONCILIATION_REQUIRED_PREFIX));
        assert!(error.to_string().contains("ambiguous submit payload"));
        assert!(client.ambiguous_submit_block(owner).is_some());
        let payload_hash = client
            .ambiguous_submit_block(owner)
            .expect("confirmed recovery should keep an ambiguous payload block");
        let duplicate = client.get_wallet_nonce(owner).await.unwrap_err();
        assert!(error_has_prefix(&duplicate, RECONCILIATION_REQUIRED_PREFIX));
        client
            .clear_ambiguous_submit_after_manual_reconciliation(
                submit_reconciliation_evidence_for_payload_transaction_observation(
                    owner,
                    payload_hash,
                    transaction_id,
                    RelayerTransactionState::Confirmed,
                    Some("0x38cbfbeae8fffa4e2b187ee5978d3ee9cafc53af0363ed90a35b7ea9016535d8"),
                ),
                manual_reconciliation_permit_token_for(owner),
            )
            .unwrap();
        client.ensure_owner_unblocked(owner).unwrap();
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].path,
            "/transaction?id=tx-recovered-without-payload"
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
                    owner_recovery_poll_permit_for(owner),
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
    async fn terminal_poll_keeps_owner_block_when_same_payload_transactions_remain() {
        let owner = address(WALLET_CREATE_OWNER);
        let payload_hash = "payload:shared-terminal-poll";
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "200 OK",
            transaction_response("tx-local-terminal", "STATE_CONFIRMED"),
        )])
        .await;
        let client = test_client(url);
        {
            let mut state = client.mutation_state().unwrap();
            state.owner_blocks.insert(
                owner,
                OwnerMutationBlock::InFlight {
                    payload_hash: payload_hash.to_string(),
                    transaction_id: Some("tx-local-terminal".to_string()),
                    created_at_unix_seconds: 1_700_000_000,
                },
            );
            state.transaction_owners.insert(
                "tx-local-terminal".to_string(),
                OwnerTransactionRecord {
                    owner,
                    payload_hash: payload_hash.to_string(),
                    source: OwnerTransactionSource::LocalSubmit,
                },
            );
            state.transaction_owners.insert(
                "tx-recovered-same-payload".to_string(),
                OwnerTransactionRecord {
                    owner,
                    payload_hash: payload_hash.to_string(),
                    source: OwnerTransactionSource::OwnerRecovery,
                },
            );
        }

        let error = client
            .poll_owner_transaction(
                owner,
                "tx-local-terminal",
                DepositWalletPollPolicy::new(1, Duration::from_millis(100)).unwrap(),
            )
            .await
            .unwrap_err();

        assert!(error_has_prefix(&error, RECONCILIATION_REQUIRED_PREFIX));
        assert_eq!(
            client.ambiguous_submit_block(owner),
            Some(payload_hash.to_string())
        );
        let blocked = client.get_wallet_nonce(owner).await.unwrap_err();
        assert!(error_has_prefix(&blocked, RECONCILIATION_REQUIRED_PREFIX));
        {
            let state = client.mutation_state().unwrap();
            assert!(!state.transaction_owners.contains_key("tx-local-terminal"));
            assert!(state
                .transaction_owners
                .contains_key("tx-recovered-same-payload"));
        }
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].path, "/transaction?id=tx-local-terminal");
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
                    owner_recovery_poll_permit_for(owner),
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
        assert!(error_has_prefix(&poll_error, RECONCILIATION_REQUIRED_PREFIX));
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
                    created_at_unix_seconds: 1_700_000_000,
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
                owner_recovery_poll_permit_for(owner),
            )
            .await
            .unwrap_err();

        assert!(matches!(error, RelayerError::Http(_)));
        let blocked = client.get_wallet_nonce(owner).await.unwrap_err();
        assert!(error_has_prefix(&blocked, RECONCILIATION_REQUIRED_PREFIX));
        {
            let state = client.mutation_state().unwrap();
            assert!(matches!(
                state.owner_blocks.get(&owner),
                Some(OwnerMutationBlock::Ambiguous { payload_hash, .. })
                    if payload_hash == &recovered_payload_hash(transaction_id)
            ));
            assert!(state.transaction_owners.contains_key(transaction_id));
        }
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
                    ..
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
                    ..
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
                owner_recovery_poll_permit_for(owner),
            )
            .await
            .unwrap_err();

        assert!(error_has_prefix(&error, RECONCILIATION_REQUIRED_PREFIX));
        {
            let state = client.mutation_state().unwrap();
            match state.owner_blocks.get(&owner) {
                Some(OwnerMutationBlock::Ambiguous {
                    payload_hash,
                    ..
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
                owner_recovery_poll_permit_for(owner),
            )
            .await
            .unwrap_err();

        assert!(error_has_prefix(&error, RECONCILIATION_REQUIRED_PREFIX));
        {
            let state = client.mutation_state().unwrap();
            match state.owner_blocks.get(&owner) {
                Some(OwnerMutationBlock::Ambiguous {
                    payload_hash,
                    ..
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
                    created_at_unix_seconds: 1_700_000_000,
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
                owner_recovery_poll_permit_for(owner),
            )
            .await
            .unwrap_err();

        assert!(error_has_prefix(&error, RECONCILIATION_REQUIRED_PREFIX));
        assert!(client.ambiguous_submit_block(owner).is_some());
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
    }

#[tokio::test]
    async fn owner_transaction_poll_unknown_state_keeps_owner_blocked_for_reconciliation() {
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
            .poll_owner_transaction(
                owner,
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
    async fn owner_transaction_poll_requires_response_owner_before_clearing_block() {
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
            .poll_owner_transaction(
                owner,
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
                manual_reconciliation_permit_token_for(owner),
            )
            .unwrap();
        client.ensure_owner_unblocked(owner).unwrap();
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 2);
    }

#[tokio::test]
    async fn owner_transaction_poll_parse_error_marks_inflight_block_reconciliation_required() {
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
            .poll_owner_transaction(
                owner,
                "tx-malformed",
                DepositWalletPollPolicy::new(1, Duration::from_millis(100)).unwrap(),
            )
            .await
            .unwrap_err();

        assert!(matches!(error, RelayerError::Other(_)));
        let blocked = client.get_wallet_nonce(owner).await.unwrap_err();
        assert!(error_has_prefix(&blocked, RECONCILIATION_REQUIRED_PREFIX));
        {
            let state = client.mutation_state().unwrap();
            assert!(matches!(
                state.owner_blocks.get(&owner),
                Some(OwnerMutationBlock::Ambiguous { .. })
            ));
        }
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
            .poll_owner_transaction(
                owner,
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
    async fn public_poll_transaction_does_not_clear_local_owner_state() {
        let owner = address(WALLET_CREATE_OWNER);
        let transaction_id = "tx-read-only-poll";
        let (url, handle) = spawn_server(vec![
            TestResponse::json("200 OK", transaction_response(transaction_id, "STATE_NEW")),
            TestResponse::json("200 OK", transaction_response(transaction_id, "STATE_CONFIRMED")),
            TestResponse::json("200 OK", transaction_response(transaction_id, "STATE_CONFIRMED")),
            TestResponse::json("200 OK", json!({"nonce": "34"}).to_string()),
        ])
        .await;
        let client = test_client(url);

        let receipt = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap();
        assert_eq!(receipt.transaction_id, transaction_id);

        let receipt = client
            .poll_transaction(
                transaction_id,
                DepositWalletPollPolicy::new(1, Duration::from_millis(100)).unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(receipt.state, RelayerTransactionState::Confirmed);

        let blocked = client.get_wallet_nonce(owner).await.unwrap_err();
        assert!(error_has_prefix(&blocked, RECONCILIATION_REQUIRED_PREFIX));
        assert!(client.ambiguous_submit_block(owner).is_none());
        {
            let state = client.mutation_state().unwrap();
            assert!(state.transaction_owners.contains_key(transaction_id));
            match state.owner_blocks.get(&owner) {
                Some(OwnerMutationBlock::InFlight {
                    transaction_id: Some(blocked_transaction_id),
                    ..
                }) => assert_eq!(blocked_transaction_id, transaction_id),
                block => panic!("expected in-flight owner block, got {block:?}"),
            }
        }

        let receipt = client
            .poll_owner_transaction(
                owner,
                transaction_id,
                DepositWalletPollPolicy::new(1, Duration::from_millis(100)).unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(receipt.state, RelayerTransactionState::Confirmed);
        client.ensure_owner_unblocked(owner).unwrap();
        let nonce = client.get_wallet_nonce(owner).await.unwrap();
        assert_eq!(nonce, U256::from(34u64));

        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 4);
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
            .poll_owner_transaction(
                owner,
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
    async fn terminal_error_poll_keeps_owner_block_until_manual_reconciliation() {
        let owner = address(WALLET_CREATE_OWNER);

        for (transaction_id, terminal_state, observed_state, expected_error) in [
            (
                "tx-terminal-invalid",
                "STATE_INVALID",
                RelayerTransactionState::Invalid,
                "transaction invalid",
            ),
            (
                "tx-terminal-failed",
                "STATE_FAILED",
                RelayerTransactionState::Failed,
                "transaction failed",
            ),
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
                .poll_owner_transaction(
                    owner,
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

            let blocked = client.get_wallet_nonce(owner).await.unwrap_err();
            assert!(error_has_prefix(&blocked, RECONCILIATION_REQUIRED_PREFIX));
            let payload_hash = client
                .ambiguous_submit_block(owner)
                .expect("terminal failure should keep owner blocked");
            {
                let state = client.mutation_state().unwrap();
                assert!(state.transaction_owners.contains_key(transaction_id));
                assert!(state.terminal_observations.contains_key(transaction_id));
            }
            client
                .clear_ambiguous_submit_after_manual_reconciliation(
                    submit_reconciliation_evidence_for_payload_transaction_observation(
                        owner,
                        payload_hash,
                        transaction_id,
                        observed_state,
                        Some("0x38cbfbeae8fffa4e2b187ee5978d3ee9cafc53af0363ed90a35b7ea9016535d8"),
                    ),
                    manual_reconciliation_permit_token_for(owner),
                )
                .unwrap();
            client.ensure_owner_unblocked(owner).unwrap();
            let nonce = client.get_wallet_nonce(owner).await.unwrap();
            assert_eq!(nonce, U256::from(37u64));

            let requests = handle.await.unwrap();
            assert_eq!(requests.len(), 3);
        }
    }
