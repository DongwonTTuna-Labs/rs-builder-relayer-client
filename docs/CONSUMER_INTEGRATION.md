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

The deposit-wallet HTTP client public surface in this PR exposes construction
only:

```text
DepositWalletRelayerUrl::parse
DepositWalletRelayerClient::new
```

Transaction and nonce read helpers remain crate-internal in this PR. The current
official `GET /transaction` reference documents `SAFE`/`PROXY` transaction
types, while the deposit-wallet docs describe `WALLET` submit/body construction
without documenting the polling response shape. Until an official or recorded
`WALLET` polling response fixture is reviewed, this crate must not claim
production deposit-wallet transaction polling compatibility. Local loopback and
recorded fixture tests preserve relayer wire evidence that the response is a
`WALLET` transaction, that `owner` is present, that `from == owner`, and that
`to` matches the configured deposit-wallet factory, and that `proxyAddress`
matches the deposit wallet derived from `owner` and the configured factory.
`WALLET-CREATE` responses are not treated as WALLET owner evidence by this parser because
deployment identity and wallet mutation identity are reviewed separately.

Relayer auth wire evidence is anchored to the official Polymarket relayer docs:

```text
https://docs.polymarket.com/api-reference/relayer/get-a-transaction-by-id
https://docs.polymarket.com/trading/gasless
https://docs.polymarket.com/api-reference/relayer-api-keys/get-all-relayer-api-keys
```

Those docs name `RELAYER_API_KEY` and `RELAYER_API_KEY_ADDRESS` as the Relayer
API key auth headers and define `RELAYER_API_KEY_ADDRESS` as the address that
owns the key. This HTTP read client sends those headers on read requests as
credential identity, while still treating transaction `owner`/`from` evidence
as a separate owner-bound response contract. Consumers must not assume the
relayer API key address, owner signer, deposit wallet, or funder are the same
identity.

WALLET nonce reads also remain crate-internal in this layer. Production URLs
reject nonce reads until the mutation-state stack owns a nonce lease from nonce
fetch through signing and submit. Consumers must not treat this PR as live
nonce, submit, polling, or recovery capable.

Migration path: consumer adapters may construct the client behind their adapter
boundary, but must not expose transaction status or nonce reads until a later PR
adds reviewed polling evidence and owner-scoped nonce lease semantics.
Rollback path: stop importing the HTTP read client and keep the existing
fixture/signing-only integration; no consumer domain type should depend on the
new HTTP DTOs.

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
dependency is pinned by commit SHA
operator approval is recorded
```
