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

This crate may expose:

```text
DepositWalletRelayerClient
DepositWalletRelayerUrl
DepositWalletCall
RelayerKeyAuth
DepositWalletRequestContext
SignedDepositWalletBatch
RelayerSubmitResponse
RelayerTransactionStatus
DepositWalletTransactionReceipt
```

The consumer app must map these into its own port types and must not leak this crate's DTOs into strategy, risk, actor state, or domain types.

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

### HTTP read surface

The deposit-wallet HTTP client public surface exposes read-only construction and
owner-bound transaction reads for adapter use:

```text
DepositWalletRelayerUrl::parse
DepositWalletRelayerClient::new
DepositWalletRelayerClient::get_transaction_for_owner
DepositWalletRelayerClient::get_wallet_nonce
```

`get_transaction_for_owner` is the owner-bound transaction read API in this
layer, but production URLs reject it in this PR. The current official
`GET /transaction` reference documents `SAFE`/`PROXY` transaction types, while
the deposit-wallet docs describe `WALLET` submit/body construction without
documenting the polling response shape. Until an official or recorded `WALLET`
polling response fixture is reviewed, this crate must not claim production
deposit-wallet transaction polling compatibility. Local loopback and recorded
fixture tests still require relayer wire evidence that the response is a
`WALLET` transaction, that `owner` is present, that `from == owner`, and that
`proxyAddress` matches the deposit wallet address derived from the response
owner and reviewed contract config. `WALLET-CREATE` responses are not treated as
WALLET owner evidence by this parser because deployment identity and wallet
mutation identity are reviewed separately. Recorded transaction fixtures whose
`proxyAddress` cannot be derived from the owner remain reconciliation-required
instead of production compatibility evidence.

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

`get_wallet_nonce` is public only for diagnostics and local loopback tests in
this layer. Production URLs reject WALLET nonce reads until the mutation-state
stack owns a nonce lease from nonce fetch through signing and submit. Consumers
must not treat this PR as live nonce, submit, or recovery capable.

Migration path: consumer adapters may wrap the read-only client behind
`RelayerPort` transaction status reads while keeping all mutation calls disabled.
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
