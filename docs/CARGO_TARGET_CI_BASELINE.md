# Cargo Target And CI Baseline

Retrieval date: 2026-07-04

This audit records the PBRSDK-3 target graph and PR validation gate. It is an
offline validation baseline only. It does not authorize live relayer mutation,
wallet deployment, production signing, order placement, trading behavior,
production credentials, private endpoints, funded-wallet data, or replayable
submit bodies.

## Cargo Target Audit

| Target kind | Name | Declared path | Tracked file status | Runtime/live risk | Resolution |
|---|---|---|---|---|---|
| lib | `polymarket_relayer` | `src/lib.rs` | tracked | library only | keep |
| example | `setup_wallet` | `examples/setup_wallet.rs` | tracked | none in normal run; offline-safe body verified | keep offline approval-plan example |
| example | `redeem_single` | `examples/redeem_single.rs` | tracked | none in normal run; offline-safe body verified | keep offline calldata example |
| example | `redeem_all` | `examples/redeem_all.rs` | tracked | none in normal run; offline-safe body verified | keep synthetic-position dry-run |
| example | `split_merge` | `examples/split_merge.rs` | tracked | none in normal run; offline-safe body verified | keep offline calldata example |
| example | `redeem_magic` | `examples/redeem_magic.rs` | tracked | none in normal run; offline-safe body verified | keep offline proxy-mode dry-run |
| example | `diagnose_gs026` | `examples/diagnose_gs026.rs` | tracked | none in normal run; offline-safe body verified | keep fixture diagnostic |
| example | `diagnose_nonce` | `examples/diagnose_nonce.rs` | tracked | none in normal run; offline-safe body verified | keep fixture hash diagnostic |
| integration test | `auth_test` | `tests/auth_test.rs` | tracked | offline test | keep |
| integration test | `builder_test` | `tests/builder_test.rs` | tracked | offline test | keep |
| integration test | `client_test` | `tests/client_test.rs` | tracked | offline test | keep |
| integration test | `deposit_wallet_signing_test` | `tests/deposit_wallet_signing_test.rs` | tracked | offline fixture test | keep |
| integration test | `deposit_wallet_test` | `tests/deposit_wallet_test.rs` | tracked | offline fixture test | keep |
| integration test | `integration_test` | `tests/integration_test.rs` | tracked | offline calldata test | keep |
| integration test | `operations_test` | `tests/operations_test.rs` | tracked | offline calldata test | keep |
| integration test | `source_matrix_test` | `tests/source_matrix_test.rs` | tracked | offline provenance test | keep |
| integration test | `public_api_boundary_test` | `tests/public_api_boundary_test.rs` | tracked | offline public API and docs boundary audit | keep |

No stale declared Cargo target was found in the current baseline. If a future
target is added, it must map to a tracked file or document a restore, removal,
or feature-gate decision before the task can be considered complete.

The previous live-oriented example bodies were remediated in place. The current
Cargo examples use synthetic condition IDs, deterministic fixture addresses, and
local calldata builders only. They do not read `.env`, environment credentials,
private keys, relayer credentials, RPC URLs, or live wallet addresses; they do
not instantiate relayer clients, RPC providers, data clients, direct executors,
or wallets; and they do not call deploy, approval setup, execute, wait,
position-fetch, or fallback paths. `tests/ci_contract_test.rs` enforces this
offline-safe example contract.

## CI Gate

`.github/workflows/rust-validation.yml` runs on pull request open, ready for
review, synchronize, and reopen events. It has read-only repository permission
and does not reference GitHub secrets.

The workflow enforces:

- `cargo fmt --all --check`
- `git diff --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --all-features`
- `cargo build --workspace --all-targets --all-features`

The existing Grimoire workflow remains separate. Rust validation does not
depend on Grimoire secrets, live relayer credentials, private endpoints,
production wallets, signing keys, or trading credentials.

## PBRSDK-4 Public API Boundary Audit

`tests/public_api_boundary_test.rs` records the PBRSDK-4 public boundary gate.
It keeps crate-root and `deposit_wallet` exports explicit, prevents the removed
infallible WALLET batch helper from returning as public API, keeps raw WALLET
submit DTO fields crate-private, blocks CLOB order/sign/cancel/post SDK modules
or examples in this relayer crate, and requires semver/migration/grep audit
evidence in the docs.

PBRSDK-6 expands that audited boundary with `RelayerReadPermit` and exactly
three production read methods; the boundary test now records those additions
and, at that boundary, continued to reject wildcard exports and a production
submit surface.

PBRSDK-7 expands the surface audit with the explicit mutation permit/evidence/
outcome types, the default-deny and one-way client gate, and exactly two
permit-bound submit methods. This is a public-surface audit only; it does not
claim transaction polling, idempotent recovery, or end-to-end live readiness.

PBRSDK-8 adds three explicit deployment-lifecycle types and two lifecycle
methods while auditing three public reads plus one permit-bound internal
expected-type helper; the combined production surface still contains exactly
two public `submit_*` methods. Its fixture and loopback audit proves only
single-shot confirmed readiness, not polling, persistence, duplicate-submit
recovery, or live enablement.

PBRSDK-9 adds exactly one inherent public `execute_wallet_batch` method with a
generic owner signer and explicit read/mutation permits, plus the required
additive `Eip712` trait implementation on the existing batch type. The
boundary audit fixes the execute signature, keeps the signer out of client
state, and confirms the combined
production surface still exposes exactly two public `submit_*` methods and the
same read-permit count. Three canonical fixtures prove ethers EIP-712 digest
parity; injected-clock loopback tests prove nonce-before-sign-before-submit,
identity separation, pre-I/O rejection, DryRun/latch behavior, submit failure
classification, and signer-error redaction. The audit uses only a documented
synthetic throwaway signer and does not authorize live execution or concurrent
same-owner batches before the deferred lease contract.

PBRSDK-10 adds `RelayerPollPolicy`, `RelayerPollOutcome`, and exactly two
inherent `poll_*` methods. Both require the existing owner- and chain-scoped
read permit and delegate to the same crate-internal expected-type transaction
helper. The low-level read audit remains three public methods plus one internal
permit occurrence, while the complete production source still exposes exactly
two public `submit_*` methods. No submit, signer, nonce, recent-transactions, or
automatic reconciliation surface is added.

The polling tests are unit tests in the existing HTTP test module, so no Cargo
target is added. The dev-dependency Tokio feature set adds only `test-util` for
`#[tokio::test(start_paused = true)]`; the production Tokio dependency remains
unchanged. A timeout-free polling-only reqwest client and plain loopback accept/
read futures keep I/O sections free of virtual timers. Tests prove exact
attempts and interval sums, backoff caps, confirmed-only success, terminal and
unknown-state handling, bounded 429/5xx and absence retries, cancellation,
type isolation, and pre-HTTP permit rejection. The server intentionally has no
internal accept timeout, so the CI or command runner remains the hang guard.
This audit does not authorize live mutation or treat exhaustion/cancellation as
resubmit authority.

PBRSDK-12 adds no Cargo target or dependency. It adds the public synchronous
`MutationIntentStore` boundary, `TryBeginOutcome`, the test/development-only
`InMemoryMutationIntentStore`, record/status, registry/lease, and
`IntentGatedClient` in the existing HTTP module. Existing `submit.rs`,
`execute.rs`, and `lifecycle.rs` method counts remain two, one, and two. The
combined production-source audit now expects three public `submit_*`
signatures (two permit-bound primitives plus one intent-gated wrapper), two
`execute_wallet_batch` signatures, and two
`ensure_deposit_wallet_deployment` signatures; every wrapper retains the
reviewed permit arguments.

PBRSDK-12 tests remain unit tests in `src/deposit_wallet/http/tests.rs`. They
reuse the existing loopback transport and injected clocks and add no fixture,
example, integration target, live host call, file store, DB harness, or
credential. The boundary audit fixes the eight public types, synchronous
atomic begin/versioned update methods, transaction-bound poll/failure
signatures, explicit re-exports, and unchanged primitive method counts. Local
evidence additionally requires warning-free rustdoc with
`RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps`.
The in-memory restart test reconstructs a registry over the same process-local
store; it does not qualify persistence. Live use remains blocked until a
consumer durable store passes the PBRSDK-24/25 restart gate.

This audit is offline only. It does not authorize live relayer mutation, CLOB
trading, wallet deployment, order placement, production credentials, private
endpoints, funded-wallet data, or replayable submit bodies.

## Evidence Requirements

Implementation and review packets should include:

- this target audit,
- `tests/public_api_boundary_test.rs` output,
- final local command output or equivalent CI evidence,
- PR workflow diff evidence,
- `git diff --check` output,
- `cargo doc --workspace --all-features --no-deps` output,
- CLOB absence and public re-export grep/import audit output,
- confirmation that logs and artifacts contain no secrets, auth headers, raw
  production signatures, private endpoints, funded-wallet data, or replayable
  production submit bodies,
- confirmation that PBRSDK-2 source matrix and fixture provenance files remain
  present and live behavior remains gated,
- confirmation that live intent wiring uses a durable transactional/CAS store,
  never `InMemoryMutationIntentStore`, and that ambiguous/unknown state has no
  automatic resubmit or implicit lease release.
