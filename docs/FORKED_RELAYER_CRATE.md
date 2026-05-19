# FORKED_RELAYER_CRATE.md

This document is the canonical policy for this forked Rust relayer crate.

## Decision

It is acceptable to fork `OrderBookTrade/rs-builder-relayer-client` or a similar Rust relayer project **only as an organization-controlled/audited internal crate**. It must not be imported as-is from upstream for production.

The app must continue to use:

```text
CLOB order/sign/cancel/balance path:
  official polymarket_client_sdk_v2

Deposit-wallet WALLET/WALLET-CREATE path:
  internal forked relayer crate OR raw RelayerHttpAdapter OR official TS/Python sidecar
```

The fork is not a replacement for the official CLOB SDK.

## Why upstream cannot be used directly

The observed third-party Rust relayer crate is Safe/Proxy-oriented. Its public examples use `RelayerTxType::Safe`, and its operations focus on Safe/Proxy deployment, approve, redeem, split, merge, and batch helpers. The deposit-wallet flow needed here is different:

```text
Deposit wallet deploy:
  POST /submit type = WALLET-CREATE

Deposit wallet on-chain actions:
  GET /nonce?type=WALLET
  sign DepositWallet Batch EIP-712 payload
  POST /submit type = WALLET
  poll /transaction

CLOB orders:
  official CLOB SDK with funder = deposit_wallet and signature_type = POLY_1271 / 3
```

Therefore the fork must add deposit-wallet-specific modules rather than only wrapping the existing Safe/Proxy execute flow.

## Fork name policy

Keep the upstream package/lib names:

```toml
[package]
name = "rs-builder-relayer-client"

[lib]
name = "polymarket_relayer"
```

Then app code imports:

```rust
use polymarket_relayer::{
    DepositWalletRelayerClient,
    DepositWalletCall,
    RelayerAuth,
};
```

The fork identity comes from the GitHub owner/repository and pinned commit, not from a new crate name. The app must still wrap it inside `pm-adapters/relayer_http`. No business/application crate may import the relayer crate directly.

## Dependency policy

During active development, use a path dependency:

```toml
[dependencies]
rs-builder-relayer-client = { path = "../rs-builder-relayer-client" }
```

For production builds, use a pinned commit, not a branch:

```toml
[dependencies]
rs-builder-relayer-client = {
  git = "ssh://git@github.com/DongwonTTuna/rs-builder-relayer-client.git",
  rev = "<audited_commit_sha>"
}
```

Forbidden:

```toml
# forbidden for production
rs-builder-relayer-client = { git = "...", branch = "main" }
rs-builder-relayer-client = "*"
rs-builder-relayer-client = "0.1"
rs-builder-relayer-client = { git = "ssh://git@github.com/OrderBookTrade/rs-builder-relayer-client.git", rev = "..." }
```

## Required fork modules

```text
rs-builder-relayer-client/
  src/
    lib.rs
    auth.rs
    client.rs
    nonce.rs
    submit.rs
    transaction.rs
    eip712.rs
    deposit_wallet/
      mod.rs
      address.rs
      deploy.rs
      batch.rs
      types.rs
    operations/
      mod.rs
      pusd.rs
      ctf_adapter.rs
      merge.rs
      redeem.rs
    errors.rs
```

The old Safe/Proxy API may remain only if clearly labeled legacy and disabled by default in this bot.

## Required public API

```rust
pub struct RelayerKeyAuth {
    pub api_key: SecretString,
    pub api_key_owner_address: Address,
}

pub struct DepositWalletRequestContext {
    pub owner_address: Address,
    pub deposit_wallet_address: Address,
}

pub struct DepositWalletCall {
    pub target: Address,
    pub value: U256,
    pub data: Bytes,
}

pub struct DepositWalletRelayerClient<S> {
    // fields private
    _phantom: std::marker::PhantomData<S>,
}

impl<S> DepositWalletRelayerClient<S>
where
    S: Signer,
{
    pub fn new(
        relayer_url: Url,
        chain_id: u64,
        owner_signer: S,
        auth: RelayerKeyAuth,
        deposit_wallet_factory: Address,
    ) -> Self;

    pub async fn derive_deposit_wallet_address(
        &self,
        owner: Address,
    ) -> Result<Address, RelayerError>;

    pub async fn deploy_deposit_wallet(
        &self,
        owner: Address,
    ) -> Result<RelayerSubmitResponse, RelayerError>;

    pub async fn get_wallet_nonce(
        &self,
        owner: Address,
    ) -> Result<U256, RelayerError>;

    pub async fn sign_deposit_wallet_batch(
        &self,
        ctx: &DepositWalletRequestContext,
        calls: &[DepositWalletCall],
        nonce: U256,
        deadline: U256,
    ) -> Result<SignedDepositWalletBatch, RelayerError>;

    pub async fn execute_deposit_wallet_batch(
        &self,
        ctx: DepositWalletRequestContext,
        calls: Vec<DepositWalletCall>,
        deadline: U256,
    ) -> Result<RelayerSubmitResponse, RelayerError>;

    pub async fn poll_transaction(
        &self,
        transaction_id: RelayerTransactionId,
    ) -> Result<RelayerTransactionStatus, RelayerError>;
}
```

## Address separation rule

Never collapse these identities:

```text
RELAYER_API_KEY_ADDRESS:
  credential/auth identity used in relayer auth headers

DEPOSIT_WALLET_OWNER_ADDRESS:
  owner/session signer used as /submit `from` and EIP-712 signer

DEPOSIT_WALLET_ADDRESS:
  smart contract wallet/funder that holds pUSD and conditional tokens
```

Equality can happen in a specific deployment, but it must never be an implicit code assumption.

## Required wire flows

### WALLET-CREATE

```text
POST /submit
{
  "type": "WALLET-CREATE",
  "from": "0xOwnerAddress",
  "to": "0xDepositWalletFactory"
}
```

No user signature is added to the WALLET-CREATE deployment body.

### WALLET batch

```text
GET /nonce?address=<owner>&type=WALLET
sign DepositWallet Batch EIP-712 payload
POST /submit
{
  "type": "WALLET",
  "from": "0xOwnerAddress",
  "signature": "0x...",
  "depositWalletParams": { ... }
}
```

The nonce must be fetched fresh immediately before signing. A stale nonce is a live-risk bug.

### Transaction polling

```text
GET /transaction?transactionID=<id>
```

The submit response may not contain the on-chain transaction hash immediately. The fork must track `transactionID` and poll to a terminal state.

## Transaction states

```rust
pub enum RelayerTransactionState {
    New,
    Executed,
    Mined,
    Confirmed,
    Invalid,
    Failed,
    Unknown(String),
}
```

Policy:

```text
Confirmed:
  terminal success

Failed / Invalid:
  terminal failure; do not blindly retry

New / Executed / Mined:
  pending; bounded polling

Unknown:
  stop mutation, persist evidence, reconcile before any new submit
```

## pUSD / CTF adapter policy

Do not use legacy convenience helpers that hard-code USDC.e or direct CTF paths unless explicitly reviewed for the current pUSD-native flow.

Required builders:

```text
pUSD approval from deposit wallet
conditional token approval from deposit wallet
adapter-based split/merge/redeem calldata if enabled
```

Each builder needs golden tests against official TS/Python/reference payloads before live use.

## App integration

`pm-adapters/relayer_http` may wrap the fork:

```rust
pub struct ForkedDepositRelayerAdapter<C> {
    inner: C,
}

#[async_trait::async_trait]
impl<C> RelayerPort for ForkedDepositRelayerAdapter<C>
where
    C: DepositWalletRelayerLike + Send + Sync,
{
    // map fork types <-> internal domain/application types
}
```

Rules:

```text
pm-runtime may construct the forked client.
pm-adapters may call the forked client.
pm-ports exposes only RelayerPort.
pm-app, pm-actors, pm-risk, pm-strategy, pm-domain must not import the forked crate.
```

## Mandatory tests before production import

```text
1. derive_deposit_wallet_address parity test with TS/Python official client
2. WALLET-CREATE request serialization golden test
3. WALLET nonce request includes type=WALLET
4. DepositWallet Batch EIP-712 digest/signature fixture test
5. WALLET submit body serialization golden test
6. transaction state parser covers New/Executed/Mined/Confirmed/Invalid/Failed/Unknown
7. no duplicate submit after timeout/ambiguous response fake test
8. pUSD approval calldata golden test
9. merge/redeem calldata golden tests for adapter route
10. CLOB integration test separately proves POLY_1271 + funder deposit wallet order path
```

## Production approval gate

The fork can be imported in production only after:

```text
- dependency is pinned by commit SHA;
- license review completed;
- supply-chain review completed;
- source diff from upstream reviewed;
- deposit-wallet WALLET/WALLET-CREATE tests passed;
- pUSD adapter calldata tests passed;
- relayer API key identity and owner signer identity are separate config values;
- operator approval is recorded.
```

If any item is missing, use the official TypeScript/Python relayer sidecar or keep relayer live mutation disabled.
