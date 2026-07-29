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
RelayerMutationPermit
RelayerMutationMode
RelayerMutationOperation
RelayerSubmitOutcome
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

### HTTP client surface

The deposit-wallet HTTP client exposes construction, exactly three reviewed
low-level production reads, two deployment-lifecycle orchestration methods,
and exactly two permit-gated public `submit_*` methods:

```text
DepositWalletRelayerUrl::parse
DepositWalletRelayerClient::new
DepositWalletRelayerClient::new_with_mutation_enabled
DepositWalletRelayerClient::disable_mutation
DepositWalletRelayerClient::is_deposit_wallet_deployed
DepositWalletRelayerClient::get_wallet_nonce
DepositWalletRelayerClient::get_transaction_for_owner
DepositWalletRelayerClient::ensure_deposit_wallet_deployment
DepositWalletRelayerClient::check_deposit_wallet_deployment_readiness
DepositWalletRelayerClient::execute_wallet_batch
DepositWalletRelayerClient::submit_wallet_create
DepositWalletRelayerClient::submit_signed_wallet_batch
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
two mutation methods with a matching mutation permit, and `GET /transactions`
remains deferred to PBRSDK-13 reconciliation work.

Transaction reads preserve the recorded PBRSDK-2 evidence contract: the
response must be a `WALLET` transaction, `owner` must be present, `from` must
equal `owner`, `to` must equal the configured deposit-wallet factory, and
`proxyAddress` must equal the wallet derived from `owner` and the configured
contract config. `WALLET-CREATE` responses are not accepted as WALLET owner
evidence because deployment identity and wallet mutation identity are reviewed
separately.

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

let outcome = client
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
ambiguous submission. Until PBRSDK-11/12 adds the owner-scoped nonce lease and
intent contract, the consumer adapter must serialize execution per owner and
must not call this method concurrently for the same owner.

### Deployment lifecycle workflow

`DepositWalletDeploymentPolicy` is always an explicit method argument and does
not implement `Default`. The normal consumer policy is `Predeployed`: it first
performs the deployed read and blocks WALLET-CREATE when the wallet is missing.
Use `DeployIfMissing` only in a runtime that explicitly owns wallet
provisioning and has a scoped mutation permit.

The consumer adapter must keep this order:

```text
choose Predeployed unless this runtime explicitly owns deployment
  -> call ensure_deposit_wallet_deployment
  -> AlreadyDeployed: record observed deployment fact; no submit occurred
  -> CreateDryRun: retain redacted evidence; no submit occurred and wallet is
     not ready
  -> CreateSubmitted: persist transaction_id and payload_keccak256 immediately
  -> call check_deposit_wallet_deployment_readiness once
  -> Ready: STATE_CONFIRMED with required hash evidence was observed
  -> Pending(New/Executed/Mined): hand off to PBRSDK-10 polling and do not call
     ensure_deposit_wallet_deployment again for that owner
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

PBRSDK-8 intentionally performs only one readiness read. It does not loop,
sleep, retry, cancel, query recent transactions, persist owner state, or submit
again. Bounded polling is PBRSDK-10, owner-scoped pending-intent enforcement is
PBRSDK-11, and later reconciliation persistence belongs to PBRSDK-12/13. Until
those controls exist, a consumer must use its adapter-owned state to forbid a
second lifecycle entry for an owner with a pending create.

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
three reads and valid `DryRun` submissions work, but every `Live` submission is
blocked. `new_with_mutation_enabled` explicitly starts the live latch enabled;
that constructor is necessary but not sufficient because each submit still
requires a matching, unexpired `Live` permit.

The consumer adapter must keep this order:

```text
create an operation/owner/chain/expiry-scoped DryRun permit
  -> call the matching submit method and retain redacted DryRun evidence
  -> operator reviews that exact evidence and records approval
  -> create a fresh Live permit for the reviewed operation/owner/chain with a
     fresh expiry and the reviewed evidence/completed-approval references
  -> use an explicitly enabled client for the one reviewed live submit
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
not `STATE_CONFIRMED`; PBRSDK-8 adds only a single-shot readiness check and does
not provide the complete polling, persistent idempotency, recent-transaction
lookup, or duplicate-submit recovery needed to claim end-to-end live readiness.

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
transaction polling unknown-state tests pass
deployment lifecycle short-circuit, policy, permit, and type-isolation tests pass
fresh-nonce WALLET execute ordering, identity, redaction, and failure-boundary tests pass
dependency is pinned by commit SHA
operator approval is recorded
```
