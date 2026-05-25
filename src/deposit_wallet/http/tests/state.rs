use super::*;

#[tokio::test]
    async fn manual_clear_rejects_active_submit_before_response() {
        let owner = address(WALLET_CREATE_OWNER);
        let client = test_client(DepositWalletRelayerUrl::loopback("http://127.0.0.1:1").unwrap());
        let payload_hash = "test-active-submit-payload".to_string();
        let _reservation = client.reserve_owner_submit(owner, payload_hash.clone()).unwrap();

        let error = client
            .clear_ambiguous_submit_after_manual_reconciliation(
                submit_reconciliation_evidence_for_payload(owner, payload_hash.clone()),
                manual_reconciliation_permit_token_for(owner),
            )
            .unwrap_err();

        assert!(error_has_prefix(&error, MUTATION_BLOCKED_PREFIX));
        {
            let state = client.mutation_state().unwrap();
            match state.owner_blocks.get(&owner) {
                Some(OwnerMutationBlock::InFlight {
                    payload_hash: current_payload_hash,
                    transaction_id: None,
                    ..
                }) => assert_eq!(current_payload_hash, &payload_hash),
                block => panic!("expected active in-flight owner block, got {block:?}"),
            }
        }
        let blocked = client.get_wallet_nonce(owner).await.unwrap_err();
        assert!(error_has_prefix(&blocked, RECONCILIATION_REQUIRED_PREFIX));
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
    fn manual_clear_requires_matching_reconciliation_payload_hash() {
        let owner = address(WALLET_CREATE_OWNER);
        let client =
            test_client(DepositWalletRelayerUrl::loopback("http://127.0.0.1:1").unwrap());
        let payload_hash = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string();
        client.record_ambiguous(owner, payload_hash.clone()).unwrap();

        let error = client
            .clear_ambiguous_submit_after_manual_reconciliation(
                submit_reconciliation_evidence_for_payload(
                    owner,
                    "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                ),
                manual_reconciliation_permit_token_for(owner),
            )
            .unwrap_err();

        assert!(error_has_prefix(&error, RECONCILIATION_REQUIRED_PREFIX));
        assert_eq!(client.ambiguous_submit_block(owner), Some(payload_hash));
    }

#[test]
    fn submit_reconciliation_evidence_rejects_invalid_inputs() {
        let owner = address(WALLET_CREATE_OWNER);
        let scope = mutation_scope(DepositWalletMutationAction::ManualReconciliation);
        let payload_hash =
            "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let transaction_hash =
            "0x38cbfbeae8fffa4e2b187ee5978d3ee9cafc53af0363ed90a35b7ea9016535d8";
        let valid_observation = || {
            DepositWalletSubmitReconciliationObservation::new(
                "tx-manual",
                RelayerTransactionState::Failed,
                None::<&str>,
                "checked",
                1_700_000_001,
            )
            .unwrap()
        };

        assert!(DepositWalletSubmitReconciliationEvidence::new(
            owner,
            mutation_scope(DepositWalletMutationAction::WalletCreate),
            "unit-test owner serialization guard",
            payload_hash,
            valid_observation(),
        )
        .is_err());
        assert!(DepositWalletSubmitReconciliationEvidence::new(
            owner,
            scope,
            " ",
            payload_hash,
            valid_observation(),
        )
        .is_err());
        assert!(DepositWalletSubmitReconciliationEvidence::new(
            owner,
            scope,
            "unit-test owner serialization guard",
            " ",
            valid_observation(),
        )
        .is_err());
        assert!(DepositWalletSubmitReconciliationObservation::new(
            "bad\nid",
            RelayerTransactionState::Failed,
            None::<&str>,
            "checked",
            1_700_000_001,
        )
        .is_err());
        assert!(DepositWalletSubmitReconciliationObservation::new(
            "tx-manual",
            RelayerTransactionState::New,
            None::<&str>,
            "checked",
            1_700_000_001,
        )
        .is_err());
        assert!(DepositWalletSubmitReconciliationObservation::new(
            "tx-manual",
            RelayerTransactionState::Confirmed,
            None::<&str>,
            "checked",
            1_700_000_001,
        )
        .is_err());
        assert!(DepositWalletSubmitReconciliationObservation::new(
            "tx-manual",
            RelayerTransactionState::Confirmed,
            Some("bad-hash"),
            "checked",
            1_700_000_001,
        )
        .is_err());
        assert!(DepositWalletSubmitReconciliationObservation::new(
            "tx-manual",
            RelayerTransactionState::Confirmed,
            Some(transaction_hash),
            " ",
            1_700_000_001,
        )
        .is_err());
        assert!(DepositWalletSubmitReconciliationObservation::new(
            "tx-manual",
            RelayerTransactionState::Confirmed,
            Some(transaction_hash),
            "checked",
            0,
        )
        .is_err());
    }

#[test]
    fn manual_clear_requires_matching_transaction_evidence_when_transaction_is_known() {
        let owner = address(WALLET_CREATE_OWNER);
        let client =
            test_client(DepositWalletRelayerUrl::loopback("http://127.0.0.1:1").unwrap());
        let payload_hash = "payload:known-transaction".to_string();
        client.record_ambiguous(owner, payload_hash.clone()).unwrap();
        client
            .record_transaction_owner("tx-known-payload", owner, payload_hash.clone())
            .unwrap();

        let error = client
            .clear_ambiguous_submit_after_manual_reconciliation(
                submit_reconciliation_evidence_for_payload_and_transaction(
                    owner,
                    payload_hash.clone(),
                    "tx-other-payload",
                ),
                manual_reconciliation_permit_token_for(owner),
            )
            .unwrap_err();

        assert!(error_has_prefix(&error, RECONCILIATION_REQUIRED_PREFIX));
        assert_eq!(client.ambiguous_submit_block(owner), Some(payload_hash));
    }

#[test]
    fn manual_clear_rejects_stale_future_or_wrong_issuer_reconciliation_evidence() {
        let owner = address(WALLET_CREATE_OWNER);
        let payload_hash = "payload:timed-reconciliation".to_string();
        for (issuer, checked_at) in [
            ("unit-test owner serialization guard", 1_699_999_999),
            ("unit-test owner serialization guard", 1_700_000_100),
            ("other issuer", 1_700_000_001),
        ] {
            let client =
                test_client(DepositWalletRelayerUrl::loopback("http://127.0.0.1:1").unwrap());
            client.record_ambiguous(owner, payload_hash.clone()).unwrap();
            let evidence = DepositWalletSubmitReconciliationEvidence::new(
                owner,
                mutation_scope(DepositWalletMutationAction::ManualReconciliation),
                issuer,
                payload_hash.clone(),
                DepositWalletSubmitReconciliationObservation::new(
                    "tx-timed-reconciliation",
                    RelayerTransactionState::Failed,
                    None::<&str>,
                    "checked mocked relayer state",
                    checked_at,
                )
                .unwrap(),
            )
            .unwrap();

            let error = client
                .clear_ambiguous_submit_after_manual_reconciliation(
                    evidence,
                    manual_reconciliation_permit_token_for(owner),
                )
                .unwrap_err();

            assert!(error_has_prefix(&error, MUTATION_BLOCKED_PREFIX));
            assert_eq!(client.ambiguous_submit_block(owner), Some(payload_hash.clone()));
        }
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
                        created_at_unix_seconds: 1_700_000_000,
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
    fn recovered_transaction_records_respect_capacity_when_owner_is_already_blocked() {
        let owner = address(WALLET_CREATE_OWNER);
        let client =
            test_client(DepositWalletRelayerUrl::loopback("http://127.0.0.1:1").unwrap());
        {
            let mut state = client.mutation_state().unwrap();
            state.owner_blocks.insert(
                owner,
                OwnerMutationBlock::Ambiguous {
                    payload_hash: "payload:ambiguous-owner".to_string(),
                    created_at_unix_seconds: 1_700_000_000,
                },
            );
            for index in 0..MAX_OWNER_MUTATION_RECORDS {
                state.transaction_owners.insert(
                    format!("tx-existing-{index}"),
                    OwnerTransactionRecord {
                        owner,
                        payload_hash: "payload:ambiguous-owner".to_string(),
                        source: OwnerTransactionSource::OwnerRecovery,
                    },
                );
            }
        }

        let error = client
            .record_recovered_inflight_transaction(owner, "tx-over-capacity")
            .unwrap_err();

        assert!(error_has_prefix(&error, MUTATION_BLOCKED_PREFIX));
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
                    created_at_unix_seconds: 1_700_000_000,
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
                submit_reconciliation_evidence_for(&client, owner),
                manual_reconciliation_permit_token_for(owner),
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
                submit_reconciliation_evidence_for_payload(
                    owner,
                    "0x1111111111111111111111111111111111111111111111111111111111111111",
                ),
                manual_reconciliation_permit_token_for(owner),
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
