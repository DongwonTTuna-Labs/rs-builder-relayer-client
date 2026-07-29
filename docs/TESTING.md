# TESTING.md

## Minimum Commands

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
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
pusd_adapter_approval_calldata_matches_fixture
pusd_adapter_merge_redeem_calldata_matches_fixture
relayer_auth_address_not_used_as_owner_implicitly
ambiguous_submit_timeout_does_not_duplicate_submit
idless_submit_timeout_blocks_owner_until_manual_reconcile
```

## Fixture Rules

- Fixtures must be sanitized.
- Fixtures must not contain production private keys, API credentials, auth headers, or raw production signatures.
- Fixture names should include endpoint and scenario.
- If a fixture comes from official TypeScript/Python SDK behavior, record the SDK version or commit.

Recommended fixture layout:

```text
tests/fixtures/
  deposit_wallet/
    derive_address.json
    wallet_create_submit_body.json
    wallet_create_transaction_response.json
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
and one-way rollback; they do not replace the later polling, persistent
idempotency, reconciliation, or duplicate-submit recovery gate.

For PBRSDK-8, retain the submitted WALLET-CREATE transaction id and payload
hash, then record `STATE_CONFIRMED` through the expected-type readiness path.
Pending or error results forbid lifecycle re-entry for that owner until the
deferred polling and owner-scoped intent controls reconcile the original
submission.

`STATE_MINED` may be recorded as pending evidence, but it must not satisfy the
manual live gate. Wallet deployment or wallet-action effects become usable only
after `STATE_CONFIRMED`.
