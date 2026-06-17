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

### HTTP client and live-capable surface

The deposit-wallet HTTP client public surface now includes the production
building blocks needed by the Amoy live smoke:

```text
DepositWalletRelayerUrl::parse
DepositWalletRelayerClient::new
DepositWalletRelayerClient::discover_deposit_wallet
DepositWalletRelayerClient::get_wallet_nonce
DepositWalletRelayerClient::submit_wallet_create
DepositWalletRelayerClient::submit_signed_wallet_batch
DepositWalletRelayerClient::get_transaction_for_owner
DepositWalletRelayerClient::poll_transaction_for_owner
DepositWalletRelayerClient::poll_transaction_for_owner_with_config
```

This is not a mainnet production readiness claim. Submit is hard-gated to the
validated Amoy staging relayer host and chain `80002`; Polygon/mainnet submit is
refused by the client. Consumers must keep the crate behind their relayer adapter
boundary and must not expose these DTOs into domain, strategy, risk, or actor
state.

The included orchestrator example is dry-run by default:

```bash
cargo run --example deposit_wallet_live -- --network amoy
```

Dry-run may build, sign, and serialize a WALLET batch after read-only discovery
and nonce lookup, but it must not call `POST /submit`. Live execution requires
both the CLI flag and an operator shell env gate:

```bash
POLYMARKET_RELAYER_ALLOW_LIVE_AMOY=1 cargo run --example deposit_wallet_live -- --network amoy --execute
```

The live gate fails closed before dotenv secret loading, signer construction,
relayer auth construction, or network requests when the env gate is absent. The
example also refuses non-Amoy networks before dotenv secret loading. Operators
must provide the env checklist documented in `docs/TESTING.md`; CI and consumer
default tests must not provide real secrets.

Relayer auth wire evidence is anchored to the official Polymarket relayer docs:

```text
https://docs.polymarket.com/api-reference/relayer/get-a-transaction-by-id
https://docs.polymarket.com/trading/gasless
https://docs.polymarket.com/api-reference/relayer-api-keys/get-all-relayer-api-keys
```

Those docs name `RELAYER_API_KEY` and `RELAYER_API_KEY_ADDRESS` as the Relayer
API key auth headers and define `RELAYER_API_KEY_ADDRESS` as the address that
owns the key. This HTTP client sends those headers as credential identity, while
still treating transaction `owner`/`from`, owner signer, deposit wallet, and
funder as separate identities. Consumers must not assume the relayer API key
address, owner signer, deposit wallet, or funder are the same identity.

Rollback path: remove the consumer adapter's live-submit wiring and keep the
golden/signing/mock-test-only integration. Because the crate remains behind the
adapter boundary, no consumer domain type should depend on the HTTP DTOs.

Consumer adapter migration status for PR #8:

- this crate PR records the required `0.2.0` migration boundary and rollback
  path, but it does not claim the downstream consumer adapter has already been
  migrated;
- any consumer pinning this PR must update its relayer adapter call sites from
  `build_wallet_batch_request_with_signature` to
  `try_build_wallet_batch_request_with_signature` in the same consumer-side
  integration change;
- live submit enablement remains Amoy-testnet-only and operator-gated, so a
  consumer adapter must default to dry-run unless Task 15 operator evidence is
  present and reviewed.

## Enablement Rule

Consumer live relayer mutation remains disabled until:

```text
WALLET-CREATE fixture tests pass
WALLET fixture tests pass
EIP-712 fixture tests pass
pUSD/CTF calldata fixture tests pass
mock relayer happy/failure paths pass
dry-run zero-submit and live-gate fail-closed tests pass
redaction and identity separation tests pass
transaction polling unknown-state tests pass
dependency is pinned by commit SHA
operator approval is recorded
Task 15 Amoy live smoke evidence reaches STATE_CONFIRMED
```
