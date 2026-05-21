# Deposit-Wallet Relayer Plans

This directory contains implementation plans for the remaining deposit-wallet
relayer work. Each file is scoped as one reviewable PR.

## Source Of Truth

- Polymarket Deposit Wallets guide:
  <https://docs.polymarket.com/trading/deposit-wallets>
- Relayer submit API:
  <https://docs.polymarket.com/api-reference/relayer/submit-a-transaction>
- Relayer transaction API:
  <https://docs.polymarket.com/api-reference/relayer/get-a-transaction-by-id>
- Official TypeScript builder relayer SDK:
  <https://github.com/Polymarket/builder-relayer-client>
- Official Python builder relayer SDK:
  <https://github.com/Polymarket/py-builder-relayer-client>

The canonical policy documents remain `docs/FORKED_RELAYER_CRATE.md`,
`docs/DEPOSIT_WALLET_RELAYER_DESIGN.md`, and `AGENTS.md`. These plan files
decompose that policy into implementation slices.

## Roadmap

```text
01 EIP-712 signing and nonce request fixture
  -> 02 mocked HTTP nonce/submit/poll client
    -> 03 pUSD/CTF calldata builders
      -> 04 consumer adapter and live gate
```

No plan in this directory makes the fork ready for live deposit-wallet
execution by itself. The live gate remains closed until signing, nonce,
submit/poll, calldata, identity separation, dry-run evidence, rollback policy,
and operator approval are all complete.

## PR Boundaries

- Keep deposit-wallet implementation under `src/deposit_wallet/`.
- Do not add top-level generic `eip712.rs`, `nonce.rs`, or `submit.rs`.
- Do not reuse legacy Safe/Proxy request, nonce, signing, or operation helpers
  unless the PR proves the wire shape, nonce source, and signer identity match.
- Keep CLOB order signing/posting outside this crate. The consumer must use the
  official Rust CLOB SDK for order flow.

## Validation Baseline

Each implementation PR must run:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo build --workspace --all-targets --all-features
git diff --check
```

For plan-only edits, the same commands still run to prove the repo remains
healthy.
