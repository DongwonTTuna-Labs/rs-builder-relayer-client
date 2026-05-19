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
wallet_batch_submit_body_matches_fixture
transaction_state_unknown_blocks_live_retry
pusd_adapter_approval_calldata_matches_fixture
pusd_adapter_merge_redeem_calldata_matches_fixture
relayer_auth_address_not_used_as_owner_implicitly
ambiguous_submit_timeout_does_not_duplicate_submit
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
    wallet_nonce_request.json
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
- unknown transaction states force non-mutating behavior.

## Manual Live Gate

Live relayer checks are operator-gated only. CI must not require production relayer secrets.

Before any real relayer mutation, record evidence for:

```text
deposit wallet derive parity
WALLET-CREATE reaches MINED/CONFIRMED if deployment is in scope
fresh GET /nonce?type=WALLET before signing
WALLET approval batch updates deposit wallet allowance
CLOB balance allowance sync observes deposit wallet state in consumer app
POLY_1271 order path accepts maker/funder shape in consumer app
merge/redeem calldata follows current pUSD adapter path
ambiguous submit timeout does not duplicate transaction
```
