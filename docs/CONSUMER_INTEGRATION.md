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
DepositWalletCall
RelayerKeyAuth
DepositWalletRequestContext
SignedDepositWalletBatch
RelayerSubmitResponse
RelayerTransactionStatus
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

### RelayerError matching in 0.2.x

The deposit-wallet HTTP client preserves the existing public `RelayerError`
enum shape. It does not add public URL-validation, mutation-gating,
ambiguous-submit, or reconciliation variants in this PR. New deposit-wallet
guard failures are surfaced through `RelayerError::Other` with stable
deposit-wallet prefixes:

```text
Invalid relayer URL:
Deposit-wallet mutation blocked:
Ambiguous deposit-wallet submit:
Deposit-wallet reconciliation required:
```

Read-only relayer quota exhaustion continues to use the existing
`RelayerError::QuotaExhausted` unit variant. Deposit-wallet `POST /submit`
mutations are stricter: a 429 after the POST boundary is treated as an
ambiguous submit, keeps the owner blocked, and requires manual reconciliation
before retrying owner nonce/sign/submit work.

Rollback path: if a consumer adapter is not ready to handle these
deposit-wallet-prefixed errors, it must stay pinned to the previous commit SHA
and keep live deposit-wallet mutation disabled. This PR does not require a
consumer to enable live submit.

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
