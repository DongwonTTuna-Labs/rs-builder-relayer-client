# TESTING.md

## Minimum Commands

`cargo fmt --all --check` is in the list below for parity with CI, but
`rustfmt.toml` sets `disable_all_formatting`, so it exits zero without checking
anything. It is not formatting evidence.

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo build --workspace --all-targets --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps
git diff --check
```

## Required Deposit-Wallet Tests

These tests are required before any production import or live relayer mutation.

```text
derive_deposit_wallet_address_matches_reference
wallet_create_submit_body_matches_fixture
wallet_nonce_uses_type_wallet
wallet_batch_signature_matches_reference_digest
wallet_batch_eip712_impl_matches_all_canonical_digests
wallet_batch_submit_body_matches_fixture
transaction_state_unknown_blocks_live_retry
is_deposit_wallet_deployed_matches_request_and_response_fixtures
is_deposit_wallet_deployed_rejects_malformed_fixture_responses
is_deposit_wallet_deployed_returns_typed_api_error_for_5xx
get_transaction_for_owner_rejects_missing_owner_evidence
read_methods_reject_owner_mismatched_permit_before_input_or_http
read_methods_reject_chain_mismatched_permit_before_input_or_http
mutation_is_default_deny_for_wallet_create_and_wallet_batch_before_http
mutation_permit_validates_references_expiry_and_scoped_getters
mutation_permit_scope_and_expiry_fail_before_http
wallet_batch_deadline_guard_treats_equal_clock_as_expired_before_http
dry_run_builds_redacted_create_and_batch_evidence_without_http
dry_run_and_permit_debug_redact_replayable_and_authorization_material
rollback_latch_disables_all_clones_while_reads_and_dry_run_continue
rollback_blocks_live_mutation_before_network_dispatch
rollback_keeps_read_permit_validation_active_before_network_dispatch
identity_redaction_is_pinned_on_the_normal_library_artifact
identity_config_validates_derivation_and_builds_legacy_context
zero_addresses_are_rejected_before_owner_derivation_validation
owner_derived_wallet_mismatch_is_a_signing_error
configured_overlap_is_observed_without_policy_rejection
summary_and_debug_output_contain_only_redacted_addresses
identity_config_summary_field_set_is_exhaustive
overlap_keys_are_stable
identity_config_surface_is_explicit_private_and_non_coercing
submit_response_anomalies_require_reconciliation_without_resubmission
submit_transport_and_oversized_success_responses_require_reconciliation
deployment_lifecycle_short_circuits_when_wallet_is_already_deployed
deployment_lifecycle_predeployed_policy_blocks_missing_wallet
deployment_lifecycle_requires_explicit_mutation_permit
deployment_lifecycle_live_create_preserves_fixture_body_and_receipt
deployment_lifecycle_dry_run_preserves_evidence_without_submit_http
deployment_lifecycle_propagates_closed_mutation_gate_after_preflight
deployment_readiness_maps_confirmed_pending_and_error_states
deployment_readiness_keeps_wallet_and_wallet_create_types_isolated
deployment_lifecycle_methods_reject_mismatched_read_permits_before_http
execute_wallet_batch_fetches_fresh_nonce_then_submits_verified_live_body
execute_wallet_batch_dry_run_reads_nonce_and_preserves_it_in_evidence
execute_wallet_batch_rejects_signer_identity_before_nonce_read
execute_wallet_batch_prevalidates_mutation_and_read_permits_before_http
execute_wallet_batch_rejects_expired_deadline_before_nonce_read
execute_wallet_batch_rejects_wrong_wallet_before_nonce_read
execute_wallet_batch_rejects_an_empty_batch_before_nonce_read
execute_wallet_batch_rejects_oversized_batch_before_nonce_read
execute_wallet_batch_preserves_submit_api_error_without_duplicate_post
execute_wallet_batch_classifies_submit_disconnect_for_reconciliation
execute_wallet_batch_closed_latch_allows_nonce_read_but_blocks_post
execute_wallet_batch_discards_signer_error_and_source_material
poll_wallet_transaction_returns_confirmed_without_delay
poll_deposit_wallet_deployment_returns_confirmed_without_delay
poll_wallet_transaction_applies_exact_exponential_intervals
poll_wallet_transaction_exhaustion_preserves_last_pending_state
poll_wallet_transaction_caps_exponential_backoff
poll_wallet_transaction_stops_on_failed_invalid_and_unknown_states
poll_wallet_transaction_retries_api_errors_on_policy_schedule
poll_wallet_transaction_retries_transport_error_on_policy_schedule
poll_wallet_transaction_cancels_in_flight_read_without_retry
poll_wallet_transaction_cancels_during_backoff_without_another_read
poll_wallet_transaction_prioritizes_immediate_cancellation_before_http
polling_keeps_wallet_and_wallet_create_transaction_types_isolated
poll_wallet_transaction_treats_missing_array_item_as_transient
poll_wallet_transaction_rejects_mismatched_permit_before_http
relayer_poll_policy_validates_bounds_and_exposes_values
mutation_intent_lease_lifecycle_preserves_versions_timestamps_and_terminal_reentry
mutation_intent_binding_and_terminal_failure_rules_reject_misdelivery_and_regression
mutation_intent_poll_outcomes_are_transaction_bound_and_distrust_variant_names
mutation_intents_block_unresolved_scope_and_keep_other_owner_or_chain_independent
mutation_intent_transition_guards_leave_records_unchanged
mutation_intent_store_assigns_generations_fences_stale_writers_and_fails_closed_at_bounds
mutation_intent_restart_recovery_uses_registry_terminal_failure_without_rebuilding_lease
mutation_intent_store_errors_fail_closed_and_post_submit_recording_failure_keeps_lock
intent_gated_execute_records_submission_then_confirmed_poll_and_reopens_owner
intent_gated_execute_rejects_existing_owner_before_nonce_or_http
intent_gated_execute_maps_disconnect_to_ambiguous_and_local_deadline_failure_to_failed
intent_gated_dry_run_reads_nonce_without_creating_a_lease
intent_gated_create_and_lifecycle_dry_runs_never_create_a_lease
intent_gated_invalid_transaction_id_is_ambiguous_without_echoing_raw_identity
intent_gated_api_failures_use_conservative_ambiguous_phase_classification
intent_gated_create_and_lifecycle_wrappers_preserve_outcomes_and_lease_policy
mutation_intent_serialization_and_debug_are_secret_free_and_redacted
intent_gated_submit_recording_failure_returns_store_error_and_leaves_owner_locked
reconciliation_evidence_validates_redacts_and_round_trips_with_legacy_records
reconcile_manually_resolves_all_unresolved_states_and_requires_current_generation
manual_reconciliation_retries_one_cas_miss_and_reports_a_second_miss
transaction_adoption_is_epoch_fenced_evidence_bound_and_poll_resolved
transaction_adoption_rejects_non_ambiguous_statuses
reconcile_by_polling_maps_confirmed_and_terminal_failure_without_submit
reconcile_by_polling_preserves_pending_cancelled_and_unknown_locks_without_submit
reconcile_by_polling_rejects_non_submitted_records_before_http
reconcile_by_polling_uses_wallet_create_type_and_rejects_wallet_receipt
ambiguous_candidate_report_filters_and_redacts_fixture_without_state_change
ambiguous_candidate_report_rejects_scope_before_http_and_enforces_item_limit
ambiguous_candidate_report_rejects_non_array_or_invalid_json
owner_concurrency_blocks_same_owner_before_second_nonce_while_submit_is_held
owner_concurrency_allows_different_owner_while_submit_is_held
owner_concurrency_reopens_only_after_confirmed_reconciliation
owner_concurrency_keeps_ambiguous_and_unknown_owners_blocked
owner_concurrency_block_error_is_stable_and_record_preserving
owner_concurrency_blocks_create_and_deploy_after_read_preflight
mutation_audit_artifact_tracks_poll_attempts_and_summarizes_reconciliation
mutation_audit_artifact_sanitizes_unknown_labels_at_write_and_export_boundaries
public_observability_debug_and_artifact_json_omit_all_secret_sentinels
failure_displays_apply_the_same_secret_redaction_contract
mutation_intent_tracing_is_structured_complete_and_redacted
pusd_approval_call_matches_recorded_batch_call
approval_calls_encode_supplied_spender_operator_and_amount
pusd_amount_round_trips_finite_base_units_into_calldata
pusd_approval_call_matches_fixture
ctf_approval_for_all_call_matches_fixture
approval_builders_reject_addresses_outside_verified_allowlists
approval_builders_reject_zero_address_targets
pusd_approval_respects_narrowed_config_allowlist
ctf_approval_respects_narrowed_config_allowlist
ctf_approval_for_all_encodes_revocation
pusd_amount_rejects_zero_and_converts_whole_units
pusd_amount_serializes_decimal_base_units_with_decimals
pusd_approval_call_rejects_zero_amount_from_unchecked_construction
calldata_amount_keeps_base_units_private
ctf_split_position_call_matches_fixture
ctf_merge_positions_call_matches_fixture
ctf_redeem_positions_call_matches_fixture
neg_risk_redeem_positions_call_matches_fixture
ctf_calls_encode_supplied_condition_partition_and_amount
ctf_redeem_and_neg_risk_encode_supplied_index_sets_and_amounts
ctf_builders_reject_invalid_partitions_and_index_sets
neg_risk_redeem_rejects_adapter_outside_verified_allowlist
neg_risk_redeem_respects_narrowed_adapter_allowlist
ctf_position_amount_rejects_zero_and_serializes_decimal_base_units
ctf_route_target_defines_every_adapter_combination
ctf_route_selector_matches_verified_wire_truth
ctf_builders_reject_zero_amount_from_unchecked_construction
ctf_split_and_merge_reject_unlimited_amount
neg_risk_redeem_accepts_duplicate_amounts
calldata_position_keeps_base_units_private
summary_exposes_only_review_metadata_and_preserves_order
four_byte_data_does_not_become_replayable_selector_output
every_builder_output_composes_into_an_ordered_batch_summary
batch_summary_omits_full_calldata_full_targets_and_signatures
wallet_batch_preflight_rejects_each_resource_limit_independently
wallet_batch_preflight_rejects_wallet_chain_and_signer_mismatches
narrowed_calldata_config_blocks_batch_composition_before_signing
fixture_builder_output_is_compatible_with_wallet_submit_body
summary_selector_boundary_omits_four_bytes_and_keeps_five_byte_routes
summary_handles_empty_calldata_without_losing_exact_lengths
summary_accepts_an_empty_batch_without_validation
relayer_auth_address_not_used_as_owner_implicitly
ambiguous_submit_timeout_does_not_duplicate_submit
idless_submit_timeout_blocks_owner_until_manual_reconcile
canonical_polygon_config_exposes_reviewed_values_sources_and_membership
strict_subsets_are_preserved_without_global_allowlist_expansion
unsupported_chain_is_rejected
structural_duplicates_collisions_zero_and_empty_lists_are_rejected
every_wire_truth_binding_has_a_distinct_failure
source_metadata_rejects_empty_oversized_control_and_insecure_values
serialize_uses_checksum_addresses_and_includes_source_metadata
calldata_config_surface_is_explicit_validated_and_synchronous
```

## Fixture Rules

- Fixtures must be sanitized.
- Fixtures must not contain production private keys, API credentials, auth headers, or raw production signatures.
- Fixture names should include endpoint and scenario.
- If a fixture comes from official TypeScript/Python SDK behavior, record the SDK version or commit.
- Recent-transaction fixtures must distinguish schema construction from a
  live-recorded response and must never be cited as proof that a candidate is
  the original ambiguous submission.

Recommended fixture layout:

```text
tests/fixtures/
  deposit_wallet/
    derive_address.json
    wallet_create_submit_body.json
    wallet_create_transaction_response.json
    wallet_recent_transactions_response.json
    wallet_nonce_request.json
    wallet_deployed_http_request.json
    wallet_deployed_response_cases.json
    wallet_batch_eip712.json
    wallet_submit_body.json
    calldata_pusd_approval_call.json
    calldata_ctf_approval_for_all_call.json
    calldata_ctf_split_position_call.json
    calldata_ctf_merge_positions_call.json
    calldata_ctf_redeem_positions_call.json
    calldata_neg_risk_redeem_positions_call.json
  relayer/
    transaction_states.json
```

## Golden Test Scope

Golden tests should prove:

- `WALLET-CREATE` request shape is not Safe deploy shape;
- `WALLET` request shape is not Safe/Proxy execute shape;
- `/nonce?type=WALLET` is used immediately before signing;
- EIP-712 domain, message, digest, and signature shape match reference behavior;
- relayer auth identity can differ from wallet owner signer;
- `/transactions` sends no query, filters only validated same-owner
  WALLET/WALLET-CREATE items, omits unknown labels, and changes no intent;
- `/deployed` uses the derived deposit-wallet address and accepts only an object
  with a boolean `deployed` field;
- owner- or chain-mismatched read permits fail before input validation, URL
  construction, or HTTP I/O;
- `DepositWalletRelayerClient::new` denies both live mutation operations before
  HTTP, even when a valid `Live` permit is supplied;
- mutation permit mode, operation, owner, chain, expiry, evidence reference,
  and operator-approval reference are validated, and scope mismatch or
  `now >= expiry` fails before HTTP;
- a signed batch with `now >= deadline` fails before HTTP;
- `DryRun` works independently of the live latch, sends no HTTP, and emits only
  redacted evidence: no auth headers, signature, full calldata, or full submit
  body may appear in serialized or debug output;
- `disable_mutation` blocks live submits through the client and every clone
  while reads and valid `DryRun` submissions continue;
- invalid/partial submit responses, post-dispatch transport failures, and
  oversized 2xx responses require reconciliation rather than resubmission;
- the deployment lifecycle always checks `/deployed` first, short-circuits on
  `true`, and blocks a missing wallet under `Predeployed`, absent mutation
  authority, or a closed live latch without sending a submit request;
- the live create lifecycle POST is JSON-equivalent to
  `wallet_create_submit_body.json`, contains no `signature` key, and preserves
  both transaction id and payload hash in its receipt;
- dry-run deployment preserves redacted evidence and performs only the
  deployed read, never a submit request;
- WALLET-CREATE readiness maps only Confirmed to `Ready`, maps New/Executed/
  Mined to `Pending`, and propagates Failed, Invalid, Unknown, malformed, and
  ambiguous evidence as errors requiring stop or reconciliation;
- public WALLET reads reject WALLET-CREATE responses, deployment readiness
  rejects WALLET responses, and both lifecycle methods reject mismatched read
  permits before HTTP;
- ethers `Eip712::encode_eip712`, the canonical deposit-wallet digest, and all
  three single-call, Amoy, and multicall expected digests are byte-identical;
- fresh-nonce execution prevalidates both permits, deadline, signer identity,
  resource limits, and the derived wallet before `GET /nonce`, then performs
  no other HTTP await before signing and at most one reviewed submit;
- live execution preserves the submitted transaction id and redacted payload
  hash, while DryRun preserves the fetched nonce and sends no POST;
- a closed live latch may observe the nonce read but blocks the POST, and
  signer backend error text and source chains are discarded before returning;
- polling maps only Confirmed to success, retains New, Executed, and Mined as
  pending, propagates Failed and Invalid immediately, and stops for Unknown or
  type/owner/permit reconciliation errors;
- polling performs exactly the configured attempts, sleeps only between
  attempts, doubles `1s` to `2s`, applies the maximum-interval cap, and returns
  `Exhausted` rather than an error after the final pending/transient attempt;
- 429 with `Retry-After`, 5xx, transport errors, and temporary array absence are
  transient only within the finite policy, while cancellation reports completed
  attempts and never starts another read;
- WALLET and WALLET-CREATE polling remain type-isolated and no polling outcome
  invokes submit, signing, nonce fetch, or automatic reconciliation;
- mutation intent begin is atomic and store-issued, rejects a second unresolved
  owner/chain lease before nonce or HTTP, and keeps different owners/chains
  independent;
- record updates preserve creation time, refresh update time, advance revision
  exactly once, and reject stale epoch/revision writers without changing the
  newer record;
- Preparing, Submitted, and AmbiguousNoId remain unresolved, while Confirmed,
  bound terminal Failed, and Reconciled admit a new generation without epoch or
  revision wrap;
- receipt, poll, and terminal-failure writes are transaction-bound; a
  mislabeled Confirmed variant, mismatched id, delayed old-intent result, and
  non-terminal error cannot release the owner;
- a registry reconstructed over the same store sees Submitted, blocks begin,
  records a bound terminal failure without a lease, and then admits a successor;
- gated live execute blocks before nonce I/O, records payload/deadline/id after
  success, maps submit uncertainty and all API failures to AmbiguousNoId, and
  maps proven local pre-submit failures to Failed;
- DryRun creates no intent, deployment AlreadyDeployed/Predeployed paths do not
  pollute owner history, and the live DeployIfMissing wrapper records exactly
  the intended extra preflight reads and submit;
- record JSON and Debug contain no raw signature, API key, auth header, private
  key, calldata, or replayable body, while Debug redacts owner and transaction
  id;
- begin/update store errors, stale CAS, epoch exhaustion, and revision
  exhaustion fail closed, and a post-submit record failure leaves Preparing
  unresolved without echoing the transaction id;
- unknown transaction states force non-mutating behavior.

## Production Read Transport Gate

The production-capable read transport is verified only against deterministic
local loopback servers. Tests cover exact deployed/nonce/transaction request
shape, auth header presence and sensitivity, redirects, 4xx/5xx and 429
classification, Retry-After, bounded bodies, malformed response evidence, and
permit rejection before HTTP. CI must not call the production host or require a
live relayer credential.

PBRSDK-8 lifecycle tests are single-shot and use only the same deterministic
loopback transport plus injected clock. They must not add polling loops, sleep,
retry, cancellation, recent-transaction lookup, or live host calls.

PBRSDK-9 execute tests use the same deterministic loopback transport, injected
clock, and `[0x42u8; 32]` synthetic throwaway signer key. That constant is
never a real credential and must not be replaced with a funded or production
key. These tests prove call ordering and local validation only; they do not
introduce a nonce lease, retry, polling, or live host call.

PBRSDK-10 polling tests use `#[tokio::test(start_paused = true)]` and the
production `tokio::time::sleep` calls directly; there is no sleeper injection
and no wall-clock sleep. The polling-only reqwest client has redirects disabled
and no request timeout. Its dedicated loopback server uses plain
`listener.accept().await` and plain stream reads, with no Tokio timeout. This
keeps non-cancellation I/O sections free of timers, so Tokio virtual time
advances only by the exact `1s + 2s` or capped backoff schedule under test.

The missing accept timeout is deliberate under paused time: a timeout would be
the next timer during I/O and auto-advance the virtual clock instead of leaving
I/O at zero elapsed time. The residual tradeoff is that a client bug that sends
too few requests can hang an individual polling test. CI and command-runner
timeouts are the hang guard; the test server does not add an internal timer.
Cancellation is the one intentional extra timer. The in-flight case holds the
first response after observing one GET, then cancels the read after 500ms and
reports zero completed attempts. The sleep-stage case writes and flushes a
STATE_NEW response before arming its 500ms cancellation timer; attempt one
therefore completes, cancellation wins the following 1s backoff, and one
completed attempt is reported. Both cases observe exactly one GET and no
second GET.

PBRSDK-12 intent tests reuse the same loopback transport and injected clock.
The only supplied store is a single-`Mutex` in-memory implementation; tests use
it to prove the atomic begin contract, sequential rejection, generation and
revision fencing, restart behavior through a newly constructed registry over
the same store, and fail-closed boundaries. This is not a durable-store or
concurrency stress qualification; deterministic concurrent expansion belongs
to PBRSDK-14. No live host, retry loop, mock venue, file store, or database
harness is introduced.

PBRSDK-13 tests remain in the same HTTP unit-test module and add one sanitized
recent-transactions fixture. Registry setup uses only `begin_intent` and the
existing lease recording methods, so reconciliation request logs are not
contaminated by setup submits. The tests prove evidence validation/redacted
Debug/serde compatibility, epoch ABA fencing, evidence-bound AmbiguousNoId
adoption, one bounded CAS retry, stored-id polling for both operations,
Confirmed/Failed/pending/cancelled/unknown mappings, permit-first reporting,
per-item filtering, response limits, and report immutability. Every polling
reconciliation request log asserts zero `POST /submit` calls. The recent fixture
is schema-constructed from official TypeScript `RelayerTransaction` fields and
the recorded-style WALLET fixture; it is not live-recorded evidence.

PBRSDK-14 owner-concurrency tests fix the crate policy at immediate blocking:
an unresolved owner receives the stable `mutation_blocked` error from
`begin_intent`; queue tickets and scheduling remain the consumer actor's
ADR-0013 responsibility. The deterministic scenarios prove that a held first
submit blocks the same owner before a second nonce fetch, a different owner
completes independently, Submitted remains blocked until a matching Confirmed
poll outcome and then permits a complete successor mutation, AmbiguousNoId and
an unknown polling result retain the lock, three repeated block attempts leave
the record and epoch unchanged, and WALLET-CREATE/deployment use the same
owner-wide gate. Deployment still performs its read-only `/deployed` preflight
exactly once before the blocked create decision; it performs no nonce fetch or
submit.

Every PBRSDK-14 test uses `#[tokio::test(start_paused = true)]`, a
mutation-enabled client assembled with `FixedClock` and a timeout-free reqwest
client. Every server that expects a request uses polling-style accept and
request-read paths with no timer. Zero-request assertions alone reuse
`spawn_optional_request_server`: its `NO_REQUEST_TIMEOUT` intentionally
auto-advances paused time to finish the negative observation, and its timed
request reader runs only if an unexpected request arrives. The held-submit
server signals request observation through a oneshot, waits on a second oneshot
for release, and only then writes the submit response. Each scenario has
exactly one registry coordination domain; separate servers and HTTP clients
isolate request logs without separating the shared store. There is no real
sleep or wall-clock ordering assertion.

The timer-free held server has the same deliberate residual risk as the
polling servers: if a regression sends fewer or more requests than the fixed
sequence expects, an individual test can hang while awaiting I/O or task
completion. CI and command-runner timeouts are the hang guard; adding an
internal timer would reintroduce paused-time auto-advance and invalidate the
deterministic ordering proof.

## PBRSDK-15 Redaction And Observability Gate

The mutation audit gate must prove all of the following with unique secret and
malicious-text sentinels:

- pre-`poll_attempts` record JSON deserializes with zero, Exhausted with a
  present state accumulates its attempts, matching Confirmed adds one, and
  Exhausted without a state plus Cancelled remain write-free no-ops;
- Unknown text is replaced at normal poll write time, and a separate durable
  row preloaded through Deserialize with an unsafe state label is replaced
  again at export time;
- artifact JSON contains every schema-v1 key, a shortened owner, the fixed
  omission marker, and the raw operator-usable transaction id, while artifact
  Debug omits that id and contains its `sha3:0x...` token;
- reconciliation export contains only decision, `value.len()` UTF-8 byte
  lengths, and recorded time. Preloaded operator reference and summary
  sentinels never appear in artifact JSON or Debug;
- Debug for every reviewed public observability type, failure Display/Debug,
  artifact JSON, and captured tracing omit the API key, auth-header name,
  private-key bytes, signature, full calldata, typed data, and replayable body;
- the tracing test owns the guard returned by
  `tracing::subscriber::set_default` across every await in a default
  current-thread `#[tokio::test(start_paused = true)]`. It must not use
  `with_default` or `flavor = "multi_thread"`;
- every tracing line has redacted owner, chain id, and epoch, plus only the
  ADR-0015 event-specific fields. Registry resolution is absent on a stale CAS
  and nonce/reference/summary/body fields are absent everywhere.

The tracing test uses the existing timeout-free loopback path and the already
declared `tracing-subscriber` dev dependency. PBRSDK-15 adds no fixture, Cargo
target, dependency, mock venue, logging collector, metrics system, OTel stack,
live host call, or credential.

## PBRSDK-17 Verified Calldata Config Gate

The pure synchronous config tests must prove the canonical Polygon addresses,
six-decimal unit, and exact source metadata through public getters. They must
also prove that strict allowlist subsets are preserved rather than expanded;
unsupported chains, zero addresses, token collisions, duplicate entries,
self-approval, and empty spender/operator lists fail before any builder can run.

Each wire-truth binding has a distinct negative case: arbitrary pUSD, arbitrary
CTF, decimals `18`, a non-reviewed spender, a non-reviewed operator, and an
adapter outside the single PBRSDK-19 Polygon adapter truth. Empty adapter lists
remain valid strict subsets. Source tests cover empty, oversized, and
control-bearing names and versions plus insecure, oversized, and
control-bearing URLs. JSON tests require checksum addresses and source metadata.
The config types derive `Serialize` only and must not expose `Deserialize`,
environment loading, or file loading.

`tests/public_api_boundary_test.rs` pins all four config types, every reviewed
config constructor/getter/helper signature, the five PBRSDK-17 crate-root
exports, and the canonical constructor function type. It also rejects
`reqwest` and public async functions anywhere in the calldata module. The
PBRSDK-17 portion remains a local config/source audit with no HTTP test or
encoding; PBRSDK-18 and PBRSDK-19 add the separately gated encoders below. None
adds a live request.

The required source hygiene checks remain match-zero gates:

```bash
grep -rn "pub use .*::\*" src/
grep -rn "dbg!\|println!" src/deposit_wallet/
grep -rn "allow(" src/deposit_wallet/http.rs src/deposit_wallet/http/ src/deposit_wallet/calldata/ --include=*.rs | grep -v "http/tests.rs"
```

## PBRSDK-18 Approval Calldata Gate

The module-local amount and approval tests named in the required list above are
mandatory. They must prove all of the following:

- the pUSD `uint256::MAX` call is target/value/data byte-identical to the
  recorded local `wallet_submit_body.json` call;
- a different verified spender, finite one-pUSD amount, and different verified
  CTF operator appear in their exact ABI words, preventing fixed-payload
  implementations;
- finite base units survive `PusdAmount` construction and builder encoding
  unchanged;
- both flat fixtures match the serialized `target`, decimal-string `value`, and
  hex `data`, while selector, address, boolean, amount, decimals, and unit
  metadata are checked independently;
- well-formed outsider and zero addresses fail, and a caller-supplied narrowed
  config remains authoritative for each builder;
- zero amount constructors fail, every `u64` whole-pUSD value converts through
  `u128`, manual JSON/Debug use decimal base units and decimals, and a test-only
  invariant bypass proves the builder's independent zero guard;
- CTF `approved = false` encodes a zero boolean word without bypassing the
  operator allowlist.

`tests/public_api_boundary_test.rs` additionally pins the three additive
crate-root exports and exact builder signatures, requires private `amount` and
`approval` modules with explicit re-exports, and fixes the exact private
`PusdAmount` field declaration through
`calldata_amount_keeps_base_units_private`. The combined calldata source must
remain synchronous and free of HTTP/runtime deserialization paths and
`crate::operations` / `crate::contracts` references.

The source matrix must require `SM-CALLDATA-PUSD-ADDR`,
`SM-CALLDATA-CTF-EXCHANGES`, `SM-CALLDATA-PUSD-DECIMALS`, and
`SM-CALLDATA-APPROVAL-ENCODING`; the non-recursive provenance audit must cover
both new flat fixtures. This gate authorizes offline call construction only. It
does not by itself authorize split/merge/redeem, adapter routing, batch
composition, signing, HTTP submission, or live execution.

## PBRSDK-19 CTF Route Calldata Gate

The fifteen named module-local tests plus
`calldata_position_keeps_base_units_private` are mandatory. Together they must
prove:

- the four golden fixtures match target, zero value, complete calldata, route,
  selector, condition id, zero parent, route array, collateral, adapter, and
  unit metadata exactly;
- alternate non-repeating condition bytes and distinct split, merge, CTF
  redeem, and NegRisk arrays appear at their measured ABI head/tail offsets,
  and every result differs from its fixed fixture;
- split/merge/redeem collateral is always the supplied config's pUSD address,
  parent collection is zero, and CTF redeem encodes no amount;
- partitions and index sets reject empty, zero, duplicate, and 65-entry input,
  while 64 entries remain supported; NegRisk quantities reject only empty,
  zero, and 65-entry input and explicitly accept duplicate quantities;
- split/merge reject constructor-bypassing zero and `PusdAmount::unlimited()`,
  while NegRisk independently rejects a constructor-bypassing zero
  `CtfPositionAmount`;
- every `CtfRoute` selector and every route/optional-adapter target combination
  is closed and exact, including distinct missing, zero, and outside-allowlist
  adapter errors;
- a config narrowed to an empty adapter subset disables NegRisk redeem even
  though the canonical config includes the reviewed adapter;
- `CtfPositionAmount` retains one private `U256` field and manually serializes
  decimal base units with `decimals = 6`.

`tests/public_api_boundary_test.rs` pins both additive types, all four builder
signatures, the public route methods, the six crate-root exports, the
calldata-module adapter constant export, private `ctf`/`position` modules, and
the exact three-line `CtfPositionAmount` field declaration. The combined
calldata source must remain synchronous and contain no HTTP dependency,
runtime `Deserialize`, `crate::operations`, or `crate::contracts` reference.
The recursively aggregated non-test deposit-wallet HTTP production surface
must also contain no `crate::operations` or `crate::contracts` reference.

`SM-CALLDATA-CTF-ROUTES` and all four new flat fixtures are required by the
source-matrix/provenance audits. That row records both observed legacy drifts.
Any new address, signature, selector, argument, or unit drift blocks the live
gate until the matrix, ADR, fixtures, implementation, and full unchanged-source
evidence are reviewed again. The legacy public paths are not a fallback for a
deposit-wallet WALLET batch. This gate authorizes only individual offline
`DepositWalletCall` construction; batch composition, position-id calculation,
condition preparation, signing, HTTP, CLOB synchronization, and live execution
remain outside scope.

## PBRSDK-20 WALLET Batch Composition Gate

The two module-local summary tests and all nine tests in
`tests/calldata_batch_composition_test.rs` are mandatory. Together they must
prove:

- all six reviewed builder outputs expose fixture-matching target, value, and
  data, compose in one ordered vector, and retain their route selector and
  exact data length in the corresponding summary index;
- call count, per-call data length, and total calldata bytes are exact, including
  empty data, mixed empty/non-empty calls, and an empty batch;
- serialized and Debug summaries contain the fixed redaction marker, redacted
  targets, and no full target or full calldata;
- exactly four bytes yields no selector and no payload bytes in JSON/Debug,
  while five bytes retains only the first four-byte selector;
- the existing public request builder independently rejects 257 calls and one
  `1024 * 1024 + 1` byte call with distinct error substrings. The private
  resource constants are neither imported nor asserted; the public preflight
  behavior is the contract exercised here;
- wrong derived wallet, Polygon/Amoy config mismatch, unsupported contract
  config, and fixture `nonOwnerSignature` each fail before any submit path, with
  the signer case specifically reporting `signer must match owner`;
- a strict config excluding CTF from the pUSD spender subset prevents the
  approval call and therefore prevents batch construction before signing;
- the canonical builder approval plus the reviewed owner, nonce, deadline,
  derived wallet, and synthetic signature passes the public WALLET request
  builder, and serialized `depositWalletParams.calls[0]` matches the fixture in
  all three fields.

`tests/public_api_boundary_test.rs` must additionally pin the three crate-root
exports, private `summary` module, exact private field blocks for both summary
types, every getter/function signature, and the summary source in the combined
synchronous/no-HTTP/no-Deserialize/no-legacy-reference audit. No fixture,
provenance ledger, source-matrix row, dependency, feature, or wire truth is
added by this gate.

This gate does not claim deadline freshness from
`try_build_wallet_batch_request_with_signature`; that function intentionally
has no wall clock. Deadline freshness remains covered by the existing
clock-injected `http/execute.rs` and `http/submit.rs` tests. PBRSDK-20 adds no
signer, nonce fetch, HTTP call, live submit, actor orchestration, or live-
readiness authority.

## PBRSDK-22a Typed Identity And Rollback Boundary Gate

The module-local tests in `src/deposit_wallet/identity.rs`, both tests in
`tests/mutation_rollback_boundary_test.rs`, and the production-artifact
redaction test plus expanded audits in `tests/public_api_boundary_test.rs` are
mandatory. Together they must prove:

- `RelayerAuthIdentity`, `DepositWalletOwner`, and `DepositWalletAddress` keep
  private tuple fields;
- all three zero-address cases return `RelayerError::InvalidAddress` before
  owner-to-wallet derivation validation, while a non-zero derived-wallet
  mismatch returns `RelayerError::Signing`;
- a valid owner/config-derived deposit wallet is accepted, the existing
  `DepositWalletRequestContext` is produced without changing its public fields,
  and `RelayerKeyAuth::from_identity` delegates to the unchanged constructor;
- equal role pairs are observed rather than rejected, all overlaps are returned
  in enum declaration order, and their three string keys remain stable;
- **Every-build compile-time field pin:** the non-`cfg` production helper
  exhaustively destructures all five private `IdentityConfigSummary` fields
  without `..`; an additional field, including `#[serde(skip)]` under
  `cfg(not(test))`, fails that build configuration with E0027;
- **Production-artifact integration exact assertions:** fixed literals pin
  ordinary and alternate Debug for all three newtypes, the config, and the
  summary, plus the complete summary JSON, without calling the production
  redaction helper as the expected-value oracle;
- **Fixed sentinel assertions:** the same normal-artifact outputs omit the
  listed lowercase raw addresses and six listed address/checksum-string hashes.
  This covers those exact encodings, not uppercase, base64, or decimal
  byte-array forms;
- after `disable_mutation`, a valid Live WALLET-CREATE permit is rejected by
  the mutation predicate and disabled-latch message before network dispatch;
- on the same disabled client, owner-mismatched and chain-mismatched read
  permits still return the read-blocked predicate before network dispatch;
- **Compile-time negative trait assertions:** each identity newtype and config
  lacks `Display`, `Deref`, and `Serialize`, plus both the HRTB
  `for<'de> Deserialize<'de>` and exact `Deserialize<'static>` instantiations;
- **Compile-time negative trait assertions:** every cross-role direction is
  checked separately for owned `From`, owned direct `Into`,
  `From<&'static Source>`, and `&'static Source: Into<Destination>`, yielding
  24 assertions. Same-type `Into<Self>` is excluded because reflexive `From`
  provides it, and `&mut` receiver conversions are outside the guaranteed
  matrix and are not asserted;
- **Source audit:** the complete `src/` source contains no literal `impl From<`
  and no fully qualified `::From<` or `::Into<`; `identity.rs` contains no
  `Deref`, `Display`, or `Deserialize` path and zero `cfg(not(test))` strings.
  This is defense in depth because aliases, macros, and build-script output can
  bypass literal search. The identity module stays private and its six reviewed
  types remain explicitly re-exported;
- implementations generated by macros or build scripts and extensions from
  external crates are outside this proof scope; the gate makes no claim that
  every possible conversion or disclosure encoding is absent;
- the complete production HTTP source retains no public `enable_mutation` and
  adds no public `set_mutation*` path.

The rollback integration target starts no server and makes no timing
assertion. It constructs the validated production URL value only because the
public URL boundary has no loopback constructor. Every awaited method is
deliberately supplied a pre-I/O failing capability or latch condition. A
successful read network round trip is outside this external target: adding a
public test transport, dev-dependency, or bespoke server is forbidden in this
round, and the existing loopback tests in `src/deposit_wallet/http/tests.rs`
already cover successful reads and rollback preserving them. This evidence is
offline and makes no live-read claim.

PBRSDK-22a proves only the fork-owned config and rollback boundary. The
consumer-owned PBRSDK-22b must separately test the exclusive
`pm-adapters/relayer_http` import, fork-DTO-free port/domain surface, and
adapter mappings. CLOB funder/POLY_1271 wiring, consumer code, HTTP behavior,
wire truth, and live execution remain outside this gate.

## PBRSDK-23a Behavior-Based No-CLOB Source Gate

`tests/no_clob_surface_test.rs` is a mandatory offline integration target. It
must prove:

- every `.rs` file under `src/` is recursively discovered, sorted for stable
  diagnostics, and audited, with at least 40 files required before the
  repository assertion can pass;
- the complete reviewed forbidden-marker list is matched case-insensitively
  without removing underscores or normalizing separators;
- the `signature_type`, `/orders`, and `rs-clob-client-v2` exceptions are
  limited to their reviewed paths and exact 8/4/1/2 occurrence counts, with
  every allowed file present and non-empty for that marker;
- the sole `/orders` occurrence in `src/auth/builder.rs` remains after the
  first `#[cfg(test)]` occurrence;
- all reviewed order/post/sign/cancel/book/price/balance-allowance function
  names remain absent after an `fn` token regardless of visibility, `async`,
  repeated Rust `Pattern_White_Space`, non-documenting line comments,
  nested/repeated block comments, mixed trivia, or an optional raw-identifier
  `r#` prefix;
- the whitespace predicate exactly enumerates and independently mutation-tests
  `U+0009` through `U+000D`, `U+0020`, `U+0085`, `U+200E`, `U+200F`, `U+2028`,
  and `U+2029`; it does not use `char::is_whitespace()`, which would miss the
  Rust lexer separators `U+200E` and `U+200F`;
- `post_order_v2` remains outside the exact-name match because `_` continues
  the audit's reviewed underscore/alphanumeric end-boundary check; longer Rust
  identifiers using non-alphanumeric XID continuation marks may be
  conservatively rejected, which is a false positive rather than a trivia
  bypass;
- the forbidden-marker and function-name lists retain their minimum sizes;
- each one-change in-memory mutation returns exactly its expected structured
  violation, and the longer-identifier positive case returns no violation.

The existing path-based CLOB module/example test remains a separate cheap first
defense. This gate mechanically checks only the specified vocabulary and
function signatures in `src/`; it does not prove all possible CLOB code absent,
inspect macro/build-script output, inspect dependencies, qualify consumer CLOB
wiring, or authorize a live call.

## PBRSDK-26 Manual Live Gate Runbook Gate

`tests/live_gate_runbook_test.rs` is a mandatory offline integration target for
`docs/MANUAL_LIVE_GATE_RUNBOOK.md`. It requires the ten runbook section titles,
all seven stop-condition labels, six exact operator-approval gate markers, and
the design-document backlink. Its pure audit returns structured violations,
first passes against the real documents, and then proves exact
one-change/one-violation behavior for a removed heading, a removed
`missing transactionID` label, and an inserted complete-address shape. Longer
hex runs are classified once at the longest matching threshold.

The secret-shape check is intentionally limited to `0x`/`0X`-prefixed ASCII
hex runs plus the specified PEM and bearer-token literal markers in the new
runbook. It does not detect unprefixed 40-hex values, UUID-shaped keys, or
base64 values and does not prove that every possible secret is absent. This
gate adds no dependency, harness, credential, host call, live submit, or
production-source change.

The existing [Manual Live Gate](#manual-live-gate) remains the broader
fork/consumer evidence policy. PBRSDK-26 does not duplicate it: the new runbook
starts only after the ten-item design checklist is complete and supplies the
operator approval, stop, rollback, and redacted PR-evidence procedure for the
first tiny-value deposit-wallet relayer validation.

## PBRSDK-27 Live Validation Decision Gate

`tests/live_validation_decision_test.rs` is a mandatory offline integration
target for `docs/LIVE_VALIDATION_DECISION.md`. Its pure audit parses the
document once with `pulldown-cmark` and table support, then checks one exact
lifecycle marker, an allowlist containing every permitted rendered H2,
status-specific section presence and absence, the canonical `## Decision`
sentence, and the exact `Field` / `Value` / `Redaction` header plus fixed,
ordered 14-row operator-evidence table. Code-block text is non-normative for
those structural rules. The secret-shape scan separately checks the complete
raw document, including code blocks.

The record is limited to the Markdown grammar it actually uses: paragraphs,
headings, lists, code blocks, tables, inline code, and the single-line status
marker HTML comment. It permits exactly one first-line H1 followed only by
allowlisted H2 sections, rejects H3 through H6, and rejects nested lists while
flushing every list item independently. Headings and tables must be top-level
blocks; a heading or table inside a list item or table cell produces
`HeadingInContainer` or `TableInContainer` and cannot create or populate an H2
section. Raw HTML, images, links, emphasis, and every other unapproved tag or
leaf event produce `UnsupportedConstruct`; unsupported container contents
cannot supply canonical Decision text or evidence values. This allowlist makes
unknown syntax an explicit audit failure rather than a silently interpreted
alternative rendering.

The expected Decision sentence must be an exact top-level paragraph in its H2
section, never a list item. The other lifecycle sentences are searched across
all rendered text outside code blocks: paragraphs, list items, every heading,
and all Field, Value, and Redaction table cells. No uncollected rendered context
may silently carry a conflicting lifecycle statement.

The real document must pass before any mutation test runs. Synthetic mutations
cover missing, duplicate, malformed, and unknown markers; ATX and setext H2,
unexpected and duplicate sections, indented and fenced code blocks, HTML
comments, raw HTML, inline HTML, images, and conditional-section drift;
middle or repeated H1, deep headings, nested lists, and conflicting lifecycle
sentences in headings and table cells; headings and tables nested in container
blocks, and Decision text substituted with a list item;
independent and full-supersede Decision mismatches; missing evidence labels;
header and row-order drift; status-specific Value-cell rules; and the specified
prefixed-hex, PEM, and bearer-prefix shapes. Hex runs are classified once at
the longest applicable threshold.

This audit verifies schema consistency, not the factual truth of prose,
blockers, external evidence, or filled observations. It does not compare the
README and does not detect unprefixed hexadecimal, UUID, or base64 forms. It
uses the test-only `pulldown-cmark` dev-dependency, which brings `unicase`
transitively and does not propagate to consumers. It adds no credential, host
call, live submit, CI live test, production dependency, or production-source
change.

## Manual Live Gate

Live relayer checks are operator-gated only. CI must not require production relayer secrets.

Before any real relayer mutation, record evidence for:

```text
deposit wallet derive parity
WALLET-CREATE reaches STATE_CONFIRMED if deployment is in scope
fresh GET /nonce?type=WALLET before signing
WALLET approval batch updates deposit wallet allowance
CLOB balance allowance sync observes deposit wallet state in consumer app
POLY_1271 order path accepts maker/funder shape in consumer app
merge/redeem calldata follows current pUSD adapter path
ambiguous submit timeout does not duplicate transaction
```

For the PBRSDK-7 gate, first record a scoped `DryRun` outcome and operator
review, then create a fresh scoped `Live` permit and use it only with an
explicitly enabled client. These checks prove default denial, review evidence,
and one-way rollback; bounded polling does not replace the persistent
idempotency, reconciliation, or duplicate-submit recovery gate.

For PBRSDK-8, retain the submitted WALLET-CREATE transaction id and payload
hash, then record `STATE_CONFIRMED` through the expected-type readiness path.
Pending or error results forbid lifecycle re-entry for that owner until the
bounded deployment poll observes confirmation and the durable owner registry
records that exact bound result. Exhaustion, cancellation, unknown, ambiguous,
or an unbound error still forbids lifecycle re-entry.

For PBRSDK-12/13, live evidence must use a consumer-supplied durable
`MutationIntentStore`; `InMemoryMutationIntentStore` is forbidden. Capture a
restart exercise proving the unresolved row blocks a new mutation, then prove
that only a matching Confirmed receipt or matching TransactionFailed/Invalid
result, or an epoch-matched operator decision with serialized redacted evidence,
reopens the owner. For an id-less ambiguity, retain the recent report, selected
transaction id when one is manually adopted, evidence text, inspected epoch,
and subsequent authoritative polling result. A report alone must leave the row
unchanged. Preparing manual reconciliation is restart recovery only and
requires proof that no live work for that owner remains. PBRSDK-14 deterministic
concurrency is a separate gate; no gate may be replaced by automatic candidate
selection or automatic resubmit.

For PBRSDK-15, retain both complementary artifacts for the reviewed mutation:
the pre-submit `DepositWalletDryRunEvidence` and the exported
`MutationIntentAuditArtifact`, joined by `payload_keccak256`. Attach only the
redacted audit artifact to tickets/PRs; original reconciliation text remains in
the protected durable store. Preserve captured structured events using the
`polymarket_relayer::mutation_intent` target and verify the closed field set,
but do not add nonce, raw transaction ids, operator text, signatures, auth
material, typed data, or submit bodies to logs. This observability evidence does
not replace the durable-store restart or live execution gates.

`STATE_MINED` may be recorded as pending evidence, but it must not satisfy the
manual live gate. Wallet deployment or wallet-action effects become usable only
after `STATE_CONFIRMED`.

## PBRSDK-28 Release Provenance Audit

`tests/release_provenance_test.rs` is an offline repository-contract audit. It
separates file loading from a pure audit function, proves the real files are
clean before every synthetic mutation, and requires each mutation target to
occur exactly once before replacement.

Before Cargo starts, `python3 -I scripts/preflight_build_integrity.py` parses
the manifest, `docs/accepted-advisories.toml`, and `.cargo/audit.toml` with
`tomllib`. It closes both advisory schemas, compares the canonical register ID
set with the ignore set, fixes both workflow run lines, requires
`publish = false` and the ethers `openssl` feature shape, and requires the
three license files to be non-empty. This is the load-bearing committed-state
check; an earlier unit test cannot repair those inputs before it runs.

The preflight also requires the working tree to be exactly the tree that would
be committed. The authority is `git write-tree`, read from the repository's canonical
metadata and default index with every `GIT_*` variable dropped, lazy fetching
refused, and `core.fsmonitor` and `core.hooksPath` overridden per command, not
the index listing, because `git add -N` records an entry the listing reports and
the tree omits, and because `git replace` can make plain `git ls-tree` answer
with one tree while the commit records another. A repository is identified by
`git rev-parse --show-toplevel` rather than by a `.git` entry at the root, and
the preflight fails when git cannot answer. Content is
compared by hashing each file with `git hash-object --no-filters` against the
recorded object id, because `git diff` honours assume-unchanged and
skip-worktree and plain `git hash-object` applies attribute-selected clean
filters. Every tracked path must be a regular file whose executable bit matches
its mode, and every file on disk must be in the tree apart from a set named in
the preflight script itself rather than in an ignore file. `README.md` and every
`src/**` file are read from disk by `tests/public_api_boundary_test.rs`,
`tests/source_matrix_test.rs`, and `tests/no_clob_surface_test.rs`, and no named
set of audited paths mentioned them. Run the preflight after staging; anything
unstaged or unadded is a disagreement, which is the point.

The Rust audit retains the manifest, workflow, license, and schema checks for
review visibility. It also requires all six release-provenance sections, one
exact accepted-advisory section, and one parsed table under that H2. The table
is interpreted with `pulldown-cmark` and `ENABLE_TABLES`, matching the renderer
rather than scanning pipe-shaped lines. The parsed header and every ordered
six-cell row must exactly equal the canonical TOML register. HTML comments,
fenced code, prose, and other sections cannot contribute an accepted row.
Inline-code IDs are accepted because CommonMark renders their code text as the
cell value.

Positive vulnerability-overclaim phrases are rejected, while an explicit
statement that the record does not make such a claim remains allowed. This
test makes no network call and does not prove the resolved TLS backend count,
consumer feature unification, advisory reachability, or HTTPS connectivity.
