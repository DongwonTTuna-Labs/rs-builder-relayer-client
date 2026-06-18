# Tasks

## 1. Implementation

- [ ] Implement ONLY the body of `build_erc20_approve_call` in `src/deposit_wallet/calldata.rs`; keep the fixed signature `build_erc20_approve_call(token: Address, spender: Address, amount: U256) -> Result<DepositWalletCall, RelayerError>` unchanged.
- [ ] Build `DepositWalletCall` for ERC20 `approve(address,uint256)` with selector `0x095ea7b3`, ABI arguments `(spender, amount)`, `target == token`, and `value == 0`.
- [ ] Return `Err(RelayerError...)` without panic when `token` or `spender` is the zero address.
- [ ] Grimoire autonomous implementation authority: only the body of `build_erc20_approve_call` in `src/deposit_wallet/calldata.rs` is editable by Grimoire.
- [ ] Broader human-authored PR/supporting-artifact scope may include fixtures, OpenSpec artifacts, Cargo metadata, policy docs, and related operation/auth/signing/nonce/http/submit/request-builder modules when explicitly reviewed and approved outside Grimoire autonomous implementation authority.

## 2. Validation

- [ ] Validation: run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo test --workspace --all-features` (green), and `openspec validate add-deposit-wallet-erc20-approve-calldata --strict`.
