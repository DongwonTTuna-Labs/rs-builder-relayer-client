# Tasks

## 1. Implementation

- [ ] Implement ONLY the body of `build_erc20_approve_call` in `src/deposit_wallet/calldata.rs`; keep the fixed signature `build_erc20_approve_call(token: Address, spender: Address, amount: U256) -> Result<DepositWalletCall, RelayerError>` unchanged.
- [ ] Build `DepositWalletCall` for ERC20 `approve(address,uint256)` with selector `0x095ea7b3`, ABI arguments `(spender, amount)`, `target == token`, and `value == 0`.
- [ ] Return `Err(RelayerError...)` without panic when `token` or `spender` is the zero address.
- [ ] Allowed write paths: only the body of `build_erc20_approve_call` in `src/deposit_wallet/calldata.rs` is editable.
- [ ] Do NOT touch: fixtures, the frozen ERC20 approve test, OpenSpec artifacts, `.github/**`, `.omo/**`, `AGENTS.md`, `Cargo.toml`, `Cargo.lock`, `src/operations/**`, auth/signing/nonce/http/submit modules, request builders outside the approved function body, or policy docs.

## 2. Validation

- [ ] Validation: run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo test --workspace --all-features` (green), and `openspec validate add-deposit-wallet-erc20-approve-calldata --strict`.
