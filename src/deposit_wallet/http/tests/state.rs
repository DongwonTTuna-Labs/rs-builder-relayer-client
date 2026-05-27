use super::*;

#[test]
    fn display_payload_hash_redacts_noncanonical_payload_values() {
        let rendered = super::super::redaction::display_payload_hash("secret-token\npayload");

        assert!(rendered.starts_with("sha3:0x"));
        assert!(!rendered.contains("secret-token"));
        assert!(!rendered.contains("payload"));
    }

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
        let blocked = client.get_wallet_nonce(owner, wallet_nonce_read_permit_for(owner)).await.unwrap_err();
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
    fn poisoned_owner_mutation_state_blocks_follow_up_mutations_after_drop() {
        let signed = signed_wallet_batch();
        let owner = signed.owner();
        let client =
            test_client(DepositWalletRelayerUrl::loopback("http://127.0.0.1:1").unwrap());
        let payload_hash = signed_digest_payload_hash(signed.digest());
        let reservation = client
            .reserve_owner_submit(owner, payload_hash.clone())
            .unwrap();
        let state = Arc::clone(&client.mutation_state);

        let poison = std::panic::catch_unwind(move || {
            let _guard = state.lock().unwrap();
            panic!("poison owner mutation state for regression coverage");
        });
        assert!(poison.is_err());
        drop(reservation);

        let blocked = match client.reserve_owner_submit(owner, "payload:after-poison".to_string()) {
            Ok(_) => panic!("poisoned mutation state should block follow-up submit reservation"),
            Err(error) => error,
        };
        assert!(error_has_prefix(&blocked, RECONCILIATION_REQUIRED_PREFIX));
        let unblocked = client.ensure_owner_unblocked(owner).unwrap_err();
        assert!(error_has_prefix(&unblocked, RECONCILIATION_REQUIRED_PREFIX));
    }

#[test]
    fn nonce_read_promotion_replaces_nonce_read_with_submit_block() {
        let owner = address(WALLET_CREATE_OWNER);
        let client =
            test_client(DepositWalletRelayerUrl::loopback("http://127.0.0.1:1").unwrap());
        let nonce_read = client.reserve_owner_nonce_read(owner).unwrap();
        let payload_hash = "payload:promoted-nonce-read".to_string();

        let reservation = client
            .promote_owner_nonce_read_to_submit(nonce_read, payload_hash.clone())
            .unwrap();

        {
            let state = client.mutation_state().unwrap();
            assert!(!state.nonce_reads.contains_key(&owner));
            assert!(matches!(
                state.owner_blocks.get(&owner),
                Some(OwnerMutationBlock::InFlight {
                    payload_hash: current_payload_hash,
                    transaction_id: None,
                    ..
                }) if current_payload_hash == &payload_hash
            ));
        }
        drop(reservation);
        client.ensure_owner_unblocked(owner).unwrap();
    }

#[test]
    fn stale_nonce_read_promotion_preserves_current_nonce_read() {
        let owner = address(WALLET_CREATE_OWNER);
        let client =
            test_client(DepositWalletRelayerUrl::loopback("http://127.0.0.1:1").unwrap());
        let stale_nonce_read = client.reserve_owner_nonce_read(owner).unwrap();
        let replacement_created_at = stale_nonce_read.created_at_unix_seconds() + 1;
        {
            let mut state = client.mutation_state().unwrap();
            state.nonce_reads.insert(owner, replacement_created_at);
        }

        let error = match client.promote_owner_nonce_read_to_submit(
            stale_nonce_read,
            "payload:stale-nonce-read".to_string(),
        ) {
            Ok(_) => panic!("stale nonce read should not promote to submit"),
            Err(error) => error,
        };

        assert!(error_has_prefix(&error, MUTATION_BLOCKED_PREFIX));
        {
            let state = client.mutation_state().unwrap();
            assert_eq!(
                state.nonce_reads.get(&owner).copied(),
                Some(replacement_created_at)
            );
            assert!(!state.owner_blocks.contains_key(&owner));
        }
    }

#[test]
    fn repeated_ambiguous_recording_preserves_original_block_timestamp() {
        let owner = address(WALLET_CREATE_OWNER);
        let client =
            test_client(DepositWalletRelayerUrl::loopback("http://127.0.0.1:1").unwrap());
        {
            let mut state = client.mutation_state().unwrap();
            state.owner_blocks.insert(
                owner,
                OwnerMutationBlock::Ambiguous {
                    payload_hash: "payload:old-boundary".to_string(),
                    created_at_unix_seconds: 123,
                },
            );
        }

        client
            .record_ambiguous(owner, "payload:old-boundary".to_string())
            .unwrap();

        let state = client.mutation_state().unwrap();
        assert!(matches!(
            state.owner_blocks.get(&owner),
            Some(OwnerMutationBlock::Ambiguous {
                payload_hash,
                created_at_unix_seconds: 123,
            }) if payload_hash == "payload:old-boundary"
        ));
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
    fn manual_clear_rejects_transaction_evidence_without_local_transaction_id() {
        let owner = address(WALLET_CREATE_OWNER);
        let client =
            test_client(DepositWalletRelayerUrl::loopback("http://127.0.0.1:1").unwrap());
        let payload_hash = "payload:idless-submit".to_string();
        client.record_ambiguous(owner, payload_hash.clone()).unwrap();

        let error = client
            .clear_ambiguous_submit_after_manual_reconciliation(
                submit_reconciliation_evidence_for_payload_and_transaction(
                    owner,
                    payload_hash.clone(),
                    "tx-manual-idless-submit",
                ),
                manual_reconciliation_permit_token_for(owner),
            )
            .unwrap_err();

        assert!(error_has_prefix(&error, RECONCILIATION_REQUIRED_PREFIX));
        assert_eq!(client.ambiguous_submit_block(owner), Some(payload_hash));
    }

#[test]
    fn idless_manual_clear_is_idempotent_after_owner_already_unblocked() {
        let owner = address(WALLET_CREATE_OWNER);
        let client =
            test_client(DepositWalletRelayerUrl::loopback("http://127.0.0.1:1").unwrap());
        let payload_hash = "payload:idless-already-cleared".to_string();

        client
            .clear_idless_ambiguous_submit_after_manual_reconciliation(
                idless_submit_reconciliation_evidence_for_payload(owner, payload_hash),
                manual_reconciliation_permit_token_for(owner),
            )
            .unwrap();

        client.ensure_owner_unblocked(owner).unwrap();
    }

#[test]
    fn idless_manual_clear_removes_matching_ambiguous_block_after_absence_evidence() {
        let owner = address(WALLET_CREATE_OWNER);
        let client =
            test_client(DepositWalletRelayerUrl::loopback("http://127.0.0.1:1").unwrap());
        let payload_hash = "payload:idless-not-accepted".to_string();
        client.record_ambiguous(owner, payload_hash.clone()).unwrap();

        client
            .clear_idless_ambiguous_submit_after_manual_reconciliation(
                idless_submit_reconciliation_evidence_for_payload(owner, payload_hash),
                manual_reconciliation_permit_token_for(owner),
            )
            .unwrap();

        client.ensure_owner_unblocked(owner).unwrap();
    }

#[test]
    fn idless_manual_clear_rejects_known_payload_transactions() {
        let owner = address(WALLET_CREATE_OWNER);
        let client =
            test_client(DepositWalletRelayerUrl::loopback("http://127.0.0.1:1").unwrap());
        let payload_hash = "payload:idless-known-transaction".to_string();
        client.record_ambiguous(owner, payload_hash.clone()).unwrap();
        client
            .record_transaction_owner("tx-known", owner, payload_hash.clone())
            .unwrap();

        let error = client
            .clear_idless_ambiguous_submit_after_manual_reconciliation(
                idless_submit_reconciliation_evidence_for_payload(owner, payload_hash.clone()),
                manual_reconciliation_permit_token_for(owner),
            )
            .unwrap_err();

        assert!(error_has_prefix(&error, RECONCILIATION_REQUIRED_PREFIX));
        assert_eq!(client.ambiguous_submit_block(owner), Some(payload_hash));
    }

#[test]
    fn idless_manual_clear_rejects_payload_mismatch() {
        let owner = address(WALLET_CREATE_OWNER);
        let client =
            test_client(DepositWalletRelayerUrl::loopback("http://127.0.0.1:1").unwrap());
        let payload_hash = "payload:idless-current".to_string();
        let evidence_payload_hash = "payload:idless-other".to_string();
        client.record_ambiguous(owner, payload_hash.clone()).unwrap();

        let error = client
            .clear_idless_ambiguous_submit_after_manual_reconciliation(
                idless_submit_reconciliation_evidence_for_payload(owner, evidence_payload_hash),
                manual_reconciliation_permit_token_for(owner),
            )
            .unwrap_err();

        assert!(error_has_prefix(&error, RECONCILIATION_REQUIRED_PREFIX));
        assert!(error.to_string().contains("did not match current ambiguous payload"));
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
        for state in [
            RelayerTransactionState::Executed,
            RelayerTransactionState::Mined,
            RelayerTransactionState::Unknown("STATE_STRANGE".to_string()),
        ] {
            assert!(DepositWalletSubmitReconciliationObservation::new(
                "tx-manual",
                state,
                None::<&str>,
                "checked",
                1_700_000_001,
            )
            .is_err());
        }
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
        assert!(DepositWalletIdlessSubmitReconciliationEvidence::new(
            owner,
            scope,
            "unit-test owner serialization guard",
            payload_hash,
            "",
            1_700_000_001,
        )
        .is_err());
        assert!(DepositWalletIdlessSubmitReconciliationEvidence::new(
            owner,
            scope,
            "unit-test owner serialization guard",
            payload_hash,
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
    fn manual_clear_requires_trusted_terminal_observation_when_transaction_is_known() {
        let owner = address(WALLET_CREATE_OWNER);
        let client =
            test_client(DepositWalletRelayerUrl::loopback("http://127.0.0.1:1").unwrap());
        let payload_hash = "payload:known-terminal-observation".to_string();
        client.record_ambiguous(owner, payload_hash.clone()).unwrap();
        client
            .record_transaction_owner("tx-known-terminal", owner, payload_hash.clone())
            .unwrap();

        let evidence = submit_reconciliation_evidence_for_payload_and_transaction(
            owner,
            payload_hash.clone(),
            "tx-known-terminal",
        );
        let error = client
            .clear_ambiguous_submit_after_manual_reconciliation(
                evidence.clone(),
                manual_reconciliation_permit_token_for(owner),
            )
            .unwrap_err();
        assert!(error_has_prefix(&error, RECONCILIATION_REQUIRED_PREFIX));
        assert_eq!(client.ambiguous_submit_block(owner), Some(payload_hash.clone()));
        {
            let state = client.mutation_state().unwrap();
            assert!(state.transaction_owners.contains_key("tx-known-terminal"));
            assert!(!state.terminal_observations.contains_key("tx-known-terminal"));
        }

        {
            let mut state = client.mutation_state().unwrap();
            state.terminal_observations.insert(
                "tx-known-terminal".to_string(),
                OwnerTransactionTerminalObservation {
                    observed_state: RelayerTransactionState::Confirmed,
                    transaction_hash: Some(
                        "0x38cbfbeae8fffa4e2b187ee5978d3ee9cafc53af0363ed90a35b7ea9016535d8"
                            .to_string(),
                    ),
                },
            );
        }
        let error = client
            .clear_ambiguous_submit_after_manual_reconciliation(
                evidence.clone(),
                manual_reconciliation_permit_token_for(owner),
            )
            .unwrap_err();
        assert!(error_has_prefix(&error, RECONCILIATION_REQUIRED_PREFIX));
        assert_eq!(client.ambiguous_submit_block(owner), Some(payload_hash.clone()));
        {
            let state = client.mutation_state().unwrap();
            assert!(state.transaction_owners.contains_key("tx-known-terminal"));
            assert!(state.terminal_observations.contains_key("tx-known-terminal"));
        }

        {
            let mut state = client.mutation_state().unwrap();
            state.terminal_observations.insert(
                "tx-known-terminal".to_string(),
                OwnerTransactionTerminalObservation {
                    observed_state: RelayerTransactionState::Failed,
                    transaction_hash: None,
                },
            );
        }
        client
            .clear_ambiguous_submit_after_manual_reconciliation(
                evidence,
                manual_reconciliation_permit_token_for(owner),
            )
            .unwrap();

        client.ensure_owner_unblocked(owner).unwrap();
        let state = client.mutation_state().unwrap();
        assert!(!state.transaction_owners.contains_key("tx-known-terminal"));
        assert!(!state.terminal_observations.contains_key("tx-known-terminal"));
    }

#[test]
    fn manual_clear_rejects_terminal_observation_transaction_hash_mismatch() {
        let owner = address(WALLET_CREATE_OWNER);
        let client =
            test_client(DepositWalletRelayerUrl::loopback("http://127.0.0.1:1").unwrap());
        let payload_hash = "payload:terminal-hash-mismatch".to_string();
        let transaction_id = "tx-terminal-hash-mismatch";
        client.record_ambiguous(owner, payload_hash.clone()).unwrap();
        client
            .record_transaction_owner(transaction_id, owner, payload_hash.clone())
            .unwrap();
        {
            let mut state = client.mutation_state().unwrap();
            state.terminal_observations.insert(
                transaction_id.to_string(),
                OwnerTransactionTerminalObservation {
                    observed_state: RelayerTransactionState::Confirmed,
                    transaction_hash: Some(
                        "0x38cbfbeae8fffa4e2b187ee5978d3ee9cafc53af0363ed90a35b7ea9016535d8"
                            .to_string(),
                    ),
                },
            );
        }

        let error = client
            .clear_ambiguous_submit_after_manual_reconciliation(
                submit_reconciliation_evidence_for_payload_transaction_observation(
                    owner,
                    payload_hash.clone(),
                    transaction_id,
                    RelayerTransactionState::Confirmed,
                    Some("0x48cbfbeae8fffa4e2b187ee5978d3ee9cafc53af0363ed90a35b7ea9016535d8"),
                ),
                manual_reconciliation_permit_token_for(owner),
            )
            .unwrap_err();

        assert!(error_has_prefix(&error, RECONCILIATION_REQUIRED_PREFIX));
        assert_eq!(client.ambiguous_submit_block(owner), Some(payload_hash));
        let state = client.mutation_state().unwrap();
        assert!(state.transaction_owners.contains_key(transaction_id));
        assert!(state.terminal_observations.contains_key(transaction_id));
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
    fn owner_mutation_state_allows_existing_transaction_update_at_capacity() {
        let client =
            test_client(DepositWalletRelayerUrl::loopback("http://127.0.0.1:1").unwrap());
        let owner = address(WALLET_CREATE_OWNER);
        {
            let mut state = client.mutation_state().unwrap();
            for index in 0..MAX_OWNER_MUTATION_RECORDS {
                state.transaction_owners.insert(
                    format!("tx-{index}"),
                    OwnerTransactionRecord {
                        owner,
                        payload_hash: format!("payload-{index}"),
                        source: OwnerTransactionSource::LocalSubmit,
                    },
                );
            }
        }

        client
            .record_transaction_owner("tx-0", owner, "payload-0".to_string())
            .unwrap();
        let state = client.mutation_state().unwrap();
        assert_eq!(state.transaction_owners.len(), MAX_OWNER_MUTATION_RECORDS);
        assert_eq!(
            state
                .transaction_owners
                .get("tx-0")
                .map(|record| (record.owner, record.payload_hash.as_str())),
            Some((owner, "payload-0"))
        );
    }

#[test]
    fn owner_mutation_state_terminal_clear_releases_transaction_capacity() {
        let client =
            test_client(DepositWalletRelayerUrl::loopback("http://127.0.0.1:1").unwrap());
        let owner = address(WALLET_CREATE_OWNER);
        let payload_hash = "payload:capacity-terminal-clear".to_string();
        {
            let mut state = client.mutation_state().unwrap();
            state.owner_blocks.insert(
                owner,
                OwnerMutationBlock::InFlight {
                    payload_hash: payload_hash.clone(),
                    transaction_id: Some("tx-terminal-capacity".to_string()),
                    created_at_unix_seconds: 1_700_000_000,
                },
            );
            for index in 0..MAX_OWNER_MUTATION_RECORDS {
                state.transaction_owners.insert(
                    if index == 0 {
                        "tx-terminal-capacity".to_string()
                    } else {
                        format!("tx-existing-{index}")
                    },
                    OwnerTransactionRecord {
                        owner,
                        payload_hash: if index == 0 {
                            payload_hash.clone()
                        } else {
                            format!("payload-existing-{index}")
                        },
                        source: OwnerTransactionSource::LocalSubmit,
                    },
                );
            }
        }

        client
            .clear_transaction_block_if_current("tx-terminal-capacity", owner, &payload_hash)
            .unwrap();

        {
            let state = client.mutation_state().unwrap();
            assert_eq!(state.transaction_owners.len(), MAX_OWNER_MUTATION_RECORDS - 1);
            assert!(!state.transaction_owners.contains_key("tx-terminal-capacity"));
            assert!(state.transaction_owners.contains_key("tx-existing-1"));
        }
        client.ensure_owner_unblocked(owner).unwrap();
        let mut reservation = client
            .reserve_owner_submit(owner, "payload:after-capacity-clear".to_string())
            .unwrap();
        reservation.clear().unwrap();
    }

#[test]
    fn recovered_transaction_without_payload_record_does_not_allocate_at_capacity() {
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

        client
            .record_recovered_inflight_transaction(owner, "tx-over-capacity")
            .unwrap();

        assert_eq!(
            client.ambiguous_submit_block(owner),
            Some("payload:ambiguous-owner".to_string())
        );
        let state = client.mutation_state().unwrap();
        assert_eq!(state.transaction_owners.len(), MAX_OWNER_MUTATION_RECORDS);
        assert!(!state.transaction_owners.contains_key("tx-over-capacity"));
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
        let other_payload_owner = address("0x0000000000000000000000000000000000000002");
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
                "tx-owner-other-payload".to_string(),
                OwnerTransactionRecord {
                    owner: other_payload_owner,
                    payload_hash: "payload-owner-other".to_string(),
                    source: OwnerTransactionSource::OwnerRecovery,
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
                .get("tx-owner-other-payload")
                .map(|record| (record.owner, record.payload_hash.as_str())),
            Some((other_payload_owner, "payload-owner-other"))
        );
        assert_eq!(
            state
                .transaction_owners
                .get("tx-other-live")
                .map(|record| record.owner),
            Some(other_owner)
        );
    }

#[test]
    fn manual_clear_rejects_additional_same_payload_transaction_records() {
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
                "tx-owner-other".to_string(),
                OwnerTransactionRecord {
                    owner,
                    payload_hash: "payload-owner".to_string(),
                    source: OwnerTransactionSource::OwnerRecovery,
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
            state.terminal_observations.insert(
                "tx-owner-stale".to_string(),
                OwnerTransactionTerminalObservation {
                    observed_state: RelayerTransactionState::Failed,
                    transaction_hash: None,
                },
            );
            state.terminal_observations.insert(
                "tx-owner-other".to_string(),
                OwnerTransactionTerminalObservation {
                    observed_state: RelayerTransactionState::Failed,
                    transaction_hash: None,
                },
            );
        }

        let error = client
            .clear_ambiguous_submit_after_manual_reconciliation(
                submit_reconciliation_evidence_for_payload_and_transaction(
                    owner,
                    "payload-owner",
                    "tx-owner-stale",
                ),
                manual_reconciliation_permit_token_for(owner),
            )
            .unwrap_err();

        assert!(error_has_prefix(&error, RECONCILIATION_REQUIRED_PREFIX));
        assert_eq!(
            client.ambiguous_submit_block(owner),
            Some("payload-owner".to_string())
        );
        let state = client.mutation_state().unwrap();
        assert!(!state.transaction_owners.contains_key("tx-owner-stale"));
        assert!(!state.terminal_observations.contains_key("tx-owner-stale"));
        assert!(state.transaction_owners.contains_key("tx-owner-other"));
        assert_eq!(
            state
                .transaction_owners
                .get("tx-other-live")
                .map(|record| record.owner),
            Some(other_owner)
        );
        drop(state);

        client
            .clear_ambiguous_submit_after_manual_reconciliation(
                submit_reconciliation_evidence_for_payload_and_transaction(
                    owner,
                    "payload-owner",
                    "tx-owner-other",
                ),
                manual_reconciliation_permit_token_for(owner),
            )
            .unwrap();

        client.ensure_owner_unblocked(owner).unwrap();
        let state = client.mutation_state().unwrap();
        assert!(!state.transaction_owners.contains_key("tx-owner-other"));
        assert!(!state.terminal_observations.contains_key("tx-owner-other"));
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
