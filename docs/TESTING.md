# TESTING.md

## Minimum Commands

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
GIT_MASTER=1 git diff --check
```

## Required Deposit-Wallet Tests

These tests are required before any production import or live relayer mutation.

```text
derive_deposit_wallet_address_matches_reference
wallet_create_submit_body_matches_fixture
wallet_nonce_uses_type_wallet
wallet_batch_signature_matches_reference_digest
wallet_batch_submit_body_matches_fixture
transaction_state_unknown_blocks_live_retry
pusd_adapter_approval_calldata_matches_fixture
pusd_adapter_merge_redeem_calldata_matches_fixture
relayer_auth_address_not_used_as_owner_implicitly
ambiguous_submit_timeout_does_not_duplicate_submit
idless_submit_timeout_blocks_owner_until_manual_reconcile
```

Task 14 adds the live-capable mock gate suite without requiring real secrets or a live network:

```text
live_mock_relayer_flow_reads_nonce_submits_without_hash_and_polls_to_confirmed
live_mock_dry_run_prepares_signed_batch_without_submit_requests
live_mock_failure_modes_cover_terminal_absent_timeout_and_429
live_mock_redacts_auth_signature_signed_body_and_asserts_identity_separation
deposit_wallet_live_example_fails_closed_before_dotenv_secret_loading
deposit_wallet_live_example_refuses_off_amoy_before_dotenv_secret_loading
```

The mock relayer suite intentionally reuses the in-repo raw TCP helpers in
`src/deposit_wallet/http/tests.rs` instead of adding `wiremock` or `httpmock`.
Those helpers already provide ordered responses, request capture, auth-header
inspection, redirect/reset simulation, timeout coverage, and zero-submit
assertions. No new test dependency is needed for Task 14.

## Fixture Rules

- Fixtures must be sanitized.
- Fixtures must not contain production private keys, API credentials, auth headers, or raw production signatures.
- Fixture names should include endpoint and scenario.
- If a fixture comes from official TypeScript/Python SDK behavior, record the SDK version or commit.
- Captured request debug/log output must redact relayer auth headers, raw signed bodies, and owner signatures even when tests inspect the raw captured request fields directly.

Recommended fixture layout:

```text
tests/fixtures/
  deposit_wallet/
    derive_address.json
    wallet_create_submit_body.json
    wallet_nonce_request.json
    wallet_batch_eip712.json
    wallet_submit_body.json
    wallet_signed_submit_body_amoy.json
  relayer/
    transaction_states.json
  operations/
    erc20_approve_calldata.json
    ctf_split_position_calldata.json
    ctf_merge_positions_calldata.json
    ctf_redeem_positions_calldata.json
```

## Golden And Mock Test Scope

Golden and mock tests must prove:

- `WALLET-CREATE` request shape is not Safe deploy shape;
- `WALLET` request shape is not Safe/Proxy execute shape;
- `/nonce?address=<owner>&type=WALLET` is used immediately before signing;
- EIP-712 domain, message, digest, and signature shape match reference behavior;
- relayer auth identity can differ from wallet owner signer, deposit wallet, and funder;
- dry-run preparation issues zero `POST /submit` requests;
- submit responses without `transactionHash` still poll by `transactionID`;
- success is accepted only at `STATE_CONFIRMED`;
- `STATE_FAILED`, `STATE_INVALID`, unknown states, empty arrays, missing requested ids, timeouts, and HTTP 429 are non-success;
- off-Amoy chain/URL and missing live gate fail before any live submit.

## CI No-Secret Policy

Default CI and local `cargo test --workspace --all-features` must not require or
read real relayer secrets, owner private keys, auth headers, or live network
connectivity. CI may run sanitized mock/golden tests only. If a test needs a
private key, it must use a documented public deterministic test key and must not
print or commit that key as production material.

## Operator-Gated Amoy Live Smoke

Live relayer checks are operator-gated only. CI must not require production relayer secrets.

Required environment checklist for the operator shell:

```text
POLYMARKET_RELAYER_ALLOW_LIVE_AMOY=1
POLYMARKET_OWNER_PRIVATE_KEY=<operator Amoy owner private key>
POLYMARKET_OWNER_ADDRESS=<optional derived owner cross-check>
POLYMARKET_RELAYER_API_KEY=<relayer API key>
POLYMARKET_RELAYER_API_KEY_ADDRESS=<relayer API key owner address>
POLYMARKET_AMOY_RELAYER_URL=https://relayer-v2-staging.polymarket.dev/
POLYMARKET_AMOY_APPROVE_TOKEN=<operator-selected Amoy ERC20 token>
POLYMARKET_AMOY_APPROVE_SPENDER=<operator-selected Amoy spender>
POLYMARKET_AMOY_APPROVE_AMOUNT=<optional decimal uint256; defaults to max>
```

Exact operator command for Task 15, after Task 14 validation is green and the
operator has explicitly approved the smoke:

```bash
POLYMARKET_RELAYER_ALLOW_LIVE_AMOY=1 cargo run --example deposit_wallet_live -- --network amoy --execute
```

Before any real relayer mutation, record sanitized evidence for:

```text
deposit wallet derive parity
WALLET-CREATE reaches STATE_CONFIRMED if deployment is in scope
fresh GET /nonce?type=WALLET before signing
WALLET approval batch reaches STATE_CONFIRMED
transaction ids, states, and optional transaction hashes only after redaction review
ambiguous submit timeout does not duplicate transaction
```

`STATE_MINED` and `STATE_EXECUTED` may be recorded as pending evidence, but they
must not satisfy the manual live gate. Wallet deployment or wallet-action effects
become usable only after `STATE_CONFIRMED`.
