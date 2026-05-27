# PR 03: Deposit-Wallet pUSD And CTF Calldata Builders

## Summary

Add deposit-wallet-specific calldata builders for pUSD approvals, conditional
token approvals, and enabled CTF adapter operations. This PR only builds calldata
that can be submitted inside `WALLET` batches by the HTTP client introduced in
PR 02 after the caller supplies the required mutation gate.

## In Scope

- Add `src/deposit_wallet/calldata/` for deposit-wallet operation calldata.
- Add builders for pUSD ERC-20 approval from the deposit wallet.
- Add builders for conditional token ERC-1155 approval from the deposit wallet.
- Add split, merge, and redeem builders only for routes that can be verified
  against current Polymarket pUSD-native adapter behavior.
- Add golden fixtures under `tests/fixtures/deposit_wallet/calldata/`.

## Out Of Scope

- No live submission of generated calldata from this PR.
- No CLOB order posting or balance sync.
- No reuse of legacy USDC.e or direct CTF helpers without explicit fixture proof.
- No broad operation abstraction that hides target contract, value, and calldata.

## Target API

- `DepositWalletCalldataConfig`: chain id, pUSD contract address, CTF contract
  address, enabled adapter addresses, token decimals, amount unit metadata, and
  source metadata for each address/decimal. The config must also carry verified
  pUSD approval spender and CTF approval operator allowlists with source
  metadata for every allowed address.
- `build_pusd_approval_call(config, spender, amount) -> DepositWalletCall`,
  where `amount` is a unit-bearing pUSD amount type, not an ambiguous raw
  integer. `spender` must be present in the verified pUSD spender allowlist.
- `build_ctf_approval_for_all_call(config, operator, approved) ->
  DepositWalletCall`. `operator` must be present in the verified CTF operator
  allowlist before any `setApprovalForAll` calldata can be built.
- `build_split_position_call(config, ...) -> DepositWalletCall` only if the
  current adapter route is verified, using unit-bearing CTF position amount
  inputs where amounts are required.
- `build_merge_positions_call(config, ...) -> DepositWalletCall` only if the
  current adapter route is verified, using unit-bearing CTF position amount
  inputs where amounts are required.
- `build_redeem_positions_call(config, ...) -> DepositWalletCall` only if the
  current adapter route is verified, using unit-bearing CTF position amount
  inputs where amounts are required.

All builders must return explicit `DepositWalletCall { target, value, data }`
values from the provided config. Hidden defaults, global contract addresses,
chain inference, and fallback targets are not allowed.

## Fixture Requirements

- Fixtures must record contract address, method selector, argument values,
  encoded calldata, and source reference.
- Amount-bearing fixtures must record the human input value, raw ABI integer,
  unit name, decimals, and decimals source reference. pUSD, CTF position amount,
  and adapter-specific quantity units must not be inferred from bare integers.
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
- pUSD approval tests must reject every non-allowlisted spender, including
  otherwise well-formed attacker addresses.
- CTF approval tests must reject every non-allowlisted operator, including
  otherwise well-formed attacker addresses, because `setApprovalForAll` grants
  operator control over all matching positions.
- Negative tests must reject ambiguous raw amount inputs and amount/decimals
  source mismatches for every amount-bearing builder.
- Standard validation commands from `docs/plans/README.md`.

## Residual Risk

- Correct calldata does not prove relayer execution or CLOB balance visibility.
- Without these calldata builders, the guarded HTTP surface from PR 02 remains a
  mocked/read-only building block, not a complete live trading flow.
- Current adapter routes may change. Each fixture must record the source used at
  implementation time.
