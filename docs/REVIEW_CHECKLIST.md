# REVIEW_CHECKLIST.md

## General

- [ ] PR is not merged by the agent.
- [ ] No private keys, API keys, auth headers, passphrases, or production signatures are committed.
- [ ] `cargo fmt --all --check` passes.
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings` passes.
- [ ] `cargo test --workspace --all-features` passes.
- [ ] `git diff --check` passes.
- [ ] Any exception to repo rules is justified by documented no-viable-alternative evidence, not convenience.
- [ ] Any rule exception documents failed alternatives, the underlying limitation, consumer impact, and migration/rollback path.
- [ ] Claims in the PR body are backed by command output, fixture evidence, official SDK/doc comparison, or explicitly marked residual risk.
- [ ] Setup/docs/behavior/live-capable changes are not mixed without an explicit scope justification.

## Failure Handling

- [ ] New fixes document the root cause with `who`, `what`, `when`, `why`, and `how` when they address a failure.
- [ ] The change resolves the root cause instead of adding an unbounded retry, sleep, silent fallback, lint allow, broad mock, or test deletion.
- [ ] Any temporary workaround has a removal condition, owner, bound, and risk note.
- [ ] Repeated failures have a regression test, fixture, or documented reason why one cannot be added.

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
- [ ] Dependency changes review public API, transitive crypto/signing crates, and HTTP/TLS impact where applicable.

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
- [ ] Consumer-impacting changes document migration path, rollback path, and any unavailable rollback condition.
- [ ] New public relayer APIs document their production capability boundary, including any method that is intentionally disabled for production URLs.
