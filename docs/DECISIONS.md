# DECISIONS.md

## ADR-0001: This Fork Is Not The CLOB Client

Decision:

```text
Use official `polymarket_client_sdk_v2` for CLOB order/sign/cancel/balance behavior.
Use this fork only for deposit-wallet relayer operations that are not covered by an official Rust relayer SDK.
```

Consequences:

- no CLOB order client is added here;
- consumer apps must keep CLOB and relayer adapters separate;
- this crate must not shape trading strategy/domain models.

## ADR-0002: Upstream Safe/Proxy Code Is Reference, Not Deposit-Wallet Implementation

Decision:

```text
Keep upstream Safe/Proxy code compiling as a reference while adding deposit-wallet-specific modules separately.
```

Reason:

- upstream public examples and APIs are Safe/Proxy-oriented;
- deposit-wallet flow uses `WALLET-CREATE`, `WALLET`, fresh `/nonce?type=WALLET`, and DepositWallet EIP-712 Batch signing;
- Safe/Proxy execute assumptions are unsafe to reuse without wire-level proof.

## ADR-0003: Identity Separation Is Required

Decision:

```text
Relayer API key owner, deposit-wallet owner signer, and deposit-wallet contract/funder are separate identities in the public API and config.
```

Consequences:

- no code may assume `RELAYER_API_KEY_ADDRESS == DEPOSIT_WALLET_OWNER_ADDRESS`;
- tests must prove auth identity and owner identity can differ;
- logs must redact addresses where appropriate but preserve enough evidence for debugging.

## ADR-0004: Production Dependency Must Be Pinned

Decision:

```text
Consumer production builds must use a pinned git `rev` or vendored source.
```

Forbidden:

```toml
rs-builder-relayer-client = { git = "...", branch = "main" }
rs-builder-relayer-client = "0.1"
rs-builder-relayer-client = "*"
rs-builder-relayer-client = { git = "ssh://git@github.com/OrderBookTrade/rs-builder-relayer-client.git", rev = "..." }
```

## ADR-0005: crates.io Publishing Is Disabled

Decision:

```text
This internal fork is consumed by path dependency during development and pinned git rev in production. It is not published to crates.io.
```

Reason:

- the crate is high-risk venue-facing infrastructure;
- public crates.io releases can imply unsupported SDK status;
- audit, provenance, and pinned commit review matter more than public distribution.

## ADR-0006: Deposit-Wallet Submit Builders Must Be Fallible

Decision:

```text
New deposit-wallet WALLET submit request builders that accept caller-provided
signatures or call payloads must return Result and run signer, config, derived
wallet, signature-shape, and batch resource-limit validation before producing a
request body. The legacy infallible
`build_wallet_batch_request_with_signature` helper is removed from the public
crate and `deposit_wallet` re-export surface at the `0.2.0` migration boundary
because it cannot be made source-compatible, non-panicking, and validated with
its original return type. Unchecked WALLET serialization remains crate-internal
and is restricted to validated builders and fixture tests. The raw
`DepositWalletBatchRequest` DTO is not re-exported from the crate root, and its
submit-body fields are crate-private so consumers cannot construct a WALLET
submit body without a validated builder.
```

Reason:

- an infallible helper cannot report invalid signatures, unsupported configs,
  or oversized calldata without either panicking or returning an unchecked
  relayer body;
- unchecked WALLET submit bodies are unsafe for consumer HTTP/live paths because
  they can mix signer, nonce owner, submit `from`, deposit wallet, and config
  identities;
- compatibility callers should migrate to `try_build_wallet_batch_request_with_signature`
  or the validated signed-batch flow before wiring any live submit path.
- the compatibility exception is intentional:
  - who: this PR changed the deposit-wallet public API boundary;
  - what: the old safe infallible WALLET submit helper is removed from public
    exports and replaced by the fallible `try_` API;
  - when: 2026-05-22, recorded as the crate's `0.2.0` migration boundary;
  - why: the old return type could only fail by panicking, silently producing a
    poisoned request, or returning an unchecked relayer body;
  - how: consumers must migrate call sites to
    `try_build_wallet_batch_request_with_signature` and handle `RelayerError`
    before any live submit wiring.

Rollback:

- keep unchecked serializers crate-internal and restrict them to validated
  builders plus fixture serialization tests;
- if a consumer still calls `build_wallet_batch_request_with_signature`, migrate
  it to `try_build_wallet_batch_request_with_signature` and handle
  `RelayerError` at the relayer adapter boundary;
- if a consumer needs raw non-live serialization, add a deliberately named
  test-only or internal API with documented owner, risk, and removal condition
  in the consumer integration PR.

## ADR-0007: PBRSDK-4 Public API Boundary Audit

Decision:

```text
Freeze the reviewed 0.2.0 public integration surface at the crate root,
document the `deposit_wallet` module as a validated-builder boundary, and keep
CLOB order/sign/cancel/post behavior out of this relayer crate.
```

Consequences:

- crate-root deposit-wallet exports remain explicit; wildcard re-exports are
  not allowed;
- `build_wallet_batch_request_with_signature` must not return to crate-root or
  `deposit_wallet` public exports;
- `DepositWalletBatchRequest` is a validated output type, not a public
  construction API; submit-body fields stay crate-private;
- legacy Safe/Proxy APIs remain reference compatibility surface and are not the
  deposit-wallet `WALLET-CREATE` or `WALLET` path;
- CLOB order/sign/cancel/post behavior remains out of this crate and remains
  owned by the official Rust CLOB SDK adapter;
- `tests/public_api_boundary_test.rs` enforces the source, docs, and manifest
  boundary so future PRs fail before rustdoc or consumer imports drift.

Review evidence:

```bash
cargo test --test public_api_boundary_test
cargo doc --workspace --all-features --no-deps
grep -R "pub use .*::\\*\\|pub mod clob\\|pub use clob\\|build_wallet_batch_request_with_signature\\|DepositWalletBatchRequest" -n src tests docs README.md
```

Migration:

- consumers importing the removed infallible helper must move to
  `try_build_wallet_batch_request_with_signature` or the validated
  `SignedDepositWalletBatch` flow;
- consumer domain, strategy, risk, and actor crates must depend on their local
  port types rather than this crate's DTOs;
- CLOB trading integrations must stay in the official Rust CLOB SDK adapter.

Rollback:

- revert this documentation/test boundary only if a new ADR records the exact
  public API replacement, consumer migration path, and CLOB ownership impact;
- do not roll back by restoring unchecked raw submit construction or adding CLOB
  modules to this crate.

## ADR-0008: PBRSDK-6 Production Relayer Read Surface

Decision:

```text
Promote only GET /deployed, GET /nonce, and GET /transaction to production
methods on DepositWalletRelayerClient. Every method requires a
RelayerReadPermit bound to the requested owner and the client's configured
deposit-wallet chain.
```

The read permit stores only `owner` and `chain_id`. It intentionally has no
expiry because these calls are idempotent reads; mutation authorization,
evidence, and expiry belong to the separate PBRSDK-7 mutation capability.
Permit validation runs before transaction-id validation, URL construction, or
HTTP I/O. The relayer URL continues to require HTTPS, the allowlisted host, the
default HTTPS port, no userinfo, and no path, query, or fragment.

The former production-host hard block is removed because PBRSDK-2 recorded the
reviewed WALLET response provenance in
`tests/fixtures/deposit_wallet/PROVENANCE.md`,
`wallet_transaction_response.json`, and
`transaction_array_response_cases.json`. This authorizes the reviewed read
surface only; it does not authorize live mutation or duplicate-submit recovery.

`GET /deployed` follows the official TypeScript relayer SDK
`@polymarket/builder-relayer-client` `0.0.10` at commit
`9122f6fb1856f1ecfe4406685bfa19a2c5a7b290`: query `address` is the derived
deposit-wallet address, `type` is `WALLET`, and only an object containing a JSON
boolean `deployed` field is accepted. A `true` result is deployment fact, not
submit readiness; readiness still requires the separate `STATE_CONFIRMED`
policy.

Consequences:

- auth headers retain sensitive marking and response/error bodies remain
  bounded;
- transaction reads retain ID, WALLET type, owner/from, factory, derived-wallet,
  state, and hash evidence validation;
- `GET /transactions` remains deferred to PBRSDK-13 reconciliation work;
- no production `POST /submit`, mutation permit, WALLET-CREATE submit, or WALLET
  submit method is added by this decision;
- production-host happy paths are not called in CI because tests use only local
  loopback servers and synthetic credentials.
