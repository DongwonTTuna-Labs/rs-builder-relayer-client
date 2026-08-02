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
| integration test | `calldata_batch_composition_test` | `tests/calldata_batch_composition_test.rs` | tracked | offline composition and request-preflight test | keep |
| integration test | `ci_contract_test` | `tests/ci_contract_test.rs` | tracked | offline repository-contract test | keep |
| integration test | `client_test` | `tests/client_test.rs` | tracked | offline test | keep |
| integration test | `deposit_wallet_signing_test` | `tests/deposit_wallet_signing_test.rs` | tracked | offline fixture test | keep |
| integration test | `deposit_wallet_test` | `tests/deposit_wallet_test.rs` | tracked | offline fixture test | keep |
| integration test | `integration_test` | `tests/integration_test.rs` | tracked | offline calldata test | keep |
| integration test | `live_gate_runbook_test` | `tests/live_gate_runbook_test.rs` | added by PBRSDK-26 | offline runbook and secret-shape audit; no host call | keep |
| integration test | `mutation_rollback_boundary_test` | `tests/mutation_rollback_boundary_test.rs` | added by PBRSDK-22a | pre-I/O rollback/read-capability boundary only | keep offline; no server or live call |
| integration test | `no_clob_surface_test` | `tests/no_clob_surface_test.rs` | added by PBRSDK-23a | offline source-marker audit; no host call | keep |
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

PBRSDK-13 adds no Cargo target, dependency, feature, example, or manifest
change. Production code adds `http/recent.rs`, one internal bounded
`GET /transactions` fetch, three intent reconciliation types, two report types,
two epoch-fenced registry methods, and two `IntentGatedClient` methods. The
existing low-level read permit count, two polling methods, two primitive
`submit_*` methods, and combined submit/execute/lifecycle counts remain
unchanged. The public boundary audit fixes all five additive types and the
`reconcile_manually`, `adopt_transaction`, `reconcile_by_polling`, and
`report_ambiguous_candidates` signatures at the HTTP, deposit-wallet, and
crate-root exports.

PBRSDK-13 tests remain unit tests in `src/deposit_wallet/http/tests.rs` and use
the existing loopback and injected-clock infrastructure. The only new fixture
is the sanitized, schema-constructed
`wallet_recent_transactions_response.json`; it is not a live-recorded response
or adoption verdict. Tests cover evidence validation and serde compatibility,
epoch ABA fencing, one CAS retry, manual adoption/reconciliation, authoritative
poll result mapping, pure-read reporting and filtering, response bounds, and
zero `POST /submit` across reconciliation polling. No live relayer, funded
wallet, credential, file/DB store, concurrency harness, or automatic resubmit is
introduced.

PBRSDK-15 adds no Cargo target, fixture, dependency, feature, example, manifest
entry, metrics backend, OTel stack, or logging collector. Production structured
events use the existing `tracing` dependency. The async capture regression uses
the already declared `tracing-subscriber` dev dependency with an in-memory
writer, ANSI/time disabled, and a thread-local default-dispatch guard held
across a default current-thread paused Tokio test.

Production code adds the schema-v1 `MutationIntentAuditArtifact`, redacted
`ReconciliationSummary`, `MutationIntentRecord.poll_attempts`, one pure-read
registry export method, and mutation lifecycle events. The public boundary
audit pins both new types, the exact schema constant, the export signature, and
explicit HTTP/deposit-wallet/crate-root re-exports without changing existing
primitive submit/execute/lifecycle counts. Unit tests cover legacy serde,
counter/no-op rules, write/export unknown-state defenses, malicious persisted
reconciliation text, JSON/Debug transaction-id separation, failure parity, and
the exact tracing field set against actual local submit material.

The PBRSDK-15 evidence is offline only. The in-memory writer is a test sink,
not a production logging system, and the artifact export does not qualify a
durable store or authorize live traffic.

PBRSDK-17 adds no Cargo target, dependency, feature, example, fixture, manifest
entry, or HTTP module change. Production code is limited to
`src/deposit_wallet/calldata/mod.rs` and `config.rs`, plus explicit module and
crate-root exports. Its tests are module-local config tests and the existing
public boundary target; there is no mock venue, network call, environment/file
loader, credential, or live host dependency.

The PBRSDK-17 boundary audit pins the four public types, all reviewed
constructors/getters/membership helpers, and the five crate-root exports. It
also pins the canonical constructor function type and requires a synchronous
module with no `reqwest`, public async function, or `Deserialize`. Canonical
Polygon configuration is source-backed and wire-truth-bound, strict subsets
remain strict, and ADR-0018 later narrows the adapter list to an empty-or-single
reviewed Polygon NegRiskAdapter subset without relaxing other bindings.

PBRSDK-18 adds no Cargo target, dependency, feature, example, manifest entry,
or HTTP module change. Production code adds only
`src/deposit_wallet/calldata/amount.rs` and `approval.rs`, their private module
declarations and explicit re-exports, and three additive crate-root exports.
Tests remain module-local plus the existing `public_api_boundary_test` and
`source_matrix_test` targets. No mock venue, network call, environment/file
loader, credential, funded wallet, live host, or bespoke test harness is added.

The two new flat JSON fixtures are
`calldata_pusd_approval_call.json` and
`calldata_ctf_approval_for_all_call.json` under the existing non-recursive
deposit-wallet fixture directory. Their provenance is recorded in the existing
ledger. Tests pin the recorded pUSD MAX call, supplied argument propagation,
finite amount preservation, both golden fixtures, outsider/zero/subset
allowlist failures, CTF revocation, unit conversion/serialization, and the
builder's defense-in-depth zero guard.

The PBRSDK-18 boundary audit pins `PusdAmount`, both builder signatures, their
private module/explicit re-export shape, the exact private base-unit field, and
all three crate-root exports. The combined calldata source remains synchronous,
contains no runtime deserialization or HTTP path, and cannot reference legacy
`crate::operations` or `crate::contracts`. The source-matrix target now requires
the three PBRSDK-17 calldata truth rows plus
`SM-CALLDATA-APPROVAL-ENCODING`.

PBRSDK-19 adds no Cargo target, dependency, feature, example, manifest entry,
HTTP module, live host call, mock venue, or bespoke harness. Production code
adds only private `src/deposit_wallet/calldata/position.rs` and `ctf.rs`, their
explicit module exports, six additive crate-root symbols, and the reviewed
NegRiskAdapter config constant/subset. Existing amount, approval, HTTP,
signing, request, legacy operation, and contract files remain unchanged.

The four new flat JSON fixtures pin ConditionalTokens split, merge, and
full-balance redeem plus NegRisk redeem. Module-local tests cover fixture bytes,
alternate-input ABI offsets for all builders, array and amount boundaries,
route-selector-target closure, canonical and narrowed adapter policy, unit
serialization, duplicate NegRisk quantities, and defense-in-depth zero/
unlimited guards. Existing `public_api_boundary_test` and
`source_matrix_test` targets expand to the two new private modules, six root
exports, exact position field shape, four fixture provenance rows, and
`SM-CALLDATA-CTF-ROUTES`; no new integration target is created.

PBRSDK-19 is offline call-construction evidence only. It does not compose a
batch, calculate positions, prepare conditions, fetch a nonce, sign, submit,
touch CLOB state, or authorize live execution. Both observed legacy route
drifts remain unchanged but are documented as forbidden deposit-wallet paths;
any further route drift blocks the live gate.

PBRSDK-20 adds exactly one Cargo integration-test target,
`calldata_batch_composition_test`, from
`tests/calldata_batch_composition_test.rs`. It adds no binary, example,
benchmark, dependency, feature, manifest entry, HTTP target, live host call,
mock venue, or bespoke harness. Production code adds only the private pure
`src/deposit_wallet/calldata/summary.rs` module, its explicit module export, and
the three additive crate-root symbols `summarize_batch_calls`,
`BatchCallSummary`, and `DepositWalletBatchSummary`.

The new integration target composes the existing six fixture-backed builders,
audits redacted ordered summaries, reaches both private resource limits through
the public request builder, checks wallet/config/chain/signer and narrowed-
config failures, and proves the canonical approval call is compatible with the
serialized WALLET request body. It reuses existing fixtures and dependencies;
no fixture, provenance, or source-matrix file changes. The existing
`public_api_boundary_test` target expands to the private summary module, exact
private field blocks, getter/function signatures, three root exports, and the
combined pure-calldata source audit.

PBRSDK-20 is offline composition and serialization-preflight evidence only.
The summary does not validate calls and deliberately contains no full
calldata, signature, auth material, full target, or calldata hash. Deadline
freshness remains in the existing clock-injected execution/submit tests. This
target adds no nonce fetch, signing operation, submit, CLOB synchronization,
actor orchestration, or live-execution authority.

PBRSDK-22a adds exactly one Cargo integration-test target,
`mutation_rollback_boundary_test`, from
`tests/mutation_rollback_boundary_test.rs`. It adds no binary, example,
benchmark, dependency, feature, manifest entry, fixture, HTTP implementation,
live credential, mock venue, public test transport, or bespoke harness.
Production code adds only the private
`src/deposit_wallet/identity.rs` config boundary, its six explicit re-exports,
and the additive `RelayerKeyAuth::from_identity` constructor.

The new target starts no server and makes no timing assertion. Its production
URL value is constructed but every awaited operation is rejected before
network dispatch: the disabled mutation latch rejects a valid Live
WALLET-CREATE permit, and the same disabled client still applies owner- and
chain-mismatched read-permit checks. Successful read round trips remain in the
existing internal loopback tests and are not duplicated by widening the public
test API.

Six module-local identity tests cover zero-check ordering, owner-derived wallet
validation, additive request-context construction, overlap ordering/keys, and
redacted Debug/summary serialization. The existing
`public_api_boundary_test` target expands to pin the six crate-root exports,
private identity module/fields, complete-tree no-`From` rule, prohibited
identity coercion/display/deserialization traits, additive auth adapter, and
absence of public `set_mutation*` reactivation paths. `SM-IDENTITY-SEPARATION`
records the local design authority and unchanged live block.

PBRSDK-22a is offline fork-boundary evidence only. It changes no wire format,
existing public signature, HTTP behavior, CLOB/POLY_1271 behavior, or live
authority. Exclusive consumer adapter imports, fork-DTO-free consumer
port/domain types, adapter tests, and CLOB funder wiring belong to PBRSDK-22b.

PBRSDK-23a adds one auto-discovered integration-test target,
`no_clob_surface_test`, without a `Cargo.toml` target declaration. It adds no
dependency, feature, fixture, binary, example, benchmark, production source,
host call, mock venue, credential, or live authority. The target recursively
reads the existing `src/**/*.rs` corpus in memory and uses only the standard
library.

Its Proof Scope is deliberately limited to the absence of the specified CLOB-
specific markers and function signatures in `src/`, except for three
path-and-occurrence-count-pinned compatibility/provenance markers. It does not
prove all CLOB code absent and does not cover renamed implementations, macro or
build-script output, external-crate behavior, consumer funder/POLY_1271 wiring,
or confirmed-only balance synchronization. Synthetic mutations validate the
same audit function without adding or changing a fixture.

PBRSDK-26 adds one auto-discovered integration-test target,
`live_gate_runbook_test`, from `tests/live_gate_runbook_test.rs`, without a
`Cargo.toml` declaration. It uses only the standard library, reads the new
manual runbook and the existing deposit-wallet design, and makes no host or
live call. Its pure audit requires the ten runbook sections, seven stop
conditions, six exact approval gates, and the design backlink, then checks the
runbook for the specified secret-shaped values. Exact synthetic mutations pin
one-change/one-violation behavior and longest-threshold hex classification.

This PBRSDK-26 target adds no dependency, fixture, binary, example, benchmark,
production source, credential, submit path, or live authority. Its shape audit
cannot detect unprefixed 40-hex values, UUID-shaped keys, or base64 values and
does not prove that all secrets are absent. The runbook is an operator
procedure for PBRSDK-27, not live validation evidence.

These audits are offline only. They do not authorize live relayer mutation, CLOB
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
- match-zero output for wildcard re-exports, deposit-wallet debug printing,
  and production/calldata `allow(` attributes,
- PBRSDK-17 source-matrix rows and calldata config boundary-test output,
- PBRSDK-18 approval fixture, unit/allowlist, private-field, and
  `SM-CALLDATA-APPROVAL-ENCODING` evidence,
- PBRSDK-19 four-route fixture/ABI-offset/negative tests, position-field and
  route public-boundary output, and `SM-CALLDATA-CTF-ROUTES` drift evidence,
- PBRSDK-20 six-builder composition, summary redaction/boundary/empty-input,
  request-preflight negative/success, and summary public-boundary output,
- PBRSDK-22a six identity unit tests, two pre-I/O rollback/read-capability
  integration tests, six-symbol/private-field/no-coercion public-boundary
  output, and `SM-IDENTITY-SEPARATION`,
- PBRSDK-23a recursive `src/` marker/function audit output, frozen conditional-
  exception counts and `/orders` test-region assertion, plus each exact
  synthetic mutation result and the longer-identifier positive boundary,
- confirmation that logs and artifacts contain no secrets, auth headers, raw
  production signatures, private endpoints, funded-wallet data, or replayable
  production submit bodies,
- a sample schema-v1 redacted mutation artifact and captured
  `polymarket_relayer::mutation_intent` event evidence with only the reviewed
  fields,
- confirmation that PBRSDK-2 source matrix and fixture provenance files remain
  present and live behavior remains gated,
- confirmation that live intent wiring uses a durable transactional/CAS store,
  never `InMemoryMutationIntentStore`, and that ambiguous/unknown state has no
  automatic resubmit or implicit lease release.
