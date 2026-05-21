# PR 03: Deposit-Wallet pUSD And CTF Calldata Builders

## Summary

Add deposit-wallet-specific calldata builders for pUSD approvals, conditional
token approvals, and enabled CTF adapter operations. This PR only builds calldata
that will later be submitted inside `WALLET` batches.

## In Scope

- Add `src/deposit_wallet/calldata/` for deposit-wallet operation calldata.
- Add builders for pUSD ERC-20 approval from the deposit wallet.
- Add builders for conditional token ERC-1155 approval from the deposit wallet.
- Add split, merge, and redeem builders only for routes that can be verified
  against current Polymarket pUSD-native adapter behavior.
- Add golden fixtures under `tests/fixtures/deposit_wallet/calldata/`.

## Out Of Scope

- No live submission of generated calldata.
- No CLOB order posting or balance sync.
- No reuse of legacy USDC.e or direct CTF helpers without explicit fixture proof.
- No broad operation abstraction that hides target contract, value, and calldata.

## Target API

- `DepositWalletCalldataConfig`: chain id, pUSD contract address, CTF contract
  address, enabled adapter addresses, and source metadata for each address.
- `build_pusd_approval_call(config, spender, amount) -> DepositWalletCall`.
- `build_ctf_approval_for_all_call(config, operator, approved) ->
  DepositWalletCall`.
- `build_split_position_call(config, ...) -> DepositWalletCall` only if the
  current adapter route is verified.
- `build_merge_positions_call(config, ...) -> DepositWalletCall` only if the
  current adapter route is verified.
- `build_redeem_positions_call(config, ...) -> DepositWalletCall` only if the
  current adapter route is verified.

All builders must return explicit `DepositWalletCall { target, value, data }`
values from the provided config. Hidden defaults, global contract addresses,
chain inference, and fallback targets are not allowed.

## Fixture Requirements

- Fixtures must record contract address, method selector, argument values,
  encoded calldata, and source reference.
- pUSD and CTF addresses must come from official docs, official SDK/config, or
  a documented current consumer runtime source.
- Fixture source references must include the source name and version, commit
  SHA, or retrieval date used for the address and adapter route.
- Each enabled split, merge, or redeem route must cite the verified adapter path.
- If a route cannot be proven, it stays out of scope for this PR.

## Validation

- Exact calldata equality for every enabled builder.
- Negative tests for every enabled builder must cover invalid config/address,
  invalid spender or operator, zero or invalid amount, unsupported route, and
  invalid route arguments as applicable to that builder.
- Standard validation commands from `docs/plans/README.md`.

## Residual Risk

- Correct calldata does not prove relayer execution or CLOB balance visibility.
- Current adapter routes may change. Each fixture must record the source used at
  implementation time.
