# TESTING.md

## Minimum Commands

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
pusd_adapter_approval_calldata_matches_fixture
pusd_adapter_merge_redeem_calldata_matches_fixture
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
  relayer/
    transaction_states.json
  operations/
    pusd_approval_calldata.json
    merge_redeem_calldata.json
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
CTF, decimals `18`, a non-reviewed spender, a non-reviewed operator, and any
non-empty adapter list. Source tests cover empty, oversized, and control-bearing
names and versions plus insecure, oversized, and control-bearing URLs. JSON
tests require checksum addresses and source metadata. The config types derive
`Serialize` only and must not expose `Deserialize`, environment loading, or file
loading.

`tests/public_api_boundary_test.rs` pins all four public types, every reviewed
constructor/getter/helper signature, the five crate-root exports, and the
canonical constructor function type. It also rejects `reqwest` and public async
functions anywhere in the calldata module. These are local config and source
audits only: PBRSDK-17 adds no HTTP test, fixture, calldata encoding, selector,
ABI, adapter route, or live request.

The required source hygiene checks remain match-zero gates:

```bash
grep -rn "pub use .*::\*" src/
grep -rn "dbg!\|println!" src/deposit_wallet/
grep -rn "allow(" src/deposit_wallet/http.rs src/deposit_wallet/http/ src/deposit_wallet/calldata/ --include=*.rs | grep -v "http/tests.rs"
```

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
