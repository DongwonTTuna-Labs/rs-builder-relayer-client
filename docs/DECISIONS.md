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
  `deposit_wallet::SignedDepositWalletBatch` flow;
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
- this PBRSDK-6 decision defers `GET /transactions`; ADR-0014 later adds the
  permit-first report-only PBRSDK-13 path;
- no production `POST /submit`, mutation permit, WALLET-CREATE submit, or WALLET
  submit method is added by this decision;
- production-host happy paths are not called in CI because tests use only local
  loopback servers and synthetic credentials.

## ADR-0009: PBRSDK-7 Explicit Mutation Permit and Dry-Run Gate

Decision:

```text
Expose WALLET-CREATE and signed WALLET submit only behind an explicit,
single-operation RelayerMutationPermit and a separate default-deny client
mutation latch. DryRun produces review evidence without HTTP; Live requires
both a matching, unexpired Live permit and an explicitly enabled client.
```

`RelayerMutationPermit::try_new` binds the permit to a mode (`DryRun` or
`Live`), operation (`WalletCreate` or `WalletBatch`), owner, chain id, Unix
expiry, evidence reference, and operator-approval reference. Expiry must be
non-zero. Both references are trimmed and must be non-empty, at most 256 bytes,
and free of control characters. At use time, operation, owner, chain, and
expiry are checked before HTTP; `now >= expires_at_unix` is expired. A signed
batch additionally fails before HTTP when `now >= deadline`. A permit for one
mode, operation, owner, or chain is not authority for another. For a `DryRun`,
the operator-approval reference may identify the pending review record; a
fresh `Live` permit must reference the completed approval record.

`DepositWalletRelayerClient::new` is default-deny for live mutation.
`new_with_mutation_enabled` creates an explicitly live-enabled client, but does
not replace the required `Live` permit. The gate is an `Arc<AtomicBool>` shared
by every clone. `disable_mutation` changes it one way to disabled; there is no
reactivation method for that client or any clone. This is the operator rollback
mechanism. Read-permit-bound reads and valid `DryRun` submissions remain
available after rollback.

`DryRun` intentionally does not consult the live mutation latch. This permits
the default-deny client to create review evidence before approval and permits
the same non-mutating inspection after rollback. It is not a bypass: permit
scope, expiry, request validation, and batch deadline checks still run, and no
HTTP request is sent. Evidence includes the operation, endpoint path, scoped
identities, payload hash, nonce/deadline and bounded call summaries as
applicable, while omitting auth headers, signatures, full calldata, and the
full replayable submit body.

After a live request may have been dispatched, an invalid or partial success
response, malformed or missing transaction id/state evidence, transport
failure, or oversized 2xx response is classified as reconciliation-required.
The caller must persist the available request-intent and error evidence and
must not resubmit on that result. A valid submit receipt is acceptance evidence
only; an unknown returned state is preserved as non-success.

Consequences:

- the reviewed public mutation surface consists of the scoped permit/evidence/
  outcome types, `new_with_mutation_enabled`, `disable_mutation`,
  `submit_wallet_create`, and `submit_signed_wallet_batch`;
- operator workflow is `DryRun` evidence, review, then a freshly created scoped
  `Live` permit used with an explicitly enabled client;
- transaction polling, recent-transaction lookup, persistent intent and
  idempotency records, duplicate-submit recovery, and complete live readiness
  remain deferred to later reconciliation work;
- this decision does not relax the `STATE_CONFIRMED`, fixture, consumer, or
  operator enablement gates.

## ADR-0010: PBRSDK-8 Deposit-Wallet Deployment Lifecycle

Decision:

```text
Compose the approved deployed read and WALLET-CREATE submit paths into an
explicit-policy deployment lifecycle, then perform at most one typed
WALLET-CREATE transaction read. Only STATE_CONFIRMED produces Ready.
```

`DepositWalletDeploymentPolicy` is supplied on every lifecycle call and does
not implement `Default`. Consumers normally choose `Predeployed`, which keeps
WALLET-CREATE disabled and returns a mutation-blocked error if the deployed
read is false. A runtime that explicitly owns provisioning may choose
`DeployIfMissing`, but a missing wallet still cannot enter submission without
an explicit `RelayerMutationPermit`.

The lifecycle always calls `is_deposit_wallet_deployed` first. That existing
path validates the owner- and chain-scoped read permit, derives the deposit
wallet, validates the configured contract source, and performs the deployed
read. A `true` result short-circuits without consulting mutation authority. On
a false result, `submit_wallet_create` remains the sole owner of mutation
operation, owner, chain, expiry, one-way latch, mode, factory, and request-body
validation. The lifecycle does not duplicate or weaken either validation
path. In short, the deployed read already performs derivation and contract
configuration validation, while `submit_wallet_create` already performs permit
and configuration validation.

The official TypeScript SDK at commit
`9122f6fb1856f1ecfe4406685bfa19a2c5a7b290` defines transaction types
`WALLET` and `WALLET-CREATE` separately. Transaction wire validation therefore
accepts an internal expected type while preserving all existing id, owner,
from, configured-factory `to`, derived `proxyAddress`, state, and hash checks.
The public `get_transaction_for_owner` continues to require `WALLET`;
deployment readiness alone uses `WALLET-CREATE`. Neither type is accepted as
the other.

`check_deposit_wallet_deployment_readiness` performs one read only. Confirmed
with the required hash evidence maps to `Ready`; New, Executed, and Mined map
to `Pending`. Invalid and Failed retain their typed errors, while Unknown,
wrong-type, malformed, absent, or otherwise ambiguous evidence remains an
error requiring reconciliation. There is no polling loop, retry, sleep,
backoff, cancellation, or resubmission in this round; bounded polling belongs
to PBRSDK-10.

The no-redeployment contract is expressed through outcomes and errors, not a
new persistence layer. `CreateSubmitted` preserves `transaction_id` and
`payload_keccak256`; consumers must persist both and reconcile until readiness
is confirmed. They must not call the deployment entry again for an owner with
a pending create. Owner-scoped intent state and code-enforced duplicate-submit
prevention belong to PBRSDK-11, with later reconciliation work remaining in
PBRSDK-12/13.

Consequences:

- the public lifecycle surface adds
  `DepositWalletDeploymentPolicy`, `DepositWalletDeploymentStatus`,
  `DepositWalletReadiness`, `ensure_deposit_wallet_deployment`, and
  `check_deposit_wallet_deployment_readiness`;
- the public submit surface remains exactly the two PBRSDK-7 permit-bound
  `submit_*` methods; lifecycle orchestration does not add another submit API;
- dry-run returns redacted create evidence and sends no submit HTTP request;
- deterministic loopback tests prove deployed short-circuiting, default
  predeployed denial, permit and latch propagation, exact signature-free
  WALLET-CREATE body, confirmed-only readiness, typed failures, type isolation,
  and read-permit rejection before HTTP;
- `wallet_create_transaction_response.json` is schema-constructed from the
  recorded WALLET response shape plus the official type enum. No live-recorded
  WALLET-CREATE response is available yet, so live shape parity remains an
  explicit residual risk and this decision does not authorize live deployment.

## ADR-0011: PBRSDK-9 Fresh-Nonce WALLET Batch Execution

Decision:

```text
Keep the owner signer outside DepositWalletRelayerClient. Accept it as a
generic execute_wallet_batch method argument, validate both scoped permits and
all batch identities before I/O, fetch the WALLET nonce immediately before
signing, then reuse the validated signed-request and permit-gated submit paths.
```

The relayer API key identity, owner signer, and deposit wallet/funder remain
separate API values. The client stores only relayer auth and contract config;
it does not store a signer or private key. `execute_wallet_batch` requires
`S: ethers::signers::Signer`, checks `signer.address()` against the requested
owner, and discards the signer's original error, Debug text, and source chain
on failure. This supports local, KMS, HSM, and other signer implementations
without allowing backend diagnostics to leak credential or intermediate
signature material through `RelayerError`.

`DepositWalletBatchToSign` implements ethers 2.0.14 `Eip712`. Its domain is
the existing DepositWallet name/version, configured chain id, and deposit
wallet verifying contract. Its struct hash reuses the same extracted private
Batch-hash helper as the canonical fixture-backed digest; it does not duplicate
the type hash or calls hash. `struct_hash` intentionally does not apply resource
limits because the execution path validates those limits before signing.
Parity tests require ethers `encode_eip712`, the existing canonical digest,
and each expected digest in `wallet_batch_eip712.json`,
`wallet_batch_eip712_amoy.json`, and
`wallet_batch_eip712_multicall.json` to be byte-identical. Existing digest
behavior and fixtures remain the source of truth.

Execution order is fixed: validate the WALLET-batch mutation permit and read
permit; reuse the returned chain and clock; reject an expired deadline; verify
signer identity, resource limits, and the owner/config-derived wallet; fetch
the fresh WALLET nonce; sign without another HTTP await between nonce and
signing; validate signature recovery, identities, chain, wallet, and limits
again through `try_build_wallet_batch_request_with_signature`; then call
`submit_signed_wallet_batch`. The submit path retains DryRun versus Live,
one-way latch, deadline, payload-hash, transaction-id, and ambiguous-result
classification behavior. DryRun intentionally performs the nonce read and
local signature so its redacted evidence contains the actual fetched nonce.

Freshness is constructive only within this single execution. The method does
not add polling, retry, sleep, nonce persistence, or an owner-scoped lease.
Consumers must forbid concurrent execution for the same owner. The
owner-scoped lease and intent contract remains PBRSDK-11/12 work; any
post-dispatch uncertainty still requires reconciliation and never authorizes a
duplicate submit.

Consequences:

- the only new inherent public method is `execute_wallet_batch`; the required
  public `Eip712` trait implementation is additive, while no new public type,
  re-export, or `submit_*` method is introduced;
- combined production source still exposes exactly two permit-bound public
  `submit_*` methods and the three low-level reads remain unchanged;
- deterministic loopback tests prove nonce-before-sign-before-submit ordering,
  auth/owner separation, exact rebuilt body, DryRun nonce evidence, pre-I/O
  rejection, latch behavior, 5xx/transport propagation, no duplicate POST, and
  signer-error redaction;
- no live host, production credential, real private key, polling, retry, or
  lease is introduced, so this decision alone does not authorize complete live
  operation.

## ADR-0012: PBRSDK-10 Bounded Confirmed-Only Transaction Polling

Decision:

```text
Add two read-permit-bound polling methods over the existing verified
GET /transaction path. Use a finite attempt policy and fixed doubling
backoff. Only STATE_CONFIRMED is success; every other outcome preserves the
original transaction identity and never authorizes signing or submission.
```

`RelayerPollPolicy::try_new` requires `max_attempts` in `1..=100`, an initial
interval of at least one millisecond and no greater than the maximum interval,
and a maximum interval no greater than 600 seconds. The delay before the next
attempt doubles from the initial interval and saturates at the configured
maximum. There is no caller-selected multiplier, jitter, default policy, or
injected sleeper. Production uses `tokio::time::sleep`; paused Tokio time makes
the same code deterministic in tests.

`poll_wallet_transaction` requires `WALLET` evidence and
`poll_deposit_wallet_deployment` requires `WALLET-CREATE` evidence. Both call
the existing crate-internal expected-type transaction read, retaining its
transaction-id, owner, from, configured-factory `to`, derived
`proxyAddress`, state, hash, and read-permit validation. New, Executed, and
Mined update the last pending state and continue. Confirmed returns the receipt
and is the only terminal success. Failed and Invalid remain immediate typed
errors. Unknown state, wrong type, missing owner evidence, read blocking, and
all other reconciliation-required evidence stop immediately and are never
blindly retried.

Transport `Http` errors, every `Api` error, and a transaction temporarily
absent from an array response are transient within the finite policy. Treating
all `Api` statuses as transient is deliberate: it keeps transport plumbing
unchanged and bounded, but it means non-transient 4xx conditions such as an
authentication failure may consume every allowed attempt before surfacing as
`Exhausted`. This loses status-level observability at the polling outcome.
Consumers and operators must retain request/error telemetry, and a later
observability task may introduce a reviewed structured classification without
changing the no-resubmit contract.

The transport already parses `Retry-After`, but this polling layer does not use
it to change the interval. The bounded local policy remains the sole schedule,
so attempts and maximum delay stay caller-auditable. Exact server-directed
delay handling is deferred to the same later observability review rather than
requiring transport-layer surgery here.

Cancellation is a caller-provided `Future<Output = ()> + Send`. A biased
`tokio::select!` checks the pinned cancellation future before each read and
each interval sleep. Dropping an in-flight GET is safe because the read is
idempotent. Callers that do not need cancellation pass
`std::future::pending::<()>()`.

`RelayerPollOutcome::Exhausted` records the exact attempts and last observed
pending state, if any. `Cancelled` records completed attempts. Neither outcome,
nor any returned error, grants authority to fetch a nonce, re-sign, resubmit,
repeat WALLET-CREATE, or infer transaction absence. This PBRSDK-10 decision
does not add recent lookup, owner intent persistence, reconciliation, or
duplicate-submit recovery; ADR-0013 and ADR-0014 add the later bounded pieces
without changing these polling semantics.

Paused-time loopback tests follow addendum A1: the polling-only reqwest client
has no request timeout, and the polling server uses plain accept and read
futures with no Tokio timeout. Therefore the only timers during non-cancelled
polling are the production interval sleeps, and `tokio::time::Instant` advances
by exactly their sum. Removing the server accept timeout means a missing
request can hang the individual test until the CI or command runner timeout;
that is an accepted test-infrastructure risk, not production behavior.

Consequences:

- the public surface adds `RelayerPollPolicy`, `RelayerPollOutcome`, and exactly
  the two reviewed `poll_*` methods, all explicitly re-exported;
- the three low-level read methods and their four read-permit occurrences
  remain unchanged, and combined production source still exposes exactly two
  public `submit_*` methods;
- deterministic loopback tests prove exact attempt counts and `1s + 2s`
  scheduling, max-interval capping, terminal failures, unknown-state stop,
  bounded 429/5xx handling, absence propagation, cancellation priority, type
  isolation, and permit rejection before HTTP;
- this decision adds no submit, signing, nonce fetch, recent-transactions read,
  live host call, persistence, or automatic reconciliation path and does not by
  itself authorize end-to-end live operation.

## ADR-0013: PBRSDK-12 Owner-Scoped Mutation Intent Registry

Decision:

```text
Place a synchronous, consumer-supplied persistence boundary in front of every
approved live mutation entry. Atomically issue an owner/chain intent epoch
before nonce, signing, or HTTP, fence every later write with epoch/revision
CAS, and reopen the scope only after bound terminal evidence or later
authoritative reconciliation.
```

`MutationIntentStore` is synchronous to avoid a new async-trait dependency.
Its implementation must be fast local storage; a consumer using blocking I/O
must isolate that work behind its blocking boundary. `try_begin` performs the
unresolved check and generation issuance in one DB transaction/CAS or, for the
provided development implementation, one `Mutex` critical section. The store,
not the caller, ignores template generations and assigns epoch zero initially
or the prior resolved epoch plus one. Overflow fails closed. `update` compares
owner, chain, epoch, and revision, ignores the candidate revision, and assigns
`expected_revision + 1`; stale writers receive `false` and cannot overwrite a
newer or terminal state.

The persisted statuses are Preparing, Submitted, AmbiguousNoId, Confirmed,
Failed, and Reconciled. Preparing, Submitted, and AmbiguousNoId are unresolved
and block a second lease for the same owner and chain. Confirmed, Failed, and
Reconciled admit a new generation. Different owners or chains are independent.
Every successful write preserves `created_at_unix` and refreshes
`updated_at_unix` from `RelayerClock`.

`MutationIntentRecord` deliberately has no raw signature, auth header, private
key, calldata, or replayable submit-body field. It stores only the scope,
versions, operation/status, optional decimal nonce, payload keccak, saturated
deadline seconds, transaction id, safe observed-state label, and timestamps.
Serialization retains the owner and transaction id so a durable store can
round-trip and reconcile identity. Manual Debug redacts the owner and hashes
the transaction id.

`InMemoryMutationIntentStore` is test/development-only. It loses the owner
block on process restart and therefore cannot protect live traffic. The crash
acceptance contract is met through the public durable-store boundary: a
durable implementation restores the unresolved row and a new registry rejects
mutation after restart. PBRSDK-24/25 live gates require durable wiring and
restart evidence before this layer may protect live execution.

`MutationIntentLease` holds the store-issued epoch and current revision. It
records a payload only while Preparing, a transaction id only on the single
Preparing-to-Submitted transition, transaction-bound pending/Confirmed
receipts only while Submitted, AmbiguousNoId only from Preparing, and a proven
local pre-submit abandonment as Failed. The latter keeps
`last_observed_state = None` to distinguish a local failure before submit from
a relayer terminal state. Dropping the lease is intentionally a no-op: implicit
release would turn a panic, cancellation, or crash into duplicate-submit
authority. A CAS miss permanently poisons that lease and returns the fixed
stale-lease error.

Terminal polling integration is registry-level so it remains usable after
restart without reconstructing a lease. `record_poll_outcome` applies only to
a Submitted row whose stored transaction id equals the explicitly supplied
poll target. A Confirmed variant is trusted only when the receipt repeats that
id and its state is actually Confirmed. Exhausted records only a present state
label and stays Submitted; Cancelled records nothing. `record_terminal_failure`
resolves only a matching Submitted row and only for `TransactionFailed` or
`TransactionInvalid`. Mismatched or delayed observations are no-ops. Unknown,
ambiguous, and reconciliation-required errors keep the owner locked until an
ADR-0014 authoritative or evidence-bound reconciliation path is used.

`OwnerMutationRegistry::gate` returns `IntentGatedClient`, whose three methods
wrap the unchanged existing execute, create-submit, and deployment lifecycle
methods. DryRun delegates without a lease. Live execution writes Preparing
before the opaque primitive can fetch a nonce, sign, or send HTTP. A submitted
receipt records payload hash and then transaction id. Reconciliation-required
errors and every `Api` error become AmbiguousNoId; this deliberately
over-blocks nonce/deployed GET 429/5xx because the current opaque primitive
cannot distinguish read phase from a POST that may have been accepted. Other
errors become the local pre-submit Failed representation. PBRSDK-13 does not
add internal phase provenance, and no round guesses that a failed API call was
safe to retry.

The deployment wrapper performs one extra deployed read before deciding
whether a lease is appropriate. AlreadyDeployed, Predeployed, absent mutation
authority, and DryRun do not create owner history. A missing wallet with
DeployIfMissing and a Live permit begins the lease, then delegates to the
unchanged lifecycle, whose own preflight accounts for the intentional second
read. If that second read observes deployment, the lease is abandoned as a
local pre-submit Failed record.

Intent begin is write-ahead, while payload and transaction progress are
recorded after the primitive returns. If the post-submit store update errors or
loses CAS, the error is propagated without the raw transaction id and the
persisted Preparing row remains unresolved. This fail-closed lock is safer than
claiming the transaction was absent. There is no automatic resubmission,
automatic reconciliation, Reconciled transition API, recent-transactions
lookup, file/DB implementation, or lease release on Drop in PBRSDK-12. The
public nonce field remains `None` because the unchanged opaque primitive does
not expose the fetched nonce; a later internal provenance hook may fill it.

Consequences:

- the explicit public surface adds `MutationIntentStore`, `TryBeginOutcome`,
  `InMemoryMutationIntentStore`, `MutationIntentRecord`,
  `MutationIntentStatus`, `OwnerMutationRegistry`, `MutationIntentLease`, and
  `IntentGatedClient` through the HTTP, deposit-wallet, and crate-root exports;
- existing `DepositWalletRelayerClient` methods and the source files that own
  them are unchanged; live consumer policy now requires the registry wrapper,
  while later live gates enforce durable-store wiring;
- the combined production source contains the two original permit-bound
  submit primitives plus one intent-gated submit wrapper, and matching
  primitive/wrapper pairs for execute and deployment lifecycle;
- deterministic in-memory, injected-clock, serialization, loopback,
  disconnect, and 5xx tests prove lifecycle, owner/chain isolation,
  transaction binding, restart recovery through a shared store, fencing,
  overflow, conservative error mapping, DryRun no-lease behavior, and
  secret-free state;
- this layer blocks duplicate entry but does not complete ambiguous recovery,
  durable database qualification, consumer actor queuing, or end-to-end live
  readiness.

## ADR-0014: PBRSDK-13 Evidence-Bound Ambiguous Reconciliation

Status: accepted for the additive `0.2.0` deposit-wallet surface.

An unresolved owner may be released only by authoritative polling of its
already stored transaction id or by an explicit operator action carrying a
validated `ReconciliationEvidence`. There is no evidence-free release method,
automatic resubmit, or automatic candidate adoption.

`ReconciliationEvidence` records a non-secret operator reference, one explicit
`ReconciliationDecision`, a bounded summary, and the registry clock time. The
reference is trimmed, non-empty, at most 256 bytes, and contains no control
characters. The summary is trimmed, non-empty, at most 1024 bytes, and permits
newlines but no other control characters. Serialize retains the reviewable
text; manual Debug exposes only the decision, timestamp, and reference/summary
lengths. `MutationIntentRecord.reconciliation` uses `#[serde(default)]` so
PBRSDK-12 rows remain loadable. Derived Deserialize follows the same durable
store-trust model as the rest of `MutationIntentRecord`: loaded evidence is not
revalidated, so consumers must protect and validate their persistence layer.

Both manual registry writes require the epoch inspected by the operator.
`reconcile_manually` accepts any unresolved Preparing, Submitted, or
AmbiguousNoId row and moves it to Reconciled with evidence. `adopt_transaction`
accepts only AmbiguousNoId, validates and stores the operator-confirmed
transaction id, and moves the row to Submitted so the existing expected-type
polling path remains authoritative. Adoption without evidence is not exposed.
An epoch mismatch fails immediately with a generation-changed error, preventing
evidence for generation A from modifying a later generation B.

These operator paths use load, state/epoch checks, and epoch/revision CAS. One
CAS `Ok(false)` causes a fresh load, a repeated epoch/state check, and exactly
one retry. A second miss returns `concurrent intent update; retry
reconciliation`; resolved, replaced, or generation-changed rows are never
overwritten. This explicit bounded failure is preferable to a silent no-op for
an operator decision.

`IntentGatedClient::reconcile_by_polling` first loads the unresolved record and
uses only a stored Submitted transaction id. WALLET intents call
`poll_wallet_transaction`; WALLET-CREATE intents call
`poll_deposit_wallet_deployment`. Confirmed and bound Failed/Invalid results are
fed only through the existing `record_poll_outcome` and
`record_terminal_failure` APIs. Exhausted remains Submitted, Cancelled records
nothing, and unknown/reconciliation-required errors propagate with the lock
unchanged. The method performs no nonce read, signing, or submit. The existing
registry recording APIs deliberately treat a stale CAS as a no-op; therefore a
returned `Resolved` can differ from the stored status when a newer concurrent
write won. The newer stored write remains authoritative and must be inspected.

For an id-less ambiguity,
`IntentGatedClient::report_ambiguous_candidates` performs a permit-first,
read-only authenticated `GET /transactions` with no query parameters. The
official TypeScript SDK `@polymarket/builder-relayer-client` `0.0.10` commit
`9122f6fb1856f1ecfe4406685bfa19a2c5a7b290` defines that endpoint and returns a
top-level `RelayerTransaction[]`. Reports include the redacted owner, inspected
intent status, payload hash when present, epoch, creation time, validated
WALLET/WALLET-CREATE candidates, and a fixed redaction marker. They omit auth
material and raw bodies and never mutate the intent.

Malformed, other-owner, non-WALLET, invalid-id, and unknown-state items are
excluded and counted in `skipped_items`; unrecognized state labels are never
copied into review output. An unsafe or oversized `createdAt` is omitted while
the otherwise valid candidate remains. Per-item skipping is appropriate here
because the endpoint is evidence discovery only and cannot unlock anything;
failing the whole report would lose safe operator context without increasing
state safety. Invalid top-level JSON/envelopes and more than 32 items still fail
the report. Candidate matching is never a reconciliation verdict: the operator
must adopt a chosen transaction with evidence or manually reconcile, and a
known transaction id must be polled rather than submitted again.

## ADR-0015: PBRSDK-15 Redacted Mutation Audit and Tracing Contract

Status: accepted for the additive `0.2.0` deposit-wallet surface.

`MutationIntentAuditArtifact` is the ticket- and PR-attachable audit view of a
stored mutation generation. Its schema is explicitly versioned by
`MUTATION_AUDIT_ARTIFACT_SCHEMA_VERSION = 1`. Any later change to field names,
field meaning, redaction behavior, or reconciliation-summary semantics requires
a schema-version increment; consumers must branch on `schema_version` rather
than assume a newer shape is v1. The registry exports any recorded state,
including resolved rows, through the read-only
`OwnerMutationRegistry::export_audit_artifact(owner, chain_id)` method. Export
does not update the row or its revision.

Schema v1 contains the redacted owner, chain, operation, epoch, revision,
status, optional nonce and payload hash, optional deadline and transaction id,
safe last-observed-state label, cumulative poll attempts, reconciliation
summary, timestamps, and a fixed omission marker. It never contains an API
key, auth header, private key, signature, signed typed data, full calldata, or
full replayable submit body. JSON retains the transaction id because operators
need it for adoption and polling; manual artifact Debug prints only its
`sha3:0x...` sanitized token. Owner is always the existing shortened checksum
form.

The artifact never serializes `ReconciliationEvidence.operator_ref` or
`summary`. Those fields are operator-authored free text and can contain pasted
secrets, including in rows restored through Deserialize without constructor
validation. `ReconciliationSummary` includes only decision, recorded time, and
the stored strings' UTF-8 byte lengths from `value.len()`. Export does not trim
again and does not count chars or graphemes. An operator who needs the original
evidence text must read the protected durable-store record directly; that
record must not be attached as the redacted artifact.

`MutationIntentRecord.poll_attempts` uses `#[serde(default)]`, so pre-v1/R7
rows load as zero. A matching Confirmed poll contributes one observation.
Exhausted with a present last state contributes its reported attempts.
Exhausted without a state and Cancelled retain their existing early-return
no-op behavior and contribute nothing. Additions use `saturating_add` and ride
the existing state-write CAS; no extra write is introduced. Unknown state text
is replaced with `<unrecognized relayer state>` both when a poll label is
stored and again when any durable row is exported. The two defenses are
independent so bypassing the normal writer cannot leak a provider or caller
string through the artifact.

`DepositWalletDryRunEvidence` and `MutationIntentAuditArtifact` are
complementary, not interchangeable. The dry-run evidence is created before
submit and owns selector/data-length/call summaries. The audit artifact owns
the persisted intent lifecycle after begin. `payload_keccak256` is their
non-secret correlation key. The audit artifact deliberately repeats no method
selector or call summary.

Mutation-intent tracing uses the fixed target
`polymarket_relayer::mutation_intent`. Every event has only redacted `owner`,
`chain_id`, and `epoch` as common correlation fields. The closed event-specific
field contract is:

```text
mutation intent started: operation
mutation intent blocked: status
mutation submitted: sanitized transaction_id
mutation ambiguous without transaction id: no additional field
mutation intent resolved: status, sanitized transaction_id
mutation intent abandoned before submit: no additional field
mutation intent manually reconciled: decision
transaction adopted: decision, sanitized transaction_id
```

Resolved events from registry poll/failure paths are emitted only after the
existing CAS returns `Ok(true)`; a stale no-op cannot claim resolution.
Lease-based events are emitted only after their existing persist succeeds.
Nonce is intentionally artifact-only and never a tracing field. Operator
reference/summary text, payload bodies, signatures, typed data, auth material,
and raw transaction ids are also forbidden in these events. No metrics, OTel,
collector, or logging-system dependency is introduced.

Regression tests preload malicious reconciliation text and unknown state
labels through Deserialize, exercise success and failure Display/Debug paths,
capture the current-thread tracing dispatcher in memory across async awaits,
and inspect the actual locally submitted signature, calldata, and full body as
forbidden sentinels. This is offline redaction and lifecycle evidence only; it
does not qualify a durable store, production log collector, or live relayer
execution.

## ADR-0016: PBRSDK-17 Verified Calldata Config And Allowlist

Status: accepted for the additive `0.2.0` deposit-wallet surface.

Deposit-wallet calldata builders must not choose contract targets, approval
spenders, operators, or collateral units from unverified caller input. The
`DepositWalletCalldataConfig` constructor is therefore the only validation
boundary for the future PBRSDK-18/PBRSDK-19 builders. Every address is carried
as `SourcedAddress`, every source is a validated `CalldataSourceRef`, and the
pUSD decimals value has its own source reference. Source names and versions are
trimmed, non-empty, at most 256 bytes, and free of control characters. Source
URLs are HTTPS-only, at most 512 bytes, and free of control characters.

This config supports only Polygon chain `137`. Amoy is intentionally rejected
because the reviewed wire truth does not provide an official Amoy pUSD
address. The Polygon pUSD, CTF, Standard Exchange, and Neg Risk Exchange
addresses are fixed to the values in `SM-CALLDATA-PUSD-ADDR` and
`SM-CALLDATA-CTF-EXCHANGES`. pUSD collateral uses six decimals from
`SM-CALLDATA-PUSD-DECIMALS`. The evidence is the official Rust CLOB SDK
collateral constant and CTF example; there is no reviewed pUSD-specific
decimals document, which remains an explicit residual risk.

`CalldataConfigInput` is a public unvalidated DTO, not an alternate authority.
`DepositWalletCalldataConfig::try_new` rejects zero addresses, token-address
collisions, duplicate entries, pUSD self-approval, empty spender/operator
lists, and every value outside the reviewed Polygon wire truth. Valid spender
lists may contain only CTF, Standard Exchange, and Neg Risk Exchange. Valid CTF
operator lists may contain only Standard Exchange and Neg Risk Exchange.
Strict subsets are preserved exactly rather than expanded to the canonical
set, so a consumer cannot gain authority it did not request.

No official deposit-wallet adapter address was available in the PBRSDK-17
reviewed source set, so that round kept the adapter allowlist empty. ADR-0018
later supersedes only that adapter clause with the source-pinned Polygon
NegRiskAdapter subset rule. This ADR does not itself infer an adapter target or
enable split, merge, or redeem behavior.

The canonical `polygon_calldata_config()` function constructs the reviewed
full allowlists and passes them through the same fallible validation path.
Addresses and sources are serialized for offline review with checksum address
format, but none of the config types implements `Deserialize`. There is no
environment, file, or runtime config loading path that could bypass
construction-time validation. The module is synchronous and has no HTTP
dependency.

This change adds source-backed configuration only. It does not encode calldata,
select ABI methods, submit a wallet batch, change the HTTP layer, or authorize
live execution. Consumers can roll back by ceasing to import the additive
config surface; existing public APIs and request paths remain unchanged.

## ADR-0017: PBRSDK-18 Unit-Safe pUSD And CTF Approval Calldata

Status: accepted for the additive `0.2.0` deposit-wallet surface.

pUSD approval amounts use the public `PusdAmount` newtype rather than a raw
integer. Its sole `U256` base-unit field is private, and its fallible base-unit
and whole-pUSD constructors reject zero. Whole pUSD is converted through
`u128` at six decimals, which exactly accommodates every `u64` input without an
unreachable overflow branch. `unlimited()` is a separate explicit constructor
for `uint256::MAX`; it is only a representation and does not choose an
unlimited-approval policy. Manual `Serialize` and `Debug` implementations emit
decimal-string `base_units` plus `decimals = 6`, avoiding the ambiguous default
hex-only `U256` representation.

The pUSD builder encodes local selector `0x095ea7b3` followed by ABI
`address,uint256`; the CTF builder encodes local selector `0xa22cb465` followed
by ABI `address,bool`. Targets always come from the supplied
`DepositWalletCalldataConfig`, and call value is always zero. A supplied zero
spender/operator is rejected distinctly before the allowlist check. Every
well-formed non-zero address must then be present in the supplied config's pUSD
spender or CTF operator allowlist, including when a consumer chooses a strict
subset. The pUSD builder also rechecks for a zero base-unit amount even though
normal `PusdAmount` construction prevents it. CTF `approved = false` remains a
valid revocation call and receives the same operator allowlist check.

The legacy `operations::approve` and `contracts` defaults are not reused. They
are Safe/Proxy-era USDC.e/global-address helpers and do not establish the pUSD
target, the caller's narrowed allowlist, or the pUSD unit invariant required by
this flow. The new approval module therefore owns its two reviewed selector
constants and uses only PBRSDK-17 config getters plus standard ABI encoding.

Two flat deposit-wallet fixtures pin selector, argument order, target, zero
value, and metadata. The pUSD `uint256::MAX` fixture is byte-identical to the
recorded local call in `wallet_submit_body.json`. Additional tests decompose a
different verified spender, finite amount, and different verified CTF operator
word-by-word so a hard-coded recorded payload cannot pass. Negative tests cover
attacker-looking addresses, zero addresses, strict-subset configs, zero amount
construction bypass, and CTF revocation. `SM-CALLDATA-APPROVAL-ENCODING`
records this offline wire boundary and the fixture provenance ledger records
both new fixtures.

This is an additive pure builder API. It does not add split, merge, redeem,
adapter routing, batch composition, nonce fetching, signing, HTTP submission,
or a live-execution claim. Consumers can roll back by ceasing to import the
three new crate-root symbols; existing public signatures and request paths are
unchanged. Residual risks remain the separately recorded lack of a
pUSD-specific official decimals document and the future policy choice around
unlimited approvals. PBRSDK-19/PBRSDK-20 and later live gates must resolve
their own route, composition, authorization, and rollback requirements.

## ADR-0018: PBRSDK-19 Verified CTF Split, Merge, And Redeem Routes

Status: accepted for the additive `0.2.0` deposit-wallet surface.

The typed calldata surface contains exactly four verified routes:
`ConditionalTokensSplit`, `ConditionalTokensMerge`,
`ConditionalTokensRedeem`, and `NegRiskAdapterRedeem`. Unsupported or
unreviewed routes are not variants of `CtfRoute`; there is no generic route
escape hatch or status API. This closed enum is the failure boundary for an
unsupported route. Each builder obtains its selector through
`CtfRoute::selector` and its target through `CtfRoute::target`, so route,
selector, target class, and adapter presence cannot drift independently.

The authoritative source is the official Rust CLOB SDK commit
`3ae1aae5e9ded38f984464c9fc0f307f8a9f41fb`. ConditionalTokens split, merge,
and redeem target Polygon CTF
`0x4D97DCd97eC945f40cF65F87097ACe5EA0476045`; their selectors are respectively
`0x72ce4275`, `0x9e7212ad`, and `0x01b7037c`. NegRisk redeem targets only the
caller config's allowlisted Polygon NegRiskAdapter
`0xd91E80cF2E7be2e162c6513ceD06f1dD0dA35296` with selector `0xdbeccb23`.
ADR-0016's adapter-empty rule is narrowed only to `adapter_allowlist` being a
subset of this one reviewed address. An empty subset remains valid and
authoritative for consumers that disable adapter routes. No other PBRSDK-17
wire-truth, structural, duplicate, zero, collision, spender, or operator
invariant is relaxed.

All ConditionalTokens routes take collateral only from `config.pusd()` and
target only `config.ctf()`. Callers cannot supply a legacy USDC.e collateral
address. `parentCollectionId` is fixed to zero because that is the only route
reviewed for this round. Split and merge use non-zero, finite `PusdAmount`
values in six-decimal collateral base units; `uint256::MAX` remains an approval
representation and is rejected for these operations. CTF redeem has no amount
argument and redeems the selected position sets in full. NegRisk redeem uses
the separate `CtfPositionAmount` type; each amount is non-zero and represents a
CTF outcome-position quantity at the CTF 1:1 collateral base-unit ratio.
Partitions and index sets are non-empty, non-zero, unique, and at most 64
entries. NegRisk amounts are non-empty and at most 64 entries, but duplicate
quantities are valid because they are amounts rather than index identifiers.

Four flat golden fixtures pin the target, zero value, selector, ABI bytes,
condition id, zero parent, arrays, units, address, and source. Alternate-input
tests decompose every route at its verified dynamic-array offsets so a fixed
fixture payload cannot pass. Negative tests pin all exact errors, both adapter
subset decisions, 64/65 boundaries, test-only zero invariant bypasses, and
split/merge unlimited rejection. The source matrix row
`SM-CALLDATA-CTF-ROUTES` records the same route contract and makes any address,
signature, selector, argument, or unit drift a live-gate blocker.

The review also found two concrete legacy drifts. `src/contracts.rs` publicly
exposes stale merge selector `0xd37bf42e` rather than `0x9e7212ad`, and
`src/operations/redeem.rs::redeem_neg_risk_positions(condition_id, index_sets)`
places index sets into the NegRisk adapter's amounts argument. Those legacy
Safe/Proxy-era public paths remain unchanged in this round to preserve the
existing public API and warning-free frozen examples. They are not valid for a
deposit-wallet WALLET batch. Source boundary tests prohibit the new calldata
modules from referencing `crate::operations` or `crate::contracts`, and
consumers must enforce the same separation. The remaining public legacy
surface is an explicit residual risk until a separately scoped compatibility
migration can remove it.

This change builds individual offline `DepositWalletCall` values only. It does
not compose a batch, prepare a condition, calculate a position id, fetch a
nonce, sign, submit HTTP, synchronize CLOB state, or authorize live execution.
Consumers migrate by selecting one of the four explicit builders and may roll
back by ceasing to import the additive symbols or by narrowing the adapter
allowlist to empty. Live enablement remains blocked until later composition,
authorization, submit, reconciliation, and operator gates pass on unchanged
source.

## ADR-0019: PBRSDK-20 WALLET Batch Composition And Redacted Summary

Status: accepted for the additive `0.2.0` deposit-wallet surface.

PBRSDK-20 permits the six already verified PBRSDK-18/PBRSDK-19 builder outputs
to be placed, in caller-selected order, into one candidate `Vec<DepositWalletCall>`
and passed through the existing public WALLET signing/request preflight. This
narrows ADR-0018's individual-call-only boundary only for offline composition,
review summary, and fixture-backed request compatibility. It does not add a
builder or route, change any wire format, fetch a nonce, create a signature,
submit HTTP, authorize live mutation, or change the legacy Safe/Proxy surface.

`summarize_batch_calls` and `DepositWalletDryRunEvidence` remain separate
because they have different stages and owners. The batch summary is a pure
pre-permit view that a consumer can create while selecting candidate calls.
Dry-run evidence belongs to the mutation client after its capability,
identity, deadline, resource, signature, and request checks. The batch summary
therefore neither duplicates nor replaces the payload-bound dry-run artifact,
and dry-run evidence is not relaxed by this decision.

The summary deliberately performs no validation. It accepts an empty batch,
empty call data, and any other `DepositWalletCall` so observation cannot become
an alternate signing policy. The existing
`try_build_wallet_batch_request_with_signature` preflight remains authoritative
for supported contract config, call count, total calldata bytes, derived
wallet, signature shape, owner identity, and signature recovery. Its private
resource constants and validator remain private; the external integration test
exercises both limits only through that public entry point.

`BatchCallSummary` contains exactly a redacted checksum target, decimal value,
optional selector, and exact data length. `DepositWalletBatchSummary` contains
the exact call count, exact sum of calldata bytes, order-preserving call
summaries, and the fixed marker
`full calldata and signatures are intentionally omitted`. Both types have
private fields and read-only getters. Targets use the existing
`0x1234...ABCD` convention. A selector is present only when `data.len() > 4`;
when `data.len() <= 4` it is absent because four bytes would make the selector
identical to the complete calldata. No signature, auth material, full target,
payload bytes, or full calldata is stored.

The same boundary now applies to `DryRunCallSummary` in the mutation dry-run
path. That code predates this ADR and used `data.get(..4)`, which succeeds at
`len == 4` and therefore published the whole calldata as a "selector". The
defect was reachable through the shipped dry-run evidence path and through the
consumer adapter that forwards these summaries, so it was not hypothetical.
The rule is now stated once and enforced in both places, each with a four-byte
and a five-byte regression test; reverting either site fails its own test.

The lesson is recorded deliberately: this ADR fixed the rule for one summary
type while an equivalent type in another module kept the old behaviour. A
confidentiality rule stated for one representation has to be applied to every
representation of the same data, and the audit for it belongs with the rule
rather than with the site that happened to be edited first.

There is intentionally no calldata hash field. Calldata is often
low-entropy. Once selector and exact length are known, a non-keyed hash becomes
an enumeration oracle: a five-byte call has only 256 candidates for its final
byte, and approval inputs may also have small effective search spaces when the
spender allowlist and conventional amount are known. Matching candidates to a
non-keyed hash can recover the original payload, making that commitment
equivalent to disclosure for this shareable summary. This API gives up binding
in favor of non-disclosure. If binding is later required, it must be handled in
an access-controlled path rather than added to this summary.

The precise empty-data disclosure contract is also fixed by this ADR:

- Allowed disclosure is the call category: a reviewed route selector only
  when `data.len() > 4`, or the fact that there is no exposed route. Exact
  `data_len`, `total_calldata_bytes`, `call_count`, decimal `value`, and only a
  redacted target are also allowed.
- Forbidden disclosure is calldata payload bytes or a derivative that makes
  them recoverable, including a whole-data non-keyed hash and a selector when
  `data.len() <= 4`. Signatures, auth material, secrets, and full target
  addresses are forbidden.
- `data_len == 0` says that no payload exists; it does not expose payload
  content. Reconstructing the zero-information empty value is tautological,
  not recovery of a hidden parameter.
- A summary is not literally replayable because it omits the full target and
  execution also requires owner authorization. The material risk addressed
  here is parameter disclosure, and empty data has no parameter.
- The design intentionally exposes a route selector for data longer than four
  bytes. Reporting that a call has no route or is a pure value transfer reveals
  strictly less than that permitted route category.
- Length bucketing would remove no practical payload disclosure while breaking
  `total_calldata_bytes == sum(call.data.len())` and the summary's purpose of
  finding unexpectedly large payloads. Exact per-call and total lengths are
  therefore retained without an empty-data exception.

The composition contract is tested from an external integration test crate,
not a source-module unit test. That placement proves the crate-root public API
is sufficient without widening a private helper. The test composes all six
verified builders, locks call order and selectors, rejects replayable summary
output, triggers the two resource limits independently, covers wallet/config/
chain/signer failures, proves a narrowed wrong config prevents composition,
and passes the fixture approval call through the public WALLET request builder
to the serialized `calls[0]` shape.

Deadline freshness remains outside this serialization/composition layer.
`try_build_wallet_batch_request_with_signature` intentionally has no wall
clock, so treating fixture deadlines as fresh here would be a false runtime
claim. The actual deadline gate is the existing clock-injected execution and
submit path in `http/execute.rs` and `http/submit.rs`, covered by
`http/tests.rs`, and is unchanged.

Rollback is additive: consumers can stop importing the three summary exports
and stop composing PBRSDK-18/PBRSDK-19 calls. Existing builders, request APIs,
HTTP behavior, fixtures, dependencies, and public signatures are unchanged.
Residual live risks and gates remain those already assigned to nonce,
authorization, submission, reconciliation, operator review, and unchanged-
source live qualification.

## ADR-0020: PBRSDK-22a Typed Identity Boundary And Read-Only Rollback Evidence

Status: accepted for the additive `0.2.0` deposit-wallet surface.

PBRSDK-22a adds an opt-in configuration boundary for the three addresses that
must not be conflated: the relayer API-key authentication identity, the EOA
that owns the deposit wallet and signs WALLET batches, and the deployed deposit
wallet contract that is also the CLOB funder. `RelayerAuthIdentity`,
`DepositWalletOwner`, and `DepositWalletAddress` are distinct private-field
newtypes, and `DepositWalletIdentityConfig` requires each role explicitly.
Consequently, swapping these values is a type error only on paths that use the
new identity config boundary. Existing raw-`Address` constructors, request
types, public fields, and function signatures remain unchanged, so this ADR
does not claim crate-wide type enforcement.

Each newtype exposes only explicit `new` and `address` methods. The design
forbids cross-role `From`/`Into`, `Deref`, and `Display`: an implicit conversion
would erase the role distinction, while `Display` would create an easy
full-address logging path. The design also keeps the identity newtypes and
config outside the serializable wire DTO surface. Their manual `Debug`
implementations use the existing checksum-prefix/suffix redaction. The
implementation module remains private while the six reviewed types are
explicitly re-exported from `deposit_wallet` and the crate root. The exact
mechanically checked subset of these rules is listed under Proof Scope below.

`DepositWalletIdentityConfig::try_new` rejects any zero identity as
`RelayerError::InvalidAddress` before attempting derivation. This order is
part of the contract: deriving from a zero owner can return a non-zero-looking
address, so derivation-first validation could misclassify an unset owner as a
relationship failure. After all three non-zero checks, the constructor derives
the wallet from the owner and supplied contract config and requires an exact
match. A mismatch is `RelayerError::Signing` because the individual addresses
are well formed but the owner-to-wallet signing relationship is invalid.

Address equality is observed, not rejected. `IdentityOverlap` reports all
equal pairs in the fixed order auth/owner, auth/wallet, owner/wallet, and
`as_key` fixes their stable external keys. Equality may be an explicit
operator configuration, and whether to reject it belongs to higher-level
policy. The owner-to-wallet derivation invariant still applies independently.

`IdentityConfigSummary` is the only serializable identity observation type.
Its design contract is five private fields containing three redacted
addresses, stable overlap keys, and the fixed marker
`full identity addresses are intentionally redacted`, with neither a full
address nor an address hash stored. As in ADR-0019, a non-keyed hash can become
a confirmation oracle when the deployment context or address allowlist is
known, so this shareable summary favors non-disclosure over binding. The
mechanical evidence for this contract is deliberately narrower and follows.

### Proof Scope

The following properties are compile-time claims with enumerated evidence:

- **Compile-time negative trait assertions:** all six cross-role directions
  are checked separately for owned `From`, owned direct `Into`,
  `From<&'static Source>`, and `&'static Source: Into<Destination>`, for 24
  assertions total. These are distinct trait instantiations; proving one does
  not prove another. Same-type `Into<Self>` is not asserted because reflexive
  `From<T> for T` supplies it.
- **Compile-time negative trait assertions:** each of the three newtypes and
  `DepositWalletIdentityConfig` is checked for absence of `Display`, `Deref`,
  and `Serialize`, plus both `for<'de> Deserialize<'de>` and the exact
  `Deserialize<'static>` instantiation.
- **Every-build compile-time field pin:** the unconditional, non-`cfg`
  `_identity_config_summary_field_shape_is_pinned` function exhaustively
  destructures the five named private `IdentityConfigSummary` fields without
  `..`. Adding a field, including a `#[serde(skip)]` field restricted to a
  non-test artifact, therefore produces E0027 in that build configuration.

Other reviewed properties use narrower evidence and are not compile-time
claims:

- **Production-artifact integration exact assertions:** the three newtypes,
  config, and summary have complete literal expectations for both `{:?}` and
  `{:#?}`, and the summary JSON has a complete literal expectation. The
  integration target links the normal library artifact, so a test-only
  representation cannot satisfy this evidence. The expectations do not call
  the production redaction helper as their oracle.
- **Fixed sentinel assertions:** the reviewed outputs omit the listed raw
  lowercase addresses and six listed address/checksum-string hashes. This
  defends only those specific encodings; uppercase, base64, and decimal
  byte-array representations are not detected by these sentinels.
- **Source audit:** recursive `src/` checks retain the literal and fully
  qualified conversion-pattern defense, and `identity.rs` contains zero
  `cfg(not(test))` strings. Alias syntax can bypass literal search, so source
  audit is defense in depth and does not replace the compile-time assertions.

The proof scope explicitly excludes `&mut` receiver-based conversions; no
negative assertion is made for them. Implementations generated by macros or
build scripts and extensions supplied by external crates are also outside the
documented proof scope. Accordingly, this ADR does not claim that every
possible conversion or identity-disclosure path is sealed.

Compatibility is additive. `DepositWalletIdentityConfig::request_context`
creates the existing public `DepositWalletRequestContext`, and
`RelayerKeyAuth::from_identity` delegates to the unchanged
`RelayerKeyAuth::new`. These adapters deliberately return to raw `Address` at
the legacy boundary. Existing callers therefore compile unchanged, while new
consumer wiring can validate and keep the three roles distinct until that
boundary.

Rollback evidence is also additive and offline. The external
`mutation_rollback_boundary_test` constructs the existing production-host URL
value but starts no server and performs no network round trip. After
`disable_mutation`, a valid Live WALLET-CREATE permit is rejected by the
mutation predicate and fixed disabled-latch message before dispatch. On the
same disabled client, owner-mismatched and chain-mismatched read permits still
reach their independent read-only validation predicates before dispatch.
`public_api_boundary_test` continues to prove that no public
`enable_mutation` path exists and additionally forbids public `set_mutation*`
paths across the complete production HTTP source.

A successful network-backed read assertion is intentionally outside this
external integration target. The public URL type accepts only the production
host, the repository has no external-integration loopback precedent, and
`Cargo.toml` plus dev-dependencies are frozen for this round. Adding a public
test transport, dependency, or bespoke server would widen the architecture to
duplicate evidence already present in `src/deposit_wallet/http/tests.rs`,
where loopback tests cover all three successful read methods and the rollback
latch preserving reads. The new test therefore proves pre-I/O rollback and
read-capability boundaries without making a live-read claim.

PBRSDK-22a is the fork-owned portion only. The consumer-owned PBRSDK-22b must
separately prove that only `pm-adapters/relayer_http` imports this fork, that
consumer port/domain types expose no fork DTOs, and that adapter tests preserve
the mapping. No consumer code, CLOB/POLY_1271 behavior, wire format, HTTP
behavior, dependency, fixture, credential, or live call changes here. Rollback
for the additive identity surface is to stop constructing the new config and
continue using the unchanged legacy entry points; live enablement remains
blocked until the consumer adapter, CLOB funder separation, and operator gates
pass.

## ADR-0021: PBRSDK-23a Behavior-Based No-CLOB Source Audit

Status: accepted for the fork-owned offline boundary gate.

The existing `repository_does_not_grow_clob_sdk_modules_or_examples` test is a
useful path-based first defense, but it rejects only a fixed set of module and
example names. An order implementation placed in `src/trading.rs`, or embedded
in an existing module, would avoid those names and pass. PBRSDK-23a therefore
retains that test and adds `tests/no_clob_surface_test.rs` as a behavior-based
source audit over every recursively discovered Rust file under `src/`. A
minimum corpus size of 40 prevents an empty or accidentally narrowed scan from
passing; the current corpus is discovered rather than hard-coded.

The audit case-insensitively rejects the reviewed CLOB-specific order,
signature, quote, balance, and cancellation markers. It also rejects `fn`
tokens followed by any of the reviewed order/post/sign/cancel/book/price/
balance-allowance function names, independent of visibility, `async`, repeated
spaces, or line breaks. Private helpers are in scope. After an independent
`fn` token, the scanner repeatedly skips Rust `Pattern_White_Space`,
non-documenting line comments, and nested block comments, then accepts an
optional raw-identifier `r#` prefix before comparing the function name. The
matched name must still end at a character outside the audit's conservative
underscore-or-Unicode-alphanumeric boundary instead of relying on an exact
`pub fn` substring. Consequently, `fn post_order_v2()` is intentionally not
matched because the underscore continues the reviewed identifier boundary.

Three existing markers require conditional exceptions, not file exemptions:

| Marker | Allowed source and frozen count | Reason |
| --- | --- | --- |
| `signature_type` | `src/types.rs` (8), `src/direct.rs` (4) | Existing relayer wallet abstraction values 0/1/2, unrelated to CLOB order signing and part of the legacy public API |
| `/orders` | `src/auth/builder.rs` (1) | HMAC reference vector under `#[cfg(test)]`; its first occurrence must remain after the file's first `#[cfg(test)]` |
| `rs-clob-client-v2` | `src/deposit_wallet/calldata/config.rs` (2) | Source citations for reviewed calldata constants |

Each listed file must exist, contain its marker, and retain the exact count.
The same marker in any other `src/` file, including a case-only variant, is a
violation. This preserves the legitimate compatibility and provenance text
without allowing the rest of an approved file to bypass the audit.

The audit implementation returns a structured list of all violations. Its
repository test and its synthetic negative tests call the same pure `audit`
function. Each mutation test first proves that the real in-memory `src/`
baseline is clean, then applies exactly one in-memory change and requires the
exact expected violation. The cases cover conditional occurrence counts and
test-region placement, forbidden signature markers, visibility/spacing,
repeated and nested comment trivia, mixed trivia, and raw identifiers. A
separate positive boundary case pins the longer-identifier behavior described
above. The disk corpus is never mutated.

### Proof Scope

The mechanical claim is limited to this: the specified markers and function
signatures do not appear anywhere in recursively collected `src/**/*.rs`,
apart from the path-and-count-pinned conditional occurrences above. It does
not prove that CLOB code is absent. An implementation that avoids the reviewed
vocabulary, generated code from macros or build scripts, and CLOB behavior
supplied by an external crate are outside this proof.

The function-declaration whitespace predicate exactly enumerates Rust's
language-level `Pattern_White_Space` set: `U+0009` through `U+000D`, `U+0020`,
`U+0085`, `U+200E`, `U+200F`, `U+2028`, and `U+2029`. It deliberately does not
use `char::is_whitespace()`, which implements Unicode `White_Space` and omits
the Rust lexer separators `U+200E` and `U+200F`. This language set is finite
and every member is independently mutation-tested. Together with repeated
non-documenting line comments, nested/repeated block comments, mixed trivia,
and raw identifiers, exact enumeration closes the lexical-trivia bypass class
for the reviewed exact-name function declarations. Remaining bypasses require
a renamed implementation, macro/build-script generation, or CLOB behavior
from an external crate, all of which remain explicitly outside this proof.
This closed-trivia claim does not claim exact Rust XID classification: a longer
identifier using a non-alphanumeric XID continuation mark can be conservatively
flagged. That is a false positive, not a trivia-based bypass.

The audit scope is `src/` only. Documentation is intentionally outside it;
for example, `POLY_1271` appears in `docs/` to describe consumer obligations.
Marker matching does not distinguish code, comments, or string literals, so
all three fail identically. This chooses conservative false positives over a
comment/string escape. Matching is ASCII case-insensitive but does not remove
underscores or otherwise normalize separators. `POLY1271`, `Poly1271`, and
`SignatureType` are consequently explicit forbidden entries rather than
depending on separator normalization.

There is a narrow structural reinforcement for the deposit-wallet client.
`DepositWalletRelayerClient` production URLs pass through
`DepositWalletRelayerUrl::parse`, which fixes the host to
`relayer-v2.polymarket.com`, accepts only a root base path with no query or
fragment, and constructs request URLs from crate-internal paths passed to
`endpoint(path)`. Those production request paths are crate-internal constants.
This reinforcement applies only to the deposit-wallet
client. Legacy `RelayClient::set_url` in `src/client.rs` has no host allowlist,
accepts an arbitrary base URL, and constructs paths as strings. This ADR
therefore does not claim that the crate as a whole cannot call a CLOB endpoint.

Conditional exceptions freeze occurrence counts and, for `/orders`, position
after the first `#[cfg(test)]`; they do not freeze complete source snippets.
Keeping the same count while replacing the meaning of an existing occurrence
with order code is therefore outside the audit. Complete snippet pinning was
intentionally rejected because its refactoring fragility outweighs the value
against that implausible count-preserving replacement scenario.

PBRSDK-23a changes no production source, public API, wire format, dependency,
fixture, consumer adapter, or live behavior. The consumer-owned PBRSDK-23b must
separately prove funder wiring, `POLY_1271` separation, fork-DTO absence in the
CLOB layer, and confirmed-only balance synchronization.

## ADR-0022: PBRSDK-27 Lifecycle-Truth Live Validation Decision Record

Decision:

```text
Record the unapproved live-validation outcome as a blocked lifecycle fact and
mechanically bind that status to one canonical Decision sentence, the
status-specific sections, and the fixed operator-evidence table. Do not run a
live submit or create production mutation authority without operator approval.
```

A decision record cannot establish its own validity through prose. A stale
claim that no request was sent is operationally dangerous after a later live
attempt, so `tests/live_validation_decision_test.rs` treats a disagreement
between lifecycle status and the canonical Decision sentence as a structured
failure. It also checks the designated status-specific section and evidence
table schema through the same CommonMark parser used to interpret the rendered
document. The audit intentionally does not claim that all surrounding prose is
factually consistent; that broader claim would exceed what the mechanical
rules inspect.

Document structure audits must use the same interpretation model as the
document renderer. Hand-written approximations split what readers see from
what the audit checks, and enumeration did not close that gap: trailing heading
spaces, tab-separated headings, indented fences and HTML comments, and setext
headings each produced an empty violation list in four consecutive rounds.
PBRSDK-27 therefore parses the document once with `pulldown-cmark`, builds H2,
paragraph, HTML, code-block, and table-aware evidence, and applies the frozen
rules to that structured view. Secret-shape scanning remains deliberately
separate and covers the complete raw source, including code blocks.

The parser and current CommonMark wording still diverge on tab-separated ATX
closing hashes. The audit does not add another heading normalization to chase
that boundary. Instead, every rendered H2 must match the frozen section-name
allowlist. Known sections can then be required, forbidden, or duplicated by
status, while every other H2 fails as `UnexpectedSection`. The safety rule is
allow known structures, not attempt to enumerate every unsafe spelling.

CommonMark still permits effectively unbounded rendered forms through raw HTML,
images, and other container or inline syntax. Five consecutive bypass rounds—
trailing spaces; tabs; indented fences and HTML comments; setext headings; and
raw HTML and images—each demonstrated that chasing individual spellings leaves
another silent-pass path. The audit therefore constrains this record to the
narrow grammar it actually uses: paragraphs, headings, lists, code blocks,
tables, inline code, and the single-line status-marker HTML comment. All other
tags and leaf events, including raw HTML, images, links, and emphasis, fail as
`UnsupportedConstruct` rather than being interpreted. Unknown syntax now fails
explicitly instead of passing silently.

The syntax allowlist alone was still insufficient because allowed constructs
could be composed into new gaps: a middle H1 moved canonical text out of its H2
section, nested tight-list items reassembled one sentence across item
boundaries, and headings or table cells hid a conflicting lifecycle sentence.
The grammar is therefore narrowed to the record's observed shape—one first-line
H1, allowlisted H2 sections, and unnested lists—and each item boundary flushes
its text independently. The conflicting-status check now covers all rendered
text outside code blocks, including paragraphs and list items, every heading,
and every table cell. With partial collection, every uncollected rendering
context becomes a bypass path.

Complete text collection is still insufficient if block-container ancestry is
discarded. A middle H1 and an H2 inside a list item are two forms of the same
defect: content that renders under a different block hierarchy can be assigned
to one global audit section. Audited headings and tables must therefore begin
at the expected top-level nesting position; a heading or table inside a list
item or table cell is rejected and cannot create or populate a section. The
canonical Decision must likewise come from a top-level paragraph, while list
item text remains visible only to the rendered-text conflict scan.

The schema status records lifecycle truth, not success. `blocked` means no
mutation request was sent, `stopped` means execution began but confirmation
was not observed, and `executed` means confirmation was observed. This keeps
the ambiguous but honest case—where a request was sent and confirmation was
not observed—inside the schema. If status instead represented success, that
case would fit no truthful category and would pressure the recorder to make a
false statement. Post-confirmation balance, allowance, and rollback outcomes
remain in the final-verdict and rollback-state fields rather than changing the
lifecycle status.

Consequences:

- the current record is `blocked` because no operator approval record exists,
  required approved runtime inputs were not supplied, and the runbook's
  tiny-value bounds remain blank;
- the operator-evidence table retains the exact `Field` / `Value` / `Redaction`
  header and exactly 14 ordered fields, and all current Value cells remain
  `UNFILLED`;
- superseding the record requires the marker, canonical Decision sentence,
  conditional sections, and status-specific evidence values to change
  together;
- the offline audit scans only the specified document shapes and limited
  secret patterns, and does not prove prose truth, external-evidence hygiene,
  or live relayer behavior;
- this decision adds only the test-scoped `pulldown-cmark` dev-dependency and
  its transitive `unicase` package; it adds no live call, production permit,
  production-source change, public API change, production dependency, or CI
  live test.

## ADR-0023: PBRSDK-28 Release Provenance And Dependency Audit Contract

Decision:

```text
Disable package publication mechanically, retain an explicit native-TLS
backend when reducing ethers features, and keep the accepted-risk register
exactly synchronized with cargo-audit configuration through an offline audit.
```

A documented publishing prohibition is policy text until the package manifest
enforces it. This fork therefore sets `publish = false`; it remains an internal
path or commit-SHA-pinned dependency rather than a crates.io release candidate.

Dependency feature reduction cannot be judged safe from graph size or passing
tests. The rejected draft used only `default-features = false`, which removed
TLS from ethers-providers' reqwest 0.11 path while all 369 tests still passed
because none opened a socket. Feature removal must be reviewed by asking what
capability disappears and inspecting the feature graph directly. The adopted
manifest pairs `default-features = false` with `features = ["openssl"]`, and
the offline audit structurally rejects the TLS-free draft shape. That manifest
check does not prove the final unified graph or a successful HTTPS connection.

The feature change moves `src/direct.rs` and middleware-internal reqwest 0.11
from rustls/webpki roots to platform native TLS. On Linux this is OpenSSL; on
Windows it is SChannel; on Apple platforms it is Security Framework. TLS
policy, proxy integration, certificate handling, and errors may therefore
change as well as trust anchors. A system CA bundle is newly required for
direct-only consumers that did not already exercise the reqwest 0.12 relayer
path.

An accepted-risk register and its tool configuration are both misleading when
they disagree. `docs/accepted-advisories.toml` and `.cargo/audit.toml` must
contain the same duplicate-free advisory-ID set, and the
`docs/RELEASE_PROVENANCE.md` table must render every canonical field in the
same order.
Because cargo-audit exits successfully for unmaintained notices by default,
both local and scheduled/manual audit execution use
`cargo audit --deny warnings`. The separate workflow is deliberately not a PR
gate: contributor-controlled Cargo configuration must not execute in a job
whose checkout action receives an implicit `GITHUB_TOKEN`. It does not
reference repository secrets, sets checkout `persist-credentials: false`, and
runs only on schedule or trusted manual dispatch. New PR dependency risk is
therefore detected by that workflow on the next scheduled or manual run, not
as a contemporaneous pull-request check.

A document audit must interpret the document the same way it is rendered.
PBRSDK-27 reached that conclusion over six review rounds, but the first
PBRSDK-28 audit returned to hand-written pipe-line matching and reproduced the
same class of bypass: a row hidden inside an HTML comment could authorize a
cargo-audit ignore without appearing in the rendered table. The lesson did not
transfer merely because it existed in another test file. This audit therefore
uses `pulldown-cmark` with tables enabled and accepts advisory rows only from
the parsed table in the parsed `Accepted advisories` H2 section; fenced code
and HTML comments cannot contribute rows.

A parser transition must cover the entire audited document. Parsing only the
table while leaving section and claim checks on raw Markdown preserved those
unchecked representations as bypass paths: a fenced raw H2 could satisfy a
required-section scan, and inline emphasis could split a rendered claim across
raw substrings. PBRSDK-27 had already reached the complete rule of one parsed
view plus a closed syntax allowlist, but the new audit carried over only the
table half and repeated the sequence. The release-provenance audit now derives
headings, table cells, and rendered claim text from one CommonMark pass and
rejects images, links, raw HTML, and other unapproved constructs together with
their descendant text.

An exception becomes a bypass path: the exemption intended to permit a
negated quotation applied to its whole text block and therefore also admitted
an explicit positive claim beside it. The document now avoids the prohibited
phrase, allowing the audit to remove the exception entirely.

An audit of a structured format must use that format's parser. The same defect
class recurred across Markdown, TOML, and YAML in this campaign when raw lines
or substrings were mistaken for rendered or semantic structure. Invisible
format characters add a related boundary: they are not necessarily Unicode
`White_Space`, so `trim` cannot remove them even when rendered output conceals
the difference. A fixed invisible/control denylist was attempted first, but
later counterexamples showed that classifying visual blankness that way cannot
close the boundary. The audit now fixes the complete accepted character set.

Checking whether a value is present gives only a partial guarantee, which
documentation can easily overstate as a complete one. Closed schemas and exact
value comparison now bind the audit config, workflow keys, action commit SHAs,
and audit command to the reviewed contract. Defense is likewise multiple only
when its evidence is independent; two checks calling the same predicate are
one defense with two call sites. The earlier General Category and
Default-Ignorable denylist still could not define visual blankness, so the
closed character alphabet replaces it rather than extending it again.

"Invisible" is not a Unicode property. A denylist was successively bypassed by
U+200B, U+2063, and U+2800, so the provenance contract now closes the character
alphabet to printable ASCII, line feed, and U+2192. New Unicode characters are
rejected until an explicit review widens that set. Exact command text is also
insufficient when a lower interpretation layer can intercept it: a repository
Cargo alias can redefine `cargo audit`. The audit therefore closes `.cargo/`
to the sole reviewed `audit.toml` entry as well as fixing the workflow command.

An audit is not a value-repair layer. Silently removing rejected rendered
characters normalized an ASCII CommonMark character reference into an accepted
advisory ID, making the normalization itself a bypass. The allowlist now checks
both source and parser-rendered text and fails without rewriting either value.
Command text, Cargo aliases, and rustup toolchain selection are distinct
interpretation layers: closing the command and alias layers still left a root
`rust-toolchain.toml` `path` override able to replace the executables. The
toolchain file therefore has its own closed schema and explicit path ban.

A tool cannot bind the bootstrap inputs that determine whether that tool runs.
`rust-toolchain.toml`, repository Cargo configuration, and Cargo automatic
target discovery all act before an integration-test audit can execute; a fake
toolchain, target runner, or disabled `autotests` flag can therefore bypass a
check implemented only inside Cargo. The build-integrity boundary moves outside
Cargo to a file-only Python `tomllib` preflight that must run before the first
Cargo command.

Moving a boundary outward exposes the next interpretation layer. The concrete
sequence in this ticket was command text, Cargo alias, the `.cargo/` directory,
`rust-toolchain.toml`, Cargo bootstrap and target discovery, then Python module
resolution. Python normally searches the script directory before the standard
library, so a repository `tomllib.py` could execute before the preflight. The
workflow now uses isolated mode and separately closes `scripts/`. This sequence
is not treated as infinite: the contract stops where repository-controlled
files no longer participate, at the `python3` interpreter binary selected by
the runner's `PATH`. Recording that stopping point and why it is outside the
repository threat model is part of the contract.

A human-readable format should not be the canonical input when enforcing it
would require two independent interpretations of that format. The earlier
design made both Python and Rust interpret the Markdown advisory table, and
their disagreement recreated the hidden-row bypass already fixed in the first
round. The canonical register is now machine-readable TOML; Python parses that
single source before Cargo, while Rust proves that the CommonMark table is only
an exact ordered rendering. Reducing the canonical interpretation to one
structured format removes the parser disagreement as an authorization path.

The register remains a snapshot because this repository does not track
`Cargo.lock`. Consumer workspaces own authoritative resolution, pinning, audit,
and rollback evidence. ID-wide ignores do not validate current path, version,
reachability, or rationale, so every dependency or advisory change requires a
fresh review rather than relying on set equality alone.
