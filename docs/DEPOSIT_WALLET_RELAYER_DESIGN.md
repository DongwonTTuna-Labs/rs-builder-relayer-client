# RELAYER_DESIGN.md

This is the canonical design for deposit-wallet relayer support in this fork.

## Scope

CLOB order signing/posting is handled through the official Rust CLOB SDK adapter. Deposit-wallet deployment and wallet batches are handled through one of these paths:

```text
preferred single-binary Rust path:
  RelayerHttpAdapter using raw relayer REST API

this crate:
  organization-controlled forked deposit-wallet relayer crate wrapped by pm-adapters/relayer_http in the consumer app

acceptable interim path:
  official TypeScript/Python relayer sidecar

not acceptable:
  upstream Safe/Proxy-oriented third-party Rust relayer crate imported directly into production app code
```

## CLOB order path vs relayer wallet batch path

| Operation | Path |
| --- | --- |
| market/orderbook/order status/cancel | official Rust CLOB SDK adapter |
| deposit-wallet order signing/posting | official Rust CLOB SDK adapter with deposit wallet funder and POLY_1271/signature type 3 |
| balance allowance read/cache sync | CLOB read adapter |
| deposit wallet deploy | relayer `WALLET-CREATE` |
| approvals, transfers, split/merge/redeem | relayer `WALLET` batch |
| transaction polling | relayer `/transaction` |

Do not implement relayer operations inside the CLOB adapter.

## Address model

```rust
pub struct RelayerAuthIdentity {
    pub api_key_address: WalletAddress,
}

pub struct DepositWalletOwner {
    pub owner_address: WalletAddress,
}

pub struct DepositWalletAddress {
    pub address: WalletAddress,
}
```

Rules:

```text
RELAYER_API_KEY_ADDRESS authenticates relayer API requests.
DEPOSIT_WALLET_OWNER_ADDRESS signs WALLET batches and is used as submit `from`.
DEPOSIT_WALLET_ADDRESS is the CLOB funder/contract wallet address.
Do not assume any two of these are equal.
```

## RelayerPort

```rust
#[async_trait::async_trait]
pub trait RelayerPort: Send + Sync {
    async fn get_wallet_nonce(
        &self,
        owner: &DepositWalletOwner,
        permit: &RelayerPermit,
    ) -> Result<RelayerNonce, RelayerError>;

    async fn submit_wallet_create(
        &self,
        request: WalletCreateRequest,
        permit: &RelayerPermit,
    ) -> Result<RelayerSubmitReceipt, RelayerError>;

    async fn submit_wallet_batch(
        &self,
        request: SignedDepositWalletBatch,
        permit: &RelayerPermit,
    ) -> Result<RelayerSubmitReceipt, RelayerError>;

    async fn get_transaction(
        &self,
        id: &RelayerTransactionId,
    ) -> Result<RelayerTransactionStatus, RelayerError>;
}
```

`RelayerPort` exposes internal domain/application types only. It must not expose fork-specific DTOs or raw SDK types.

## Endpoint flows

### WALLET nonce

```text
GET /nonce?address=<owner>&type=WALLET
```

The nonce must be fresh immediately before signing a `WALLET` batch.

### WALLET-CREATE

```text
POST /submit
body.type = "WALLET-CREATE"
body.from = deposit wallet owner address
body.to = active deposit wallet factory address
```

Use only if the runtime owns wallet deployment. If wallets are pre-deployed, this flow remains disabled.

### WALLET batch

```text
POST /submit
body.type = "WALLET"
body.from = deposit wallet owner address
body.signature = EIP-712 DepositWallet Batch signature
body.depositWalletParams = signed DepositWallet batch payload
```

Used for approvals, transfers, and pUSD-native CTF operations.

### Transaction polling

```text
GET /transaction?id=<transactionID>
```

`POST /submit` returns a `transactionID`. The on-chain transaction hash may be unavailable until polling succeeds.

## Transaction state machine

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
  terminal success; deposit-wallet effects may be used by later actions

Mined:
  pending/non-terminal; included onchain but do not rely on deposit-wallet
  readiness or wallet action effects until STATE_CONFIRMED

Failed / Invalid:
  terminal failure; do not blindly retry

New / Executed:
  pending/non-terminal; poll under timeout policy

Unknown:
  stop relayer mutation and reconcile; do not duplicate submit
```

## EIP-712 signing

Deposit-wallet `WALLET` batches require dedicated EIP-712 signing. The signed payload is not interchangeable with CLOB order signatures.

Required evidence:

```text
owner address
nonce
chain id
verifying contract / deposit wallet address
batch calls
safe redacted payload summary
signature hash
```

Raw signatures and private keys must never be logged.

## Forked internal relayer crate option

A fork of `OrderBookTrade/rs-builder-relayer-client` or similar code may be used only if it becomes an internal deposit-wallet relayer crate that satisfies `FORKED_RELAYER_CRATE.md`.

Approved pattern:

```text
official Polymarket Rust CLOB SDK
  -> used by pm-adapters/clob_rs_sdk only

forked internal deposit-wallet relayer crate
  -> used by pm-adapters/relayer_http only
  -> implements WALLET-CREATE / WALLET / DepositWallet Batch EIP-712
  -> hidden behind RelayerPort
```

Fork naming:

```toml
[package]
name = "rs-builder-relayer-client"

[lib]
name = "polymarket_relayer"
```

Keep the upstream package/lib names. The fork is identified by the GitHub owner/repository and pinned commit.

Development dependency:

```toml
rs-builder-relayer-client = { path = "../rs-builder-relayer-client" }
```

Production dependency:

```toml
rs-builder-relayer-client = {
  git = "ssh://git@ssh.dongwontuna.net/DongwonTTuna-Labs/rs-builder-relayer-client.git",
  rev = "<audited_commit_sha>"
}
```

Floating dependencies such as `branch = "main"` are forbidden for production.

## Required fork API surface

The forked crate should model deposit-wallet relayer operations directly, not rename Safe/Proxy APIs.

```rust
pub struct DepositWalletCall {
    pub target: Address,
    pub value: U256,
    pub data: Bytes,
}

pub struct RelayerKeyAuth {
    pub api_key: SecretString,
    pub api_key_owner_address: Address,
}

pub struct DepositWalletRequestContext {
    pub owner_address: Address,
    pub deposit_wallet_address: Address,
}

impl<S> DepositWalletRelayerClient<S>
where
    S: Signer,
{
    pub async fn derive_deposit_wallet_address(&self, owner: Address) -> Result<Address>;
    pub async fn deploy_deposit_wallet(&self, owner: Address) -> Result<RelayerSubmitResponse>;
    pub async fn get_wallet_nonce(
        &self,
        owner: Address,
        mutation_gate: DepositWalletMutationGate,
    ) -> Result<U256>;
    pub async fn get_wallet_nonce_with_lease(
        &self,
        owner: Address,
        mutation_gate: DepositWalletMutationGate,
    ) -> Result<DepositWalletNonceLease>;
    pub async fn sign_deposit_wallet_batch(
        &self,
        ctx: &DepositWalletRequestContext,
        calls: &[DepositWalletCall],
        nonce: U256,
        deadline: U256,
    ) -> Result<SignedDepositWalletBatch>;
    pub async fn execute_deposit_wallet_batch(
        &self,
        ctx: DepositWalletRequestContext,
        calls: Vec<DepositWalletCall>,
        deadline: U256,
    ) -> Result<RelayerSubmitResponse>;
    pub async fn poll_transaction(&self, id: RelayerTransactionId) -> Result<RelayerTransactionStatus>;
}
```

Production WALLET signing must use the leased nonce path:
`get_wallet_nonce_with_lease` -> sign with `lease.nonce()` ->
`submit_signed_wallet_batch_with_nonce_lease`. The bare `get_wallet_nonce`
method is retained for compatibility and diagnostics, but it is not a
production live-signing API because its owner reservation ends when the `U256`
is returned.

## CTF/pUSD adapter policy

Latest pUSD-native CTF operations must route through the current adapter path. Do not rely on legacy USDC.e or direct CTF helper defaults from third-party libraries.

Required calldata builders:

```text
pUSD approval from deposit wallet
conditional token approval from deposit wallet
split positions if enabled
merge positions
redeem positions
```

Each builder requires golden tests against official TS/Python/reference payloads before live use.

## Retry and idempotency

Relayer submit is not safely retryable unless the previous submit has been reconciled.

Required local evidence:

```text
relayer_intent_id
signed payload hash
nonce
submit timestamp
transactionID if known
last observed relayer state
poll attempts
redacted call summary
```

On timeout:

```text
1. poll by known transactionID if available.
2. query recent transactions if supported and safe.
3. reconcile wallet/account state.
4. only then decide whether a new signed batch is required.
```

## Fork acceptance tests

| Test | Required proof |
| --- | --- |
| deposit wallet derive parity | Rust result equals official TS/Python reference |
| WALLET-CREATE request serialization | `POST /submit` body is exactly the WALLET-CREATE shape |
| WALLET batch request serialization | `type = "WALLET"`, nonce, signature, and `depositWalletParams` are correct |
| EIP-712 DepositWallet Batch fixture | digest/signature shape matches reference fixture |
| fresh nonce | batch signing fetches `GET /nonce?type=WALLET` immediately before signing |
| auth/owner split | relayer API key address may differ from owner signer address |
| pUSD adapter calldata | approval/merge/redeem route uses current adapter policy |
| CLOB integration | official CLOB SDK uses deposit wallet funder and POLY_1271 separately from relayer |
| live tiny-value validation | WALLET-CREATE/WALLET path succeeds under explicit operator gate |

## Live enablement checklist

- [ ] address identity separation configured;
- [ ] nonce flow tested;
- [ ] WALLET-CREATE disabled unless explicitly needed;
- [ ] WALLET batch signing golden tests passed;
- [ ] transaction state parser handles unknown states;
- [ ] pUSD adapter calldata golden tests passed;
- [ ] no duplicate submit after timeout tests passed;
- [ ] forked crate dependency pinned by commit SHA or vendored source;
- [ ] `RelayerPermit` cannot be created in read-only, dry-run, or cancel-only modes;
- [ ] operator has reviewed redacted payload summaries.
