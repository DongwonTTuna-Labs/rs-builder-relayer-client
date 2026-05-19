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
