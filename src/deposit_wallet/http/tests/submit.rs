use super::*;

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
                        mutation_scope(DepositWalletMutationAction::WalletCreate),
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
    async fn production_submit_requires_trusted_permit_before_auth_or_http() {
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
            .submit_wallet_create(
                address(WALLET_CREATE_OWNER),
                mutation_permit_for_scope(
                    address(WALLET_CREATE_OWNER),
                    mutation_scope(DepositWalletMutationAction::WalletCreate),
                ),
            )
            .await
            .unwrap_err();

        assert!(error_has_prefix(&error, MUTATION_BLOCKED_PREFIX));

        let production_evidence = DepositWalletOwnerSerializationEvidence::new(
            address(WALLET_CREATE_OWNER),
            client.mutation_scope(DepositWalletMutationAction::WalletCreate),
            "unit-test owner serialization guard",
            "production-owner-lease",
            1_699_999_900,
            1_700_000_200,
        )
        .unwrap();
        let error = DepositWalletMutationPermit::from_owner_serialization_evidence(
            "production submit",
            production_evidence,
        )
        .unwrap_err();
        assert!(error_has_prefix(&error, MUTATION_BLOCKED_PREFIX));

        let signed = signed_wallet_batch();
        let owner = signed.owner();
        let production_batch_evidence = DepositWalletOwnerSerializationEvidence::new(
            owner,
            client.mutation_scope(DepositWalletMutationAction::WalletBatch),
            "unit-test owner serialization guard",
            "production-batch-owner-lease",
            1_699_999_900,
            1_700_000_200,
        )
        .unwrap();
        let error = DepositWalletMutationPermit::from_owner_serialization_evidence(
            "production WALLET submit",
            production_batch_evidence,
        )
        .unwrap_err();
        assert!(error_has_prefix(&error, MUTATION_BLOCKED_PREFIX));

        let error = client
            .submit_signed_wallet_batch(
                signed,
                mutation_permit_for_scope(
                    owner,
                    mutation_scope(DepositWalletMutationAction::WalletBatch),
                ),
            )
            .await
            .unwrap_err();

        assert!(error_has_prefix(&error, MUTATION_BLOCKED_PREFIX));
    }

#[tokio::test]
    async fn mutation_scope_rejects_unsupported_contract_config_without_zero_chain_fallback() {
        let owner = address(WALLET_CREATE_OWNER);
        let url = DepositWalletRelayerUrl::loopback("http://127.0.0.1:1").unwrap();
        let invalid_config = DepositWalletContractConfig {
            factory: Address::zero(),
            implementation: Address::zero(),
        };
        let clock: Arc<dyn DepositWalletClock> = Arc::new(FixedClock { now: 1_700_000_000 });
        let sleeper: Arc<dyn DepositWalletSleeper> = Arc::new(RecordingSleeper::default());
        let client = DepositWalletRelayerClient::from_parts(
            reqwest_client(Duration::from_secs(2)),
            url,
            relayer_auth(),
            invalid_config,
            clock,
            sleeper,
        );

        let error = client
            .try_mutation_scope(DepositWalletMutationAction::WalletCreate)
            .unwrap_err();
        assert!(matches!(error, RelayerError::Signing(_)));

        let error = client
            .submit_wallet_create(owner, mutation_permit())
            .await
            .unwrap_err();

        assert!(matches!(error, RelayerError::Signing(_)));
        assert!(client.ambiguous_submit_block(owner).is_none());
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
        let rendered = error.to_string();
        assert!(rendered.contains("submit authentication failed before POST"));
        assert!(!rendered.contains("invalid"));
        assert!(!rendered.contains(API_KEY));
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
                mutation_scope(DepositWalletMutationAction::WalletCreate),
                "unit-test caller lock",
                "owner-lock-key-456",
                1_699_999_900,
                1_700_000_200,
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

#[test]
    fn owner_serialization_evidence_rejects_invalid_inputs() {
        let owner = address(WALLET_CREATE_OWNER);
        let scope = mutation_scope(DepositWalletMutationAction::WalletCreate);

        assert!(DepositWalletOwnerSerializationEvidence::new(
            owner,
            scope,
            " ",
            "lease",
            1_699_999_900,
            1_700_000_200,
        )
        .is_err());
        assert!(DepositWalletOwnerSerializationEvidence::new(
            owner,
            scope,
            "unit-test guard",
            "",
            1_699_999_900,
            1_700_000_200,
        )
        .is_err());
        assert!(DepositWalletOwnerSerializationEvidence::new(
            owner,
            scope,
            "unit-test guard",
            "lease",
            0,
            1,
        )
        .is_err());
        assert!(DepositWalletOwnerSerializationEvidence::new(
            owner,
            scope,
            "unit-test guard",
            "lease",
            1_700_000_000,
            1_700_000_000,
        )
        .is_err());
        assert!(DepositWalletOwnerSerializationEvidence::new(
            owner,
            scope,
            "unit-test guard",
            "lease",
            1_699_999_000,
            1_700_000_000,
        )
        .is_err());
    }

#[test]
    fn mutation_permit_freshness_validates_clock_skew_lease_and_expiry_boundaries() {
        let owner = address(WALLET_CREATE_OWNER);
        let scope = mutation_scope(DepositWalletMutationAction::WalletCreate);
        let now = 1_700_000_000;
        let permit_for = |acquired_at_unix_seconds, expires_at_unix_seconds| {
            DepositWalletMutationPermit::from_owner_serialization_evidence(
                "freshness boundary test",
                DepositWalletOwnerSerializationEvidence::new(
                    owner,
                    scope,
                    "unit-test owner serialization guard",
                    "freshness-lease",
                    acquired_at_unix_seconds,
                    expires_at_unix_seconds,
                )
                .unwrap(),
            )
            .unwrap()
        };

        let skew_boundary =
            permit_for(now + MAX_EVIDENCE_CLOCK_SKEW_SECONDS, now + MAX_EVIDENCE_CLOCK_SKEW_SECONDS + 1);
        assert!(super::super::permit::validate_permit_fresh(&skew_boundary, now).is_ok());
        let future_beyond_skew = permit_for(
            now + MAX_EVIDENCE_CLOCK_SKEW_SECONDS + 1,
            now + MAX_EVIDENCE_CLOCK_SKEW_SECONDS + 2,
        );
        assert!(super::super::permit::validate_permit_fresh(&future_beyond_skew, now).is_err());

        let acquired_at = now - MAX_OWNER_SERIALIZATION_LEASE_SECONDS + 1;
        let last_fresh = permit_for(acquired_at, acquired_at + MAX_OWNER_SERIALIZATION_LEASE_SECONDS);
        assert!(super::super::permit::validate_permit_fresh(&last_fresh, now).is_ok());
        let expired_at_now = permit_for(
            now - MAX_OWNER_SERIALIZATION_LEASE_SECONDS,
            now,
        );
        assert!(super::super::permit::validate_permit_fresh(&expired_at_now, now).is_err());
        let stale = permit_for(
            now - MAX_OWNER_SERIALIZATION_LEASE_SECONDS - 1,
            now - 1,
        );
        assert!(super::super::permit::validate_permit_fresh(&stale, now).is_err());
    }

#[tokio::test]
    async fn mutation_permit_rejects_wrong_scope_or_future_lease_before_http() {
        let url = DepositWalletRelayerUrl::loopback("http://127.0.0.1:1").unwrap();
        let bad_auth = RelayerKeyAuth::new("invalid\nheader", address(API_KEY_ADDRESS));
        let client =
            test_client_with_auth_clock_timeout(url, bad_auth, 1_700_000_000, Duration::from_secs(1));
        let owner = address(WALLET_CREATE_OWNER);

        let wrong_scope = DepositWalletOwnerSerializationEvidence::new(
            owner,
            mutation_scope(DepositWalletMutationAction::WalletBatch),
            "unit-test owner serialization guard",
            "wrong-scope-lease",
            1_699_999_900,
            1_700_000_200,
        )
        .unwrap();
        let error = client
            .submit_wallet_create(
                owner,
                DepositWalletMutationGate::Permit(
                    DepositWalletMutationPermit::from_owner_serialization_evidence(
                        "wrong action scope",
                        wrong_scope,
                    )
                    .unwrap(),
                ),
            )
            .await
            .unwrap_err();
        assert!(error_has_prefix(&error, MUTATION_BLOCKED_PREFIX));

        let future_lease = DepositWalletOwnerSerializationEvidence::new(
            owner,
            mutation_scope(DepositWalletMutationAction::WalletCreate),
            "unit-test owner serialization guard",
            "future-lease",
            1_700_000_100,
            1_700_000_200,
        )
        .unwrap();
        let error = client
            .submit_wallet_create(
                owner,
                DepositWalletMutationGate::Permit(
                    DepositWalletMutationPermit::from_owner_serialization_evidence(
                        "future lease",
                        future_lease,
                    )
                    .unwrap(),
                ),
            )
            .await
            .unwrap_err();
        assert!(error_has_prefix(&error, MUTATION_BLOCKED_PREFIX));
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
            .submit_signed_wallet_batch(signed, wallet_batch_mutation_permit_for(owner))
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
                .submit_signed_wallet_batch(signed, wallet_batch_mutation_permit_for(owner))
                .await
                .unwrap_err();

            assert!(error_has_prefix(&error, AMBIGUOUS_SUBMIT_PREFIX));
            if let Some(status) = expected {
                assert!(error.to_string().contains(&status.to_string()));
            } else {
                assert!(error.to_string().contains("429"));
            }
            assert!(client.ambiguous_submit_block(owner).is_some());
            let blocked = client.get_wallet_nonce(owner).await.unwrap_err();
            assert!(error_has_prefix(&blocked, RECONCILIATION_REQUIRED_PREFIX));
            let blocked = client
                .submit_signed_wallet_batch(signed_wallet_batch(), wallet_batch_mutation_permit_for(owner))
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
            .submit_signed_wallet_batch(
                signed,
                mutation_permit_for_scope(
                    owner,
                    client.mutation_scope(DepositWalletMutationAction::WalletBatch),
                ),
            )
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
            .submit_signed_wallet_batch(
                signed,
                mutation_permit_for_scope(
                    owner,
                    client.mutation_scope(DepositWalletMutationAction::WalletBatch),
                ),
            )
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
    async fn signed_submit_salvages_valid_transaction_id_from_unusable_success_response() {
        let signed = signed_wallet_batch();
        let owner = signed.owner();
        let (url, handle) = spawn_server(vec![
            TestResponse::json("200 OK", json!({"nonce": signed.nonce().to_string()}).to_string()),
            TestResponse::json(
                "200 OK",
                json!({
                    "transactionID": "tx-salvaged-submit",
                    "state": "STATE_CONFIRMED",
                    "transactionHash": "not-a-transaction-hash"
                })
                .to_string(),
            ),
            TestResponse::json(
                "200 OK",
                json!({
                    "transactionID": "tx-salvaged-submit",
                    "state": "STATE_CONFIRMED",
                    "transactionHash": "0x38cbfbeae8fffa4e2b187ee5978d3ee9cafc53af0363ed90a35b7ea9016535d8",
                    "owner": to_checksum(&owner, None)
                })
                .to_string(),
            ),
        ])
        .await;
        let client = test_client(url);

        let error = client
            .submit_signed_wallet_batch(signed, wallet_batch_mutation_permit_for(owner))
            .await
            .unwrap_err();

        assert!(error_has_prefix(&error, AMBIGUOUS_SUBMIT_PREFIX));
        assert!(error.to_string().contains("transaction id hash"));
        assert!(!error.to_string().contains("tx-salvaged-submit"));
        assert!(client.ambiguous_submit_block(owner).is_some());
        let blocked = client.get_wallet_nonce(owner).await.unwrap_err();
        assert!(error_has_prefix(&blocked, RECONCILIATION_REQUIRED_PREFIX));

        let receipt = client
            .poll_owner_transaction(
                owner,
                "tx-salvaged-submit",
                DepositWalletPollPolicy::new(1, Duration::from_millis(100)).unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(receipt.state, RelayerTransactionState::Confirmed);
        client.ensure_owner_unblocked(owner).unwrap();
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[1].path, SUBMIT_PATH);
        assert_eq!(requests[2].path, "/transaction?id=tx-salvaged-submit");
    }

#[tokio::test]
    async fn idless_unusable_submit_success_responses_block_until_reconciliation() {
        for response in [
            TestResponse::json("200 OK", ""),
            TestResponse::json("200 OK", "{"),
            TestResponse::json(
                "200 OK",
                json!({"transactionID": "bad transaction id", "state": "STATE_CONFIRMED"})
                    .to_string(),
            ),
            TestResponse::json_without_content_length(
                "200 OK",
                "x".repeat(MAX_SUCCESS_BODY_BYTES + 1),
            ),
        ] {
            let owner = address(WALLET_CREATE_OWNER);
            let (url, handle) = spawn_server(vec![response]).await;
            let client = test_client(url);

            let error = client
                .submit_wallet_create(owner, mutation_permit())
                .await
                .unwrap_err();

            assert!(error_has_prefix(&error, AMBIGUOUS_SUBMIT_PREFIX));
            let payload_hash = client
                .ambiguous_submit_block(owner)
                .expect("id-less parse failure should keep an ambiguous payload block");
            let nonce_error = client.get_wallet_nonce(owner).await.unwrap_err();
            assert!(error_has_prefix(&nonce_error, RECONCILIATION_REQUIRED_PREFIX));
            let duplicate = client
                .submit_wallet_create(owner, mutation_permit())
                .await
                .unwrap_err();
            assert!(error_has_prefix(&duplicate, RECONCILIATION_REQUIRED_PREFIX));

            client
                .clear_idless_ambiguous_submit_after_manual_reconciliation(
                    idless_submit_reconciliation_evidence_for_payload(owner, payload_hash),
                    manual_reconciliation_permit_token_for(owner),
                )
                .unwrap();
            client.ensure_owner_unblocked(owner).unwrap();
            let requests = handle.await.unwrap();
            assert_eq!(requests.len(), 1);
            assert_eq!(requests[0].path, SUBMIT_PATH);
        }
    }

#[tokio::test]
    async fn submit_signed_wallet_batch_rejects_stale_nonce_before_post() {
        let signed = signed_wallet_batch();
        let (url, handle) = spawn_server(vec![
            TestResponse::json(
                "200 OK",
                json!({"nonce": (signed.nonce() + U256::one()).to_string()}).to_string(),
            ),
            TestResponse::json(
                "200 OK",
                json!({"nonce": signed.nonce().to_string()}).to_string(),
            ),
            TestResponse::json(
                "200 OK",
                transaction_response("tx-stale-nonce-retry", "STATE_NEW"),
            ),
        ])
        .await;
        let client = test_client(url);
        let owner = signed.owner();

        let error = client
            .submit_signed_wallet_batch(
                signed.clone(),
                mutation_permit_for_scope(
                    owner,
                    client.mutation_scope(DepositWalletMutationAction::WalletBatch),
                ),
            )
            .await
            .unwrap_err();

        assert!(matches!(error, RelayerError::Signing(message) if message.contains("nonce")));
        assert!(client.ambiguous_submit_block(owner).is_none());
        client.ensure_owner_unblocked(owner).unwrap();

        let receipt = client
            .submit_signed_wallet_batch(
                signed,
                mutation_permit_for_scope(
                    owner,
                    client.mutation_scope(DepositWalletMutationAction::WalletBatch),
                ),
            )
            .await
            .unwrap();
        assert_eq!(receipt.transaction_id, "tx-stale-nonce-retry");

        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[0].method, "GET");
        assert!(requests[0].path.contains("/nonce?address="));
        assert_eq!(requests[1].method, "GET");
        assert!(requests[1].path.contains("/nonce?address="));
        assert_eq!(requests[2].method, "POST");
        assert_eq!(requests[2].path, SUBMIT_PATH);
    }

#[tokio::test]
    async fn signed_wallet_batch_nonce_mismatch_does_not_post_and_clears_reservation() {
        let signed = signed_wallet_batch();
        let owner = signed.owner();
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "200 OK",
            json!({"nonce": (signed.nonce() + U256::one()).to_string()}).to_string(),
        )])
        .await;
        let client = test_client(url);

        let error = client
            .submit_signed_wallet_batch(signed, wallet_batch_mutation_permit_for(owner))
            .await
            .unwrap_err();

        assert!(matches!(error, RelayerError::Signing(message) if message.contains("nonce")));
        assert!(client.ambiguous_submit_block(owner).is_none());
        client.ensure_owner_unblocked(owner).unwrap();
        let mut retry_reservation = client
            .reserve_owner_submit(owner, "payload:retry-after-nonce-mismatch".to_string())
            .unwrap();
        retry_reservation.clear().unwrap();
        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, "GET");
        assert!(requests[0].path.contains("/nonce?address="));
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
            .submit_signed_wallet_batch(
                signed,
                mutation_permit_for_scope(
                    owner,
                    client.mutation_scope(DepositWalletMutationAction::WalletBatch),
                ),
            )
            .await
            .unwrap_err();

        assert!(matches!(error, RelayerError::Signing(_)), "{error:?}");
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
        let before_deadline = deadline - 100;
        let clock: Arc<dyn DepositWalletClock> = Arc::new(SequenceClock::new([
            before_deadline,
            before_deadline,
            before_deadline,
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
            .submit_signed_wallet_batch(
                signed,
                DepositWalletMutationGate::Permit(mutation_permit_token_for_scope_times(
                    owner,
                    client.mutation_scope(DepositWalletMutationAction::WalletBatch),
                    deadline - 200,
                    deadline + 100,
                )),
            )
            .await
            .unwrap_err();

        assert!(
            matches!(error, RelayerError::Signing(ref message) if message.contains("expired")),
            "{error:?}"
        );
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
    async fn signed_wallet_batch_rechecks_permit_after_nonce_lookup_before_post() {
        let signed = signed_wallet_batch();
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "200 OK",
            json!({"nonce": signed.nonce().to_string()}).to_string(),
        )])
        .await;
        let clock: Arc<dyn DepositWalletClock> = Arc::new(SequenceClock::new([
            1_700_000_000,
            1_700_000_000,
            1_700_000_001,
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
        let evidence = DepositWalletOwnerSerializationEvidence::new(
            owner,
            mutation_scope(DepositWalletMutationAction::WalletBatch),
            "unit-test short owner serialization guard",
            "unit-test-short-owner-lease",
            1_699_999_900,
            1_700_000_001,
        )
        .unwrap();
        let gate = DepositWalletMutationGate::Permit(
            DepositWalletMutationPermit::from_owner_serialization_evidence(
                "unit-test permit expires after nonce lookup",
                evidence,
            )
            .unwrap(),
        );

        let error = client
            .submit_signed_wallet_batch(signed, gate)
            .await
            .unwrap_err();

        assert!(error_has_prefix(&error, MUTATION_BLOCKED_PREFIX));
        assert!(client.ambiguous_submit_block(owner).is_none());
        client.ensure_owner_unblocked(owner).unwrap();
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
            .submit_signed_wallet_batch(
                signed,
                DepositWalletMutationGate::Permit(mutation_permit_token_for_scope_times(
                    owner,
                    client.mutation_scope(DepositWalletMutationAction::WalletBatch),
                    1_999_999_900,
                    2_000_000_100,
                )),
            )
            .await
            .unwrap_err();

        assert!(matches!(error, RelayerError::Signing(message) if message.contains("expired")));
    }

#[tokio::test]
    async fn partial_submit_response_records_owner_scoped_ambiguous_block_until_cleared() {
        let owner = address(WALLET_CREATE_OWNER);
        let (url, handle) = spawn_server(vec![TestResponse::json(
            "200 OK",
            json!({"transactionID": "", "state": "STATE_NEW"}).to_string(),
        )])
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
                submit_reconciliation_evidence_for(&client, owner),
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
                submit_reconciliation_evidence_for(&client, owner),
                unchecked_mutation_permit(
                    owner,
                    "checked mocked relayer state",
                    DepositWalletOwnerSerializationEvidence::new(
                        owner,
                        mutation_scope(DepositWalletMutationAction::ManualReconciliation),
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
                submit_reconciliation_evidence_for(&client, owner),
                unchecked_mutation_permit(
                    Address::from_low_u64_be(99),
                    "checked mocked relayer state",
                    owner_serialization_evidence_for(Address::from_low_u64_be(99)),
                ),
            )
            .unwrap_err();
        assert!(error_has_prefix(&error, MUTATION_BLOCKED_PREFIX));
        assert!(client.ambiguous_submit_block(owner).is_some());

        let payload_hash = client
            .ambiguous_submit_block(owner)
            .expect("test owner should have an ambiguous submit block");
        client
            .clear_idless_ambiguous_submit_after_manual_reconciliation(
                idless_submit_reconciliation_evidence_for_payload(owner, payload_hash),
                manual_reconciliation_permit_token_for(owner),
            )
            .unwrap();
        client.ensure_owner_unblocked(owner).unwrap();
        assert!(client.ambiguous_submit_block(owner).is_none());
        assert!(client.ambiguous_submit_transaction_ids(owner).is_empty());

        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].path, SUBMIT_PATH);
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
        let blocked_nonce = client.get_wallet_nonce(owner).await.unwrap_err();
        assert!(error_has_prefix(&blocked_nonce, RECONCILIATION_REQUIRED_PREFIX));

        let requests = handle.await.unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].path, SUBMIT_PATH);
    }

#[tokio::test]
    async fn submit_post_client_timeout_records_ambiguous_block_and_blocks_duplicate() {
        let owner = address(WALLET_CREATE_OWNER);
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
            tokio::time::sleep(Duration::from_millis(300)).await;
            vec![request]
        });
        let url = DepositWalletRelayerUrl::loopback(&format!("http://{addr}")).unwrap();
        let client =
            test_client_with_auth_clock_timeout(url, relayer_auth(), 1_700_000_000, Duration::from_millis(100));

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
            .submit_signed_wallet_batch(signed, wallet_batch_mutation_permit_for(owner))
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
        let blocked = client.get_wallet_nonce(owner).await.unwrap_err();
        assert!(error_has_prefix(&blocked, RECONCILIATION_REQUIRED_PREFIX));
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
        let blocked = client.get_wallet_nonce(owner).await.unwrap_err();
        assert!(error_has_prefix(&blocked, RECONCILIATION_REQUIRED_PREFIX));
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
        assert!(rendered.contains("tx-weird"));
        assert!(client.ambiguous_submit_block(owner).is_some());
        assert_eq!(
            client.ambiguous_submit_transaction_ids(owner),
            vec!["tx-weird".to_string()]
        );
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
    async fn immediate_terminal_failure_submit_requires_owner_poll_reconciliation() {
        let owner = address(WALLET_CREATE_OWNER);

        for (transaction_id, state, expected_state, expected_error) in [
            (
                "tx-invalid-now",
                "STATE_INVALID",
                RelayerTransactionState::Invalid,
                "invalid",
            ),
            (
                "tx-failed-now",
                "STATE_FAILED",
                RelayerTransactionState::Failed,
                "failed",
            ),
        ] {
            let (url, handle) = spawn_server(vec![
                TestResponse::json("200 OK", transaction_response(transaction_id, state)),
                TestResponse::json("200 OK", transaction_response(transaction_id, state)),
                TestResponse::json("200 OK", json!({"nonce": "34"}).to_string()),
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

            let result = client
                .poll_owner_transaction(
                    owner,
                    transaction_id,
                    DepositWalletPollPolicy::new(1, Duration::from_millis(100)).unwrap(),
                )
                .await;
            match expected_error {
                "invalid" => {
                    assert!(matches!(result.unwrap_err(), RelayerError::TransactionInvalid(_)));
                }
                "failed" => {
                    assert!(matches!(result.unwrap_err(), RelayerError::TransactionFailed(_)));
                }
                _ => unreachable!(),
            }
            let blocked = client.get_wallet_nonce(owner).await.unwrap_err();
            assert!(error_has_prefix(&blocked, RECONCILIATION_REQUIRED_PREFIX));
            let payload_hash = client
                .ambiguous_submit_block(owner)
                .expect("terminal failure should keep owner blocked until manual reconciliation");
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
                        expected_state,
                        Some("0x38cbfbeae8fffa4e2b187ee5978d3ee9cafc53af0363ed90a35b7ea9016535d8"),
                    ),
                    manual_reconciliation_permit_token_for(owner),
                )
                .unwrap();
            client.ensure_owner_unblocked(owner).unwrap();

            let nonce = client.get_wallet_nonce(owner).await.unwrap();
            assert_eq!(nonce, U256::from(34u64));
            let mut retry_reservation = client
                .reserve_owner_submit(owner, format!("payload:retry-after-{transaction_id}"))
                .unwrap();
            retry_reservation.clear().unwrap();
            let requests = handle.await.unwrap();
            assert_eq!(requests.len(), 3);
            assert_eq!(requests[0].path, SUBMIT_PATH);
            assert_eq!(requests[1].path, format!("/transaction?id={transaction_id}"));
            assert!(requests[2].path.contains("/nonce?address="));
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
            .poll_owner_transaction(
                owner,
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
