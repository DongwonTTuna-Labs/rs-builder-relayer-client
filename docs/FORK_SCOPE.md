# Fork Scope

This repository is a controlled fork of `OrderBookTrade/rs-builder-relayer-client`.

## Purpose

The consumer Rust workspace needs a relayer crate for Polymarket deposit-wallet flows that are not covered by an official Rust builder relayer SDK.

## Initial Boundary

- Keep upstream Safe/Proxy code compiling as a reference.
- Add deposit-wallet support in a separate module.
- Avoid leaking fork-specific DTOs into consumer domain crates.
- Keep all HTTP/auth/EIP-712 details inside this crate.

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
