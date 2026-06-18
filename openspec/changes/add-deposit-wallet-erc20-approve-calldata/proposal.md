# Change Proposal: Deposit Wallet ERC20 Approve Calldata

## Why

This forward change adds one deterministic ERC20 `approve(address,uint256)` calldata builder for deposit-wallet `WALLET` batches. Grimoire will implement it later as a clean build-per-OpenSpec experiment, after the human-authored OpenSpec artifacts define the narrow target.

The intended outcome is a single builder that can express an ERC20 token approval as a `DepositWalletCall` without using venue addresses as truth and without routing through adapter flows. This proposal doesn't claim fixture proof, live readiness, production safety, or general Grimoire capability.

## What Changes

ADD a capability with a fallible `approve(address,uint256)` calldata builder returning `DepositWalletCall`; no other behavior.

The scope is exactly one deterministic calldata builder:

1. It accepts caller-supplied ERC20 token, spender, and amount inputs.
2. It returns one `DepositWalletCall` for the ERC20 `approve(address,uint256)` call.
3. The call target is the supplied ERC20 token address.
4. The call value is zero.
5. The call data is the `approve(address,uint256)` selector plus ABI-encoded spender and amount.
6. Builder failures are reported through the existing relayer error boundary.

No signing, nonce lookup, HTTP submit, transaction polling, wallet deployment, adapter routing, or production address selection changes are included.

## Capabilities

New capability: `deposit-wallet-erc20-approve-calldata`.

This capability covers only deterministic ERC20 `approve(address,uint256)` calldata construction returning `DepositWalletCall` for a deposit-wallet `WALLET` batch. It doesn't include execution or any other calldata surface.

## Impact

This task creates the initial OpenSpec proposal and schema metadata for change `add-deposit-wallet-erc20-approve-calldata`.

Later tasks may add human-authored `design.md`, delta spec, `tasks.md`, independent fixtures, a verification test, and a compiling stub before Grimoire receives implementation work. Those artifacts are not part of this task.

Any later verdict must stay narrow: evidence may show whether Grimoire followed a pinned OpenSpec in a bounded PR, not whether calldata is live ready or production safe.

## Non-goals

This change explicitly excludes:

1. pUSD adapter routing.
2. split, merge, and redeem calldata.
3. CTF and NegRisk behavior.
4. `setApprovalForAll` and conditional token approvals.
5. live execution, submit calls, transaction polling, and wallet deployment.
6. production-readiness claims, live-readiness claims, and claims that generated calldata is production safe.
7. real Polymarket production addresses as truth.
8. Any behavior beyond the one deterministic ERC20 `approve(address,uint256)` calldata builder returning `DepositWalletCall`.
