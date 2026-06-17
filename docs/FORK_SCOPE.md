# Fork Scope

This repository is a controlled fork of `OrderBookTrade/rs-builder-relayer-client`.

## Purpose

The consumer Rust workspace needs a relayer crate for Polymarket deposit-wallet flows that are not covered by an official Rust builder relayer SDK.

## Initial Boundary

- Keep upstream Safe/Proxy code compiling as a reference.
- Add deposit-wallet support in a separate module.
- Avoid leaking fork-specific DTOs into consumer domain crates.
- Keep all HTTP/auth/EIP-712 details inside this crate.
- Keep live-capable deposit-wallet behavior narrowly scoped to Polygon Amoy testnet until operator evidence is recorded and reviewed.

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
- Mock relayer happy/failure coverage, dry-run zero-submit proof, live-gate fail-closed proof, and redaction/identity tests.

## Current Amoy-Only Expansion

The fork now contains a dry-run-default `examples/deposit_wallet_live.rs` orchestrator and production HTTP primitives for the Amoy live smoke. This expansion exists only to satisfy the deposit-wallet AGENTS.md gate with automated mock/golden proof before Task 15. It is not a mainnet readiness statement, and it does not enable live split/merge/redeem or CLOB behavior.

## Non-Goals

- No CLOB order client.
- No trading strategy.
- No mainnet deposit-wallet live execution in this round.
- No live split/merge/redeem execution in this round; those calldata builders are golden-fixtured and dry-run only.
- No live execution without dry-run default, `--execute`, `POLYMARKET_RELAYER_ALLOW_LIVE_AMOY=1`, Amoy chain `80002`, and the validated Amoy relayer URL.
- No crates.io publishing unless explicitly re-approved.
- No production import by branch name.
