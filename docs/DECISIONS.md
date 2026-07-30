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
