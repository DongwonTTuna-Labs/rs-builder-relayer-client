# Fork Scope

This repository is a controlled fork of `OrderBookTrade/rs-builder-relayer-client`.

## Purpose

The consumer Rust workspace needs a relayer crate for Polymarket deposit-wallet flows that are not covered by an official Rust builder relayer SDK.

## Initial Boundary

- Keep upstream Safe/Proxy code compiling as a reference.
- Add deposit-wallet support in a separate module.
- Avoid leaking fork-specific DTOs into consumer domain crates.
- Keep all HTTP/auth/EIP-712 details inside this crate.

## Documentation Index

- `FORKED_RELAYER_CRATE.md`: canonical fork policy and public API target.
- `DEPOSIT_WALLET_RELAYER_DESIGN.md`: relayer endpoint flow and state machine.
- `SECURITY.md`: secret, signing, supply-chain, and identity separation policy.
- `TESTING.md`: required fork tests and manual live gates.
- `DECISIONS.md`: ADRs for why this fork exists and how it may be consumed.
- `CONSUMER_INTEGRATION.md`: how `polymarket-liquidity-farming-rs` should depend on this crate.
- `REVIEW_CHECKLIST.md`: PR and production-readiness checklist.
- `REFERENCES.md`: official docs and reference implementations.
- `LEGACY_SAFE_PROXY_RELAYER_GUIDE.md`: upstream Safe/Proxy reference only.
- `PUBLISHING_DISABLED.md`: crates.io publishing policy.

## Required Deposit-Wallet Work

- `WALLET-CREATE` request construction.
- Fresh `/nonce?type=WALLET` lookup.
- DepositWallet Batch EIP-712 domain/message construction.
- `WALLET` submit request serialization.
- Transaction polling and terminal state mapping.
- pUSD adapter approval, merge, and redeem calldata helpers.

## Non-Goals

- No CLOB order client.
- No trading strategy.
- No live execution example until request signing and serialization fixtures are validated.
- No crates.io publishing unless explicitly re-approved.
- No production import by branch name.
