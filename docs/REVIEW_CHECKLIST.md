# REVIEW_CHECKLIST.md

## General

- [ ] PR is not merged by the agent.
- [ ] No private keys, API keys, auth headers, passphrases, or production signatures are committed.
- [ ] `cargo fmt --all --check` passes.
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings` passes.
- [ ] `cargo test --workspace --all-features` passes.
- [ ] `git diff --check` passes.

## Fork Scope

- [ ] The change belongs in this relayer crate, not in the consumer CLOB adapter or strategy code.
- [ ] Upstream Safe/Proxy behavior is not silently reused for deposit-wallet `WALLET` flow.
- [ ] New deposit-wallet code lives under `src/deposit_wallet/` or clearly related support modules.
- [ ] Legacy Safe/Proxy helpers remain clearly separated.

## Deposit-Wallet Flow

- [ ] `WALLET-CREATE` is implemented separately from Safe deploy.
- [ ] `WALLET` batch is implemented separately from Safe/Proxy execute.
- [ ] `/nonce?type=WALLET` is fetched fresh before signing.
- [ ] EIP-712 DepositWallet Batch domain/message/signature are fixture-tested.
- [ ] `POST /submit` request body has exact `type = "WALLET"` or `type = "WALLET-CREATE"` shape.
- [ ] Transaction polling handles New/Executed/Mined/Confirmed/Invalid/Failed/Unknown.
- [ ] Unknown/ambiguous state does not trigger duplicate submit.

## Identity And Security

- [ ] Relayer API key owner, wallet owner signer, and deposit wallet/funder are separate config/API fields.
- [ ] Tests prove relayer auth identity may differ from owner signer identity.
- [ ] Secret-bearing types do not leak through `Debug`, logs, errors, snapshots, or fixtures.
- [ ] Production dependency instructions use pinned git `rev`, not branch.

## pUSD / CTF Operations

- [ ] No legacy USDC.e/direct CTF helper is used for current pUSD-native deposit-wallet operations without explicit review.
- [ ] pUSD approval calldata has a golden test.
- [ ] conditional token approval calldata has a golden test.
- [ ] merge/redeem calldata follows current adapter route and has golden tests.

## Consumer Integration

- [ ] Consumer imports are limited to `pm-adapters/relayer_http` or runtime wiring.
- [ ] No fork-specific DTO leaks into consumer domain/strategy/risk/actor state.
- [ ] Official Rust CLOB SDK remains responsible for CLOB order path.
- [ ] Live relayer mutation remains gated until all fork acceptance tests and operator approval are recorded.
