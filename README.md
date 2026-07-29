# rs-builder-relayer-client

Internal fork of `OrderBookTrade/rs-builder-relayer-client` for the
`DongwonTTuna/polymarket-liquidity-farming-rs` migration.

This fork is not an official Polymarket SDK. It preserves the upstream
Safe/Proxy implementation as legacy reference code while the reviewed
deposit-wallet relayer surface is added in focused, audited PRs.

## Reviewed 0.2.0 Public API Boundary

The crate root is the consumer-facing integration surface. For deposit-wallet
work, use the reviewed fallible APIs such as
`try_build_wallet_batch_request_with_signature`,
`DepositWalletRelayerClient`, `DepositWalletRelayerUrl`,
`RelayerReadPermit`, `DepositWalletRequestContext`, `DepositWalletCall`,
`DepositWalletDeploymentPolicy`, `DepositWalletDeploymentStatus`,
`DepositWalletReadiness`,
`RelayerKeyAuth`, `RelayerMutationPermit`, `RelayerMutationMode`,
`RelayerMutationOperation`, `RelayerSubmitOutcome`,
`DepositWalletDryRunEvidence`, `DryRunCallSummary`,
`DepositWalletSubmitReceipt`, and the documented request/response types
re-exported from `polymarket_relayer`.

The reviewed HTTP surface has three owner- and chain-scoped low-level reads,
two deployment-lifecycle methods, one fresh-nonce WALLET batch execution
method, and exactly two explicitly gated public `submit_*` methods.
`RelayerReadPermit` is required for
`is_deposit_wallet_deployed`, `get_wallet_nonce`, and
`get_transaction_for_owner`, as well as both lifecycle methods.
`ensure_deposit_wallet_deployment` checks deployed fact first and requires an
explicit `DepositWalletDeploymentPolicy`; `Predeployed` is the normal consumer
choice and blocks WALLET-CREATE when deployment is missing.
`check_deposit_wallet_deployment_readiness` performs one WALLET-CREATE status
read and maps only `STATE_CONFIRMED` to `Ready`. `submit_wallet_create` and
`submit_signed_wallet_batch` each require a mode-, operation-, owner-, chain-,
and expiry-scoped `RelayerMutationPermit`. `DepositWalletRelayerClient::new`
starts with live mutation denied; `new_with_mutation_enabled` is the explicit
constructor whose local mutation latch starts enabled, and `disable_mutation`
is a shared one-way rollback latch across all clones. Valid `DryRun` permits
produce redacted evidence without HTTP and remain usable after that latch is
disabled.

`execute_wallet_batch` accepts the owner signer as a generic method argument;
the client never stores it. The method validates both permits, deadline,
signer identity, resource limits, and the owner/config-derived wallet before
fetching `GET /nonce?type=WALLET`. It then signs immediately, rebuilds the
request through the existing signature-recovery checks, and delegates to the
permit-gated submit path. Dry-run execution still reads the fresh nonce and
signs locally, but sends no submit request.

This mutation surface is a permit and transport gate, not a claim of complete
deposit-wallet live readiness. A successful deployed read records deployment
fact only, and a submit receipt records relayer acceptance evidence rather than
`STATE_CONFIRMED`. PBRSDK-8 adds confirmed-only single-shot readiness and
PBRSDK-9 adds fresh-nonce batch execution, but bounded polling, owner-scoped
nonce leases and pending-intent enforcement, persistent
idempotency/reconciliation, recent-transaction recovery, and the remaining
operator gates are later work. Preserve each submitted transaction id and
payload hash, never re-enter deployment for an owner with a pending create,
and do not execute concurrent batches for the same owner until the later lease
contract is implemented.

Do not treat this crate as a CLOB order/sign/cancel/post SDK. CLOB
order/sign/cancel/post behavior remains out of this crate and belongs in the
official Polymarket Rust CLOB SDK plus the consumer CLOB adapter.

The removed infallible `build_wallet_batch_request_with_signature` helper is
not part of the public integration surface. Consumers must migrate to the
fallible `try_` API or the validated signed-batch flow and keep raw
deposit-wallet submit DTO construction out of domain, strategy, risk, and actor
layers.

Rust SDK for [Polymarket's gasless relayer](https://docs.polymarket.com/trading/gasless). Redeem positions, approve tokens, split/merge — zero gas.

## Documentation

- `docs/FORKED_RELAYER_CRATE.md`: fork policy and target API surface.
- `docs/DEPOSIT_WALLET_RELAYER_DESIGN.md`: deposit-wallet relayer design.
- `docs/SECURITY.md`: secret/signing/supply-chain rules.
- `docs/TESTING.md`: required fixture and acceptance tests.
- `docs/CONSUMER_INTEGRATION.md`: dependency and adapter boundary rules.
- `docs/LEGACY_SAFE_PROXY_RELAYER_GUIDE.md`: upstream Safe/Proxy reference only.

## Legacy Safe/Proxy Quickstart

The examples below are retained from upstream as Safe/Proxy references. They
are not proof that deposit-wallet `WALLET-CREATE` or `WALLET` support exists.
Do not use them as the implementation guide for the migration.

```bash
cargo new my-redeemer && cd my-redeemer
cargo add ethers tokio --features tokio/full
cargo add anyhow dotenvy hex
```

Create `.env`:
```
PRIVATE_KEY=0x...
BUILDER_KEY=...
BUILDER_SECRET=...
BUILDER_PASSPHRASE=...
# Optional: Use Alchemy or QuickNode for Direct Fallback. Default polygon-rpc.com is unstable.
POLYGON_RPC_URL=https://...
```

`src/main.rs`:
```rust
use polymarket_relayer::{RelayClient, AuthMethod, RelayerTxType};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let wallet = std::env::var("PRIVATE_KEY")?.parse()?;
    let mut client = RelayClient::new(
        137, wallet,
        AuthMethod::builder(
            &std::env::var("BUILDER_KEY")?,
            &std::env::var("BUILDER_SECRET")?,
            &std::env::var("BUILDER_PASSPHRASE")?,
        ),
        RelayerTxType::Safe,
    ).await?;

    // Read nonce from on-chain (avoids stale relayer API nonce → GS026)
    if let Ok(rpc) = std::env::var("POLYGON_RPC_URL") {
        client.set_rpc_url(rpc);
    }

    client.setup_approvals().await?.wait().await?;
    println!("Done. You can now trade gaslessly.");
    Ok(())
}
```

```bash
cargo run
```

## Getting Your Credentials

| Credential | Where |
|---|---|
| `PRIVATE_KEY` | Your Polygon wallet private key (MetaMask > Account Details > Export) |
| Relayer API key | [polymarket.com/settings > Relayer API Keys](https://polymarket.com/settings) (anyone) |

No Builder keys? Use `AuthMethod::relayer_key("key", "address")` instead — same features, simpler setup.

---

## Install

```toml
[dependencies]
rs-builder-relayer-client = {
  git = "ssh://git@ssh.dongwontuna.net/DongwonTTuna-Labs/rs-builder-relayer-client.git",
  rev = "<commit-sha>"
}
ethers = "2"
tokio = { version = "1", features = ["full"] }
anyhow = "1"
dotenvy = "0.15"
hex = "0.4"
```

## Redeem Example

Add `CONDITION_ID=0x...` to your `.env`, then:

```rust
use polymarket_relayer::{AuthMethod, RelayClient, RelayerTxType, operations};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    let wallet = std::env::var("PRIVATE_KEY")?.parse()?;
    let client = RelayClient::new(
        137,
        wallet,
        AuthMethod::builder(
            &std::env::var("BUILDER_KEY")?,
            &std::env::var("BUILDER_SECRET")?,
            &std::env::var("BUILDER_PASSPHRASE")?,
        ),
        RelayerTxType::Safe,
    ).await?;

    let condition_id_hex = std::env::var("CONDITION_ID")?;
    let condition_id_bytes = hex::decode(condition_id_hex.trim_start_matches("0x"))?;
    let mut cid = [0u8; 32];
    cid.copy_from_slice(&condition_id_bytes);

    let tx = operations::redeem_regular(cid, &[1, 2]);
    let result = client.execute(vec![tx], "Redeem").await?.wait().await?;
    println!("Transaction Hash: {:?}", result.tx_hash);

    Ok(())
}
```

## API

| Operation | Code |
|---|---|
| Redeem regular position | `operations::redeem_regular(condition_id, &[1, 2])` |
| Redeem neg-risk position | `operations::redeem_neg_risk_positions(condition_id, &[1, 2])` |
| Approve USDC for exchange | `client.setup_approvals()` |
| Deploy Safe wallet | `client.deploy()` |
| Split USDC into tokens | `operations::split_regular(cid, &[1, 2], amount)` |
| Merge tokens back to USDC | `operations::merge_regular(cid, &[1, 2], amount)` |
| Execute single/multiple ops | `client.execute(vec![tx1], "desc")` |
| Execute true multi-send batch | `client.execute_batch(vec![tx1, tx2], "desc")` |
| Execute chunks sequentially | `client.execute_sequential(vec![vec![tx1], vec![tx2]], None, None)` |
| Direct on-chain fallback | `DirectExecutor::new(rpc_url, wallet, 137)?` |

## Auth

```rust
// Builder API keys (HMAC — enables gasless)
AuthMethod::builder("key", "secret", "passphrase")

// Relayer API keys (from polymarket.com/settings > API Keys)
AuthMethod::relayer_key("api_key", "wallet_address")
```

## Direct Fallback (when relayer returns 429)

> **Warning:** Do **not** use `https://polygon-rpc.com/` as your RPC URL — it frequently causes TLS handshake EOF errors and connection resets, especially under load. Use a dedicated provider instead:
> - [Alchemy](https://www.alchemy.com/) (recommended): `https://polygon-mainnet.g.alchemy.com/v2/YOUR_KEY`
> - [QuickNode](https://www.quicknode.com/): `https://YOUR_ENDPOINT.quiknode.pro/YOUR_KEY/`
> - [LlamaRPC](https://llamarpc.com/): `https://polygon.llamarpc.com`

```rust
use polymarket_relayer::{DirectExecutor, RelayerError};

let rpc_url = std::env::var("POLYGON_RPC_URL")
    .expect("Set POLYGON_RPC_URL to an Alchemy/QuickNode endpoint");

// Safe wallet (signature_type=2, default)
let direct = DirectExecutor::new(&rpc_url, wallet, 137)?;

// Proxy wallet (signature_type=1, e.g. magic.link)
let direct = DirectExecutor::new_proxy(&rpc_url, wallet, 137)?;

// Proxy with explicit address (when derived address differs)
let direct = DirectExecutor::new_proxy_with_address(&rpc_url, wallet, 137, proxy_addr)?;

match client.execute(vec![tx], "Redeem").await {
    Err(RelayerError::QuotaExhausted) => {
        let result = direct.execute(&tx).await?;  // pays gas in MATIC
    }
    other => { /* handle normally */ }
}
```

## Batching & Execution Strategies

Depending on whether you use `RelayerTxType::Safe` or `RelayerTxType::Proxy`, the SDK provides several execution models:

* **`client.execute` / `client.execute_batch`**:
  * **Safe Wallets**: Uses official Gnosis `MultiSend` contracts. Multiple operations are packed tightly into a single transaction. Safe is highly durable and recommended for heavy batching (> 2 operations).
  * **Proxy Wallets**: While the OpenGSN proxy supports a `(uint8, address, uint256, bytes)[]` array structure, the Polymarket relayer bot imposes strict total-transaction gas limits top-level. **Batching more than 2 operations with Proxy wallets is highly discouraged** and might hit silent `relay hub: internal transaction failure` errors due to gas starvation. The SDK dynamically scales requests up to a hard cap of 400K gas. 

* **`client.execute_sequential`**: 
  Designed purely to circumvent Proxy Relayer bottlenecks when you have e.g. 10 positions to redeem. It executes batches step-by-step, patiently awaiting `STATE_CONFIRMED` to prevent nonce collisions and OpenGSN RelayHub deadlocks across Gelato's relayer pools.

## Examples

```bash
cp .env.example .env   # fill in your keys

cargo run --example redeem_all                  # dry-run: scan positions
cargo run --example redeem_all -- --execute     # actually redeem
cargo run --example setup_wallet                # deploy Safe + approvals
cargo run --example redeem_single               # redeem one position
cargo run --example split_merge                 # split/merge demo
cargo run --example redeem_magic                # magic.link proxy wallet redeem
```

## References

- [Gasless Docs](https://docs.polymarket.com/trading/gasless) | [Python SDK](https://github.com/Polymarket/py-builder-relayer-client) | [TypeScript SDK](https://github.com/Polymarket/builder-relayer-client)

## Donate

**Ethereum / Polygon:** `0xF4c6635dFfB53f21c500c1604EC284f8A8a7150D`
