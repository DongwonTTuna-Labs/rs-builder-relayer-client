# CONSUMER_INTEGRATION.md

## Intended Consumer

The primary consumer is:

```text
DongwonTTuna/polymarket-liquidity-farming-rs
```

The consumer app must use this crate only behind its relayer adapter boundary.

## Allowed Shape

```text
pm-runtime
  constructs DepositWalletRelayerClient

pm-adapters/relayer_http
  wraps this crate and implements RelayerPort

pm-ports
  exposes RelayerPort only

pm-domain / pm-strategy / pm-risk / pm-actors / pm-app
  do not import this crate
```

## Development Dependency

From the consumer repo root:

```toml
# crates/pm-adapters/Cargo.toml
[dependencies]
rs-builder-relayer-client = { path = "../rs-builder-relayer-client" }
```

If the relative path changes, keep it as a sibling repo path. Do not vendor this crate inside the consumer workspace unless explicitly decided.

## Production Dependency

Use a pinned commit:

```toml
# crates/pm-adapters/Cargo.toml
[dependencies]
rs-builder-relayer-client = {
  git = "ssh://git@ssh.dongwontuna.net/DongwonTTuna-Labs/rs-builder-relayer-client.git",
  rev = "<audited_commit_sha>"
}
```

Forbidden:

```toml
rs-builder-relayer-client = { git = "...", branch = "main" }
rs-builder-relayer-client = "0.1"
rs-builder-relayer-client = "*"
rs-builder-relayer-client = { git = "ssh://git@github.com/OrderBookTrade/rs-builder-relayer-client.git", rev = "..." }
```

## Public API Boundary

The consumer app should expose internal domain/application types through `RelayerPort`.

### Reviewed 0.2.0 public integration surface

The reviewed crate-root integration surface is intentionally narrow:

```text
DepositWalletRelayerClient
DepositWalletRelayerUrl
DepositWalletDeploymentPolicy
DepositWalletDeploymentStatus
DepositWalletReadiness
RelayerReadPermit
RelayerPollPolicy
RelayerPollOutcome
CalldataConfigInput
CalldataSourceRef
DepositWalletCalldataConfig
SourcedAddress
polygon_calldata_config
PusdAmount
build_pusd_approval_call
build_ctf_approval_for_all_call
RelayerMutationPermit
RelayerMutationMode
RelayerMutationOperation
RelayerSubmitOutcome
MutationIntentStore
InMemoryMutationIntentStore
MutationIntentAuditArtifact
MutationIntentRecord
MutationIntentStatus
TryBeginOutcome
OwnerMutationRegistry
MutationIntentLease
IntentGatedClient
ReconciliationSummary
MUTATION_AUDIT_ARTIFACT_SCHEMA_VERSION
DepositWalletDryRunEvidence
DepositWalletSubmitReceipt
DryRunCallSummary
DepositWalletCall
RelayerKeyAuth
DepositWalletRequestContext
DepositWalletCreateRequest
WalletNonceRequest
RelayerTransactionState
SignedDepositWalletBatch
RelayerSubmitResponse
DepositWalletTransactionReceipt
derive_deposit_wallet_address
deposit_wallet_contract_config
build_wallet_create_request
build_wallet_nonce_request
try_build_wallet_batch_request_with_signature
```

The consumer app must map these into its own port types and must not leak this
crate's DTOs into strategy, risk, actor state, or domain types.

`DepositWalletBatchRequest` is a validated submit-body output type, not a public
construction surface. It is intentionally not re-exported from the crate root;
its submit-body fields remain crate-private. Consumers that need to submit a
WALLET batch must obtain it from
`try_build_wallet_batch_request_with_signature` or from the validated
`SignedDepositWalletBatch` flow inside the relayer adapter boundary.

Legacy Safe/Proxy APIs such as `RelayClient`, `AuthMethod`, `DirectExecutor`,
and `operations::*` remain available as upstream compatibility/reference
surface. They are not the deposit-wallet `WALLET-CREATE` or `WALLET`
implementation path and must not be reused implicitly for deposit-wallet
production flows.

WALLET submit request construction must use the fallible
`try_build_wallet_batch_request_with_signature` API or the validated
`SignedDepositWalletBatch` flow. The old infallible
`build_wallet_batch_request_with_signature` helper is intentionally not part of
the public integration surface because it cannot report signer, config,
derived-wallet, signature-shape, or resource-limit failures.

This is a `0.2.0` breaking migration boundary. Consumer adapters that still
import or call `build_wallet_batch_request_with_signature` must switch to
`try_build_wallet_batch_request_with_signature`, propagate `RelayerError`, and
keep the error handling inside the relayer adapter rather than domain or
strategy layers. Raw `DepositWalletBatchRequest` construction is not a public
crate-root API; request DTO fields stay crate-private so submit bodies are
produced through validated builders.

CLOB order/sign/cancel/post behavior remains out of this crate. Consumers must
keep CLOB trading, cancellation, balance/orderbook reads, and order-posting
logic in the official Polymarket Rust CLOB SDK adapter rather than importing or
adding CLOB modules, examples, or order APIs here.

Boundary audit evidence for PBRSDK-4:

```bash
cargo test --test public_api_boundary_test
cargo doc --workspace --all-features --no-deps
grep -R "pub use .*::\\*\\|pub mod clob\\|pub use clob\\|build_wallet_batch_request_with_signature\\|DepositWalletBatchRequest" -n src tests docs README.md
```

### Verified calldata configuration (PBRSDK-17)

Calldata builders receive a validated config rather than raw contract targets
or global defaults. Normal Polygon consumers should use the canonical
constructor and inspect only its getters:

```rust
use ethers::types::Address;
use polymarket_relayer::{polygon_calldata_config, Result};

fn reviewed_approval_targets(
    spender: Address,
    operator: Address,
) -> Result<(bool, bool)> {
    let config = polygon_calldata_config()?;

    Ok((
        config.is_allowed_pusd_spender(spender),
        config.is_allowed_ctf_operator(operator),
    ))
}
```

`CalldataConfigInput` exists for explicit strict-subset construction and future
reviewed extension. It is not validated until
`DepositWalletCalldataConfig::try_new` succeeds and must not cross the consumer
adapter into domain, strategy, risk, or actor state. PBRSDK-17 supports only
Polygon `137`, preserves strict allowlist subsets, and exposes an empty adapter
allowlist. The PBRSDK-17 config boundary itself performs no ABI encoding or HTTP
work. PBRSDK-18 supplies the approval encoders described below, while PBRSDK-19
still owns adapter-route verification. Config serialization is for offline
review only. There is no `Deserialize`, environment, or file-loading path.

### Unit-safe approval calldata builders (PBRSDK-18)

Construct pUSD amounts through `PusdAmount`; the builders do not accept a raw
`u64` or `U256`. Both calls use the target and narrowed allowlist from the
specific config passed by the caller:

```rust
use ethers::types::Address;
use polymarket_relayer::{
    build_ctf_approval_for_all_call, build_pusd_approval_call,
    polygon_calldata_config, DepositWalletCall, PusdAmount, Result,
};

fn approval_calls(
    pusd_spender: Address,
    ctf_operator: Address,
) -> Result<(DepositWalletCall, DepositWalletCall)> {
    let config = polygon_calldata_config()?;
    let amount = PusdAmount::from_whole_pusd(1)?;
    let pusd = build_pusd_approval_call(&config, pusd_spender, amount)?;
    let ctf = build_ctf_approval_for_all_call(&config, ctf_operator, true)?;

    Ok((pusd, ctf))
}
```

`PusdAmount::from_base_units` and `from_whole_pusd` reject zero.
`PusdAmount::unlimited()` is an explicit representation of `uint256::MAX`, not
an unlimited-approval policy decision. CTF approval revocation is expressed by
passing `false`; the operator must still be in the supplied config's verified
allowlist. These functions only return `DepositWalletCall` values. They do not
compose a batch, fetch a nonce, sign, submit, or authorize live execution.

### HTTP client surface

The deposit-wallet HTTP client exposes construction, exactly three reviewed
low-level production reads, two bounded transaction-polling methods, two
deployment-lifecycle orchestration methods, and exactly two permit-gated
primitive `submit_*` methods. `IntentGatedClient` adds one wrapper for each
live mutation entry without changing the existing client signatures:

```text
DepositWalletRelayerUrl::parse
DepositWalletRelayerClient::new
DepositWalletRelayerClient::new_with_mutation_enabled
DepositWalletRelayerClient::disable_mutation
DepositWalletRelayerClient::is_deposit_wallet_deployed
DepositWalletRelayerClient::get_wallet_nonce
DepositWalletRelayerClient::get_transaction_for_owner
DepositWalletRelayerClient::poll_wallet_transaction
DepositWalletRelayerClient::poll_deposit_wallet_deployment
DepositWalletRelayerClient::ensure_deposit_wallet_deployment
DepositWalletRelayerClient::check_deposit_wallet_deployment_readiness
DepositWalletRelayerClient::execute_wallet_batch
DepositWalletRelayerClient::submit_wallet_create
DepositWalletRelayerClient::submit_signed_wallet_batch
OwnerMutationRegistry::gate
IntentGatedClient::execute_wallet_batch
IntentGatedClient::submit_wallet_create
IntentGatedClient::ensure_deposit_wallet_deployment
IntentGatedClient::reconcile_by_polling
IntentGatedClient::report_ambiguous_candidates
OwnerMutationRegistry::adopt_transaction
OwnerMutationRegistry::reconcile_manually
OwnerMutationRegistry::export_audit_artifact
```

Each read requires a `RelayerReadPermit` whose owner equals the requested owner
and whose chain id matches the client's deposit-wallet contract config. Create
one at the relayer adapter boundary and pass the same owner to the read:

```rust
use polymarket_relayer::RelayerReadPermit;

let permit = RelayerReadPermit::for_owner(owner, 137);

let deployed = client
    .is_deposit_wallet_deployed(owner, &permit)
    .await?;
let nonce = client.get_wallet_nonce(owner, &permit).await?;
let transaction = client
    .get_transaction_for_owner(owner, transaction_id, &permit)
    .await?;
```

The read permit intentionally has no expiry because it grants only idempotent
reads. It does not grant submit authority, reserve the nonce for signing, or
replace `RelayerMutationPermit`. `POST /submit` is reachable only through the
two mutation methods with a matching mutation permit. The additive
`GET /transactions` path is reachable only through the unresolved-intent report
method after read-permit validation; it is read-only evidence discovery and
does not adopt a candidate or release the owner.

Transaction reads preserve the recorded PBRSDK-2 evidence contract: the
response must be a `WALLET` transaction, `owner` must be present, `from` must
equal `owner`, `to` must equal the configured deposit-wallet factory, and
`proxyAddress` must equal the wallet derived from `owner` and the configured
contract config. `WALLET-CREATE` responses are not accepted as WALLET owner
evidence because deployment identity and wallet mutation identity are reviewed
separately.

### Owner-scoped mutation intent wiring

Live consumer wiring must provide one durable `MutationIntentStore` and one
`OwnerMutationRegistry` shared by every relayer adapter actor that can mutate
the same owner. The synchronous store methods must be fast local operations.
A DB-backed implementation must make `try_begin` and `update` transactional or
CAS-based, and must hide blocking I/O behind the consumer's blocking boundary.

```rust
use std::sync::Arc;

use polymarket_relayer::{MutationIntentStore, OwnerMutationRegistry};

// Adapter-owned durable implementation; not supplied by this crate.
let store: Arc<dyn MutationIntentStore> =
    Arc::new(ConsumerDurableMutationIntentStore::open(database)?);
let registry = OwnerMutationRegistry::new(store);
let gated = registry.gate(&client);
```

Use `gated.execute_wallet_batch`, `gated.submit_wallet_create`, or
`gated.ensure_deposit_wallet_deployment` for every live entry. The registry
writes Preparing before nonce, signing, or submit I/O. Preparing, Submitted,
and AmbiguousNoId reject a second lease for the same `(owner, chain_id)`;
different owners and chains remain independent. DryRun delegates without a
lease. The original client methods remain additive compatibility primitives,
not the approved live consumer entry after PBRSDK-12.

Prefer `gated.reconcile_by_polling` for a Submitted intent. It loads the stored
transaction id, selects the WALLET or WALLET-CREATE expected-type poll from the
stored operation, and records only the existing transaction-bound outcome.
Confirmed may resolve the intent only when the receipt also says Confirmed and
carries the same transaction id. Failed/Invalid resolve through the existing
bound terminal-failure API. Exhausted stores only a present pending label;
Cancelled is a no-op. Other errors and unknown/reconciliation-required evidence
leave the owner blocked.

`MutationIntentRecord` stores only owner/chain identity, generation/revision,
operation/status, an optional decimal nonce placeholder, payload hash,
deadline, transaction id, safe observed-state label, cumulative poll attempts,
optional reconciliation evidence, and timestamps. It has no signature, auth header, private-key,
calldata, or replayable-body field. Record Debug redacts owner, hashes
transaction ids, and prints evidence text lengths rather than text. Dropping
`MutationIntentLease` never deletes or resolves the record.

### Redacted mutation audit artifact

Export a ticket-ready snapshot with the registry, not by serializing the
durable record directly:

```rust
let artifact = registry.export_audit_artifact(owner, chain_id)?;
let attachment_json = serde_json::to_string_pretty(&artifact)?;
attach_to_operator_ticket(attachment_json)?;
```

Export is a pure read and accepts Preparing, Submitted, AmbiguousNoId,
Confirmed, Failed, and Reconciled rows. The owner is shortened, an unrecognized
stored state is replaced with `<unrecognized relayer state>`, and manual
reconciliation text is reduced to decision, UTF-8 byte lengths, and recorded
time. `operator_ref` and `summary` are never present in the artifact even when
the durable row was restored through Deserialize. Operators who need those
original strings must inspect their protected store record; do not attach that
record in place of the redacted artifact.

JSON retains the transaction id for later adoption/polling. Artifact Debug
replaces it with a `sha3:0x...` token. Nonce is allowed in the artifact but is
intentionally absent from mutation-intent tracing. `poll_attempts` counts a
matching Confirmed observation as one and adds Exhausted attempts only when a
last state was present. Older records without that field export zero.

Sample schema-v1 redacted artifact:

```json
{
  "schema_version": 1,
  "owner": "0x6e0c...B5b5",
  "chain_id": 137,
  "operation": "WalletBatch",
  "epoch": 4,
  "revision": 5,
  "status": "Confirmed",
  "nonce": null,
  "payload_keccak256": "0x1111111111111111111111111111111111111111111111111111111111111111",
  "deadline_unix": 1760000000,
  "transaction_id": "tx-reviewed-don-61",
  "last_observed_state": "Confirmed",
  "poll_attempts": 4,
  "reconciliation": {
    "decision": "ConfirmedOnChain",
    "operator_ref_len": 15,
    "summary_len": 30,
    "recorded_at_unix": 1760000001
  },
  "created_at_unix": 1760000000,
  "updated_at_unix": 1760000004,
  "redaction": "api keys, auth headers, private keys, signatures, typed data, and full submit bodies are intentionally omitted"
}
```

The pre-submit `DepositWalletDryRunEvidence` remains the source for selector
and call summaries. The mutation audit artifact deliberately omits them;
`payload_keccak256` is the correlation key between the two artifacts.

`InMemoryMutationIntentStore` loses all records on restart and is restricted to
tests and development. It must never protect live traffic. A durable store is
the restart contract: unresolved rows survive and block new mutation until a
bound terminal result or later authoritative reconciliation resolves them.
PBRSDK-24/25 live gates require that durable wiring.

### Ambiguous reconciliation runbook

Use this order for every blocked owner. None of these steps grants submit
authority by itself, and no step automatically retries the original submit.

1. Inspect `registry.intent(owner, chain_id)` and retain its `epoch`. If the
   intent is Submitted with a transaction id, call
   `gated.reconcile_by_polling` first. Confirmed or bound Failed/Invalid is
   authoritative; StillPending, Cancelled, unknown, and every other error keep
   the owner blocked.
2. If the intent has no transaction id, call
   `gated.report_ambiguous_candidates(owner, &read_permit)`. The report contains
   the inspected epoch, redacted intent metadata, validated candidates, and a
   skipped-item count. It is a read-only review artifact, not a match verdict.
3. After independent venue/on-chain review, create a fresh
   `ReconciliationEvidence`. If the operator has identified the original
   transaction id, call `registry.adopt_transaction` with the report epoch and
   then return to step 1. Adoption is accepted only from AmbiguousNoId.
4. If the operator instead has evidence that the original request was not
   accepted, was already confirmed through another authoritative observation,
   or was superseded, call `registry.reconcile_manually` with the inspected
   epoch and the corresponding decision. Only this explicit evidence-bearing
   transition permits a later `begin_intent`.

Every manual call is epoch-fenced. A generation-changed or concurrent-update
error requires a new `intent()` inspection and new operator decision; never
reuse stale evidence blindly. Candidate type, timestamp, state, payload hash,
or proximity is never sufficient for automatic adoption.

**Preparing warning:** `reconcile_manually` on Preparing exists only for
restart recovery. A living process may still have an in-flight submit tied to
that row. Before reconciling Preparing, the operator must prove that no work for
that owner is still running; otherwise manual release can race the original
submission.

### Bounded confirmed-only polling

Use `RelayerPollPolicy` to continue observing a known transaction id without
creating submit authority. The policy is explicit and finite; it doubles the
interval until the configured cap:

```rust
use std::time::Duration;

use polymarket_relayer::{RelayerPollOutcome, RelayerPollPolicy};

let policy = RelayerPollPolicy::try_new(
    3,
    Duration::from_secs(1),
    Duration::from_secs(60),
)?;

match client
    .poll_wallet_transaction(
        owner,
        transaction_id,
        policy,
        &permit,
        std::future::pending::<()>(),
    )
    .await?
{
    RelayerPollOutcome::Confirmed(receipt) => {
        // This is the only outcome that authorizes reliance on wallet effects.
        persist_confirmed_receipt(receipt)?;
    }
    RelayerPollOutcome::Exhausted { attempts, last_state } => {
        persist_pending_poll(attempts, last_state)?;
        // Reconcile the original transaction; do not sign or submit again.
    }
    RelayerPollOutcome::Cancelled { attempts } => {
        persist_cancelled_poll(attempts)?;
        // Cancellation is not evidence that the original request is absent.
    }
}
```

Use `poll_deposit_wallet_deployment` for the transaction id returned by
WALLET-CREATE. It applies the same result policy but requires WALLET-CREATE
wire evidence; the WALLET method and deployment method reject each other's
transaction type. A caller with its own shutdown signal may pass that future
instead of `std::future::pending::<()>()`.

New, Executed, and Mined remain pending. Confirmed alone returns
`RelayerPollOutcome::Confirmed`; Failed, Invalid, Unknown, wrong-type, permit,
and ambiguous evidence stop as errors. HTTP transport errors, all API statuses,
and a transaction temporarily absent from an array are retried only within the
finite policy. The schedule intentionally ignores `Retry-After`; a 4xx auth
error can therefore consume the bounded attempts and end as `Exhausted`.
Preserve transport telemetry for operator diagnosis.

Polling calls only the existing verified `GET /transaction` path. It never
calls submit, signs, fetches a nonce, queries `GET /transactions`, or infers
that exhaustion/cancellation permits another mutation. The separate recent
report is operator evidence discovery only and never replaces polling.

`is_deposit_wallet_deployed` derives the deposit-wallet address from the owner
and sends that derived address to `GET /deployed?address=...&type=WALLET`. A
`true` response records deployment fact only. It is not submit readiness and
must not bypass the separate `STATE_CONFIRMED`, mutation capability, recovery,
or operator gates.

### Fresh-nonce WALLET batch execution

Prefer `execute_wallet_batch` when connecting an owner signer to the reviewed
WALLET batch flow. The signer is a method argument, not client state, so
relayer auth identity remains independent from the owner signer and the
derived deposit wallet/funder:

```rust
use ethers::signers::{LocalWallet, Signer};
use ethers::types::U256;
use polymarket_relayer::{
    derive_deposit_wallet_address, DepositWalletRequestContext,
};

// Documentation-only synthetic throwaway key; never use it for real assets,
// production credentials, or a funded wallet.
let signer = LocalWallet::from_bytes(&[0x42u8; 32])?.with_chain_id(137u64);
let owner = signer.address();
let ctx = DepositWalletRequestContext {
    owner_address: owner,
    deposit_wallet_address: derive_deposit_wallet_address(owner, config)?,
};

let outcome = registry
    .gate(&client)
    .execute_wallet_batch(
        ctx,
        calls,
        U256::from(deadline_unix),
        &signer,
        &read_permit,
        &mutation_permit,
    )
    .await?;
```

The read and mutation permits must both target `owner` and the configured
chain; the mutation permit operation must be `WalletBatch`. The method rejects
an expired deadline, mismatched signer, wrong derived wallet, or oversized
batch before nonce I/O. It then performs `GET /nonce?address=<owner>&type=WALLET`
immediately before local EIP-712 signing, revalidates signer recovery and all
request identities, and delegates to `submit_signed_wallet_batch`.

DryRun follows the same path through the fresh nonce read and local signature,
then returns redacted evidence without a submit HTTP request. A closed live
latch blocks the POST but intentionally does not retroactively block the nonce
read. Preserve a submitted transaction id and payload hash, and never retry an
ambiguous submission. The registry rejects a second same-owner/chain live
execution before nonce I/O while Preparing, Submitted, or AmbiguousNoId is
unresolved. The nonce field remains `None` in this wrapper round because the
opaque primitive does not expose the fetched nonce; PBRSDK-13 does not change
that limitation, and a future provenance hook must preserve this public
wrapper.

### Deployment lifecycle workflow

`DepositWalletDeploymentPolicy` is always an explicit method argument and does
not implement `Default`. The normal consumer policy is `Predeployed`: it first
performs the deployed read and blocks WALLET-CREATE when the wallet is missing.
Use `DeployIfMissing` only in a runtime that explicitly owns wallet
provisioning and has a scoped mutation permit.

The consumer adapter must keep this order:

```text
choose Predeployed unless this runtime explicitly owns deployment
  -> call registry.gate(&client).ensure_deposit_wallet_deployment
  -> AlreadyDeployed: record observed deployment fact; no submit occurred
  -> CreateDryRun: retain redacted evidence; no submit occurred and wallet is
     not ready
  -> CreateSubmitted: persist transaction_id and payload_keccak256 immediately
  -> call check_deposit_wallet_deployment_readiness once
  -> Ready: STATE_CONFIRMED with required hash evidence was observed
  -> Pending(New/Executed/Mined): call poll_deposit_wallet_deployment with the
     original transaction id and do not call ensure_deposit_wallet_deployment
     again for that owner
  -> any error: stop mutation and reconcile; never infer authority to resubmit
```

The deployed preflight owns read-permit, derived-wallet, configured factory,
chain, and source validation. If deployment is missing,
`submit_wallet_create` owns mutation operation, owner, chain, expiry, latch,
mode, and request validation. The orchestration layer intentionally does not
duplicate those checks. `WALLET-CREATE` serialization remains the approved
`type`/`from`/factory-`to` body and contains no user signature.

Readiness accepts only a transaction response whose type is `WALLET-CREATE`.
The public `get_transaction_for_owner` remains WALLET-only, so neither response
type can cross the other lifecycle. Confirmed is the only `Ready` state. Failed
and Invalid remain typed errors; Unknown, mismatched type, malformed evidence,
absence, and ambiguous transport/results require reconciliation.

PBRSDK-8 intentionally performs only one readiness read. PBRSDK-10 now provides
the bounded continuation above, and PBRSDK-12 persists the submitted owner
intent through the consumer's durable store. Neither layer queries recent
transactions, reconciles automatically, or submits again. `Exhausted`,
`Cancelled`, unknown, or ambiguous evidence does not release the registry
guard; use the PBRSDK-13 runbook above for authoritative or evidence-bound
reconciliation.

Relayer auth wire evidence is anchored to the official Polymarket relayer docs:

```text
https://docs.polymarket.com/api-reference/relayer/get-a-transaction-by-id
https://docs.polymarket.com/trading/gasless
https://docs.polymarket.com/api-reference/relayer-api-keys/get-all-relayer-api-keys
```

Those docs name `RELAYER_API_KEY` and `RELAYER_API_KEY_ADDRESS` as the Relayer
API key auth headers and define `RELAYER_API_KEY_ADDRESS` as the address that
owns the key. This HTTP read client sends those headers on read requests as
credential identity, while still treating the permit owner and transaction
`owner`/`from` evidence as a separate owner-bound contract. Consumers must not
assume the relayer API key address, owner signer, deposit wallet, or funder are
the same identity.

Migration path: consumer adapters may add `RelayerReadPermit` at their relayer
adapter boundary and map the three read results into local port types. Do not
leak the permit or HTTP receipt DTO into domain, strategy, risk, or actor state.
Rollback path: stop calling the three read methods and retain the existing
fixture/signing-only integration; no consumer domain type should depend on the
HTTP DTOs or permit.

### Explicit mutation workflow and rollback

`DepositWalletRelayerClient::new` is the normal default-deny constructor. Its
three low-level reads, two bounded polling methods, and valid `DryRun`
submissions work, but every `Live` submission is blocked.
`new_with_mutation_enabled` explicitly starts the live latch enabled;
that constructor is necessary but not sufficient because each submit still
requires a matching, unexpired `Live` permit.

The consumer adapter must keep this order:

```text
create an operation/owner/chain/expiry-scoped DryRun permit
  -> call the matching intent-gated method and retain redacted DryRun evidence
  -> operator reviews that exact evidence and records approval
  -> create a fresh Live permit for the reviewed operation/owner/chain with a
     fresh expiry and the reviewed evidence/completed-approval references
  -> use OwnerMutationRegistry::gate with an explicitly enabled client for the
     one reviewed live submit
```

A `DryRun` permit is not upgraded or reused as live authority. Dry-run request
validation, scope/expiry checks, and batch deadline checks still apply, but it
does not consult the live latch and sends no HTTP request. The evidence is safe
for operator review because it contains a payload hash and bounded summaries
while omitting auth headers, signatures, full calldata, and the full replayable
submit body. The initial permit's operator-approval reference may point to the
pending review record; the fresh `Live` permit must point to the completed
approval record. Both references must be non-secret identifiers: dry-run
evidence getters, JSON serialization, and Debug output intentionally retain
their original trimmed values even though permit Debug prints only their
lengths.

For rollback, call `disable_mutation` on the enabled client and stop issuing
new live permits. The one-way latch is shared by that client and all clones, so
all later live submissions through them fail. Owner-scoped reads and valid
`DryRun` evidence generation continue after the latch. There is no re-enable
method; resuming live work requires a newly constructed enabled client, a new
operator review decision, and a freshly created `Live` permit. Preserve any
submit receipt, payload hash, or reconciliation-required error before dropping
the old client. The latch does not cancel a live submit that already passed the
gate or recall an in-flight POST; reconcile any such request before taking
further mutation action.

Invalid or partial success responses, transport failures, and oversized 2xx
responses require reconciliation before any new submit. A submit receipt is
not `STATE_CONFIRMED`; PBRSDK-8 adds a single-shot readiness check and PBRSDK-10
adds bounded confirmed-only polling. PBRSDK-12 adds the owner-scoped durable
store boundary and lease fencing, and PBRSDK-13 adds evidence-bound recent
lookup, manual adoption/reconciliation, and the stored-id polling wrapper.
Automatic candidate selection, automatic resubmit, deterministic concurrency
qualification, and durable-store live qualification remain absent, so these
additions do not claim end-to-end live readiness.

Consumer adapter migration status for PR #8:

- this crate PR records the required `0.2.0` migration boundary and rollback
  path, but it does not claim the downstream consumer adapter has already been
  migrated;
- any consumer pinning this PR must update its relayer adapter call sites from
  `build_wallet_batch_request_with_signature` to
  `try_build_wallet_batch_request_with_signature` in the same consumer-side
  integration change;
- live submit enablement remains blocked by the enablement rule below, so a
  consumer adapter that has not completed this migration must not treat this PR
  as live-submit capable.

## Enablement Rule

Consumer live relayer mutation remains disabled until:

```text
WALLET-CREATE fixture tests pass
WALLET fixture tests pass
EIP-712 fixture tests pass
pUSD/CTF calldata fixture tests pass
identity separation tests pass
bounded transaction polling timing, cancellation, exhaustion, and unknown-state tests pass
deployment lifecycle short-circuit, policy, permit, and type-isolation tests pass
fresh-nonce WALLET execute ordering, identity, redaction, and failure-boundary tests pass
durable owner mutation intent store is wired and restart recovery is exercised
same-owner unresolved mutation is rejected before nonce/signing/HTTP
known-id reconciliation polls the stored id without any POST /submit
id-less reconciliation records a redacted report and epoch-fenced operator evidence
candidate adoption is manual, evidence-bound, and followed by authoritative polling
dependency is pinned by commit SHA
operator approval is recorded
```
