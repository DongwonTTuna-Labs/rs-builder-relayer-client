# Design: Deposit Wallet ERC20 Approve Calldata

## Context

Change `add-deposit-wallet-erc20-approve-calldata` exists to define one bounded OpenSpec target before Grimoire writes code. The proposal scopes this to a deterministic ERC20 `approve(address,uint256)` calldata builder for deposit-wallet `WALLET` batches. It doesn't claim fixture proof, live readiness, production safety, or broader Grimoire capability.

This design closes the spec gap that would otherwise let implementation choices drift. It records the calldata source of truth, the anti-circularity contract for fixtures and tests, the integration shape with deposit-wallet batch calls, and the exact path Grimoire may edit.

## Goals / Non-Goals

Goals:

1. Define a narrow ERC20 `approve(address,uint256)` builder for deposit-wallet batches.
2. Require caller supplied `token`, `spender`, and `amount` inputs.
3. Require output as `DepositWalletCall { target, value, data }` where `target == token` and `value == 0`.
4. Require a fallible `Result<_, RelayerError>` boundary.
5. Require fixture and test evidence before any calldata correctness claim.

Non-Goals:

1. No pUSD adapter routing.
2. No CTF, NegRisk, conditional-token, split, merge, redeem, transfer, or adapter calldata.
3. No `setApprovalForAll` support.
4. No signing, nonce lookup, HTTP submit, transaction polling, wallet deployment, or live execution.
5. No production address selection and no real Polymarket production address as truth.
6. No change to legacy Safe/Proxy operations or public root exports unless a later spec explicitly allows it.
7. No live-readiness, production-safety, or general Grimoire capability claim.

## Decisions

### Source of truth & ABI provenance

The calldata target is the canonical ERC20 function `approve(address,uint256)` with selector `0x095ea7b3`. The ABI argument order is exactly `(spender, amount)`.

The official Polymarket TS SDK ERC20 approve encoder is the external source to cite for the fixture oracle and any correctness claim. The local legacy pattern in `src/operations/approve.rs:8-22` and selector constant in `src/contracts.rs` may be read as a reference only. They aren't an independent oracle, and Grimoire must not edit them for this experiment.

ABI provenance must be recorded with these fields:

1. `source repo+commit/ref`: ERC20 standard canonical signature `approve(address,uint256)`; cross-reference Polymarket/builder-relayer-client `main` commit `9122f6fb1856f1ecfe4406685bfa19a2c5a7b290`, `src/abis/erc20Abi.ts` approve ABI plus examples using viem `prepareEncodeFunctionData`/`encodeFunctionData`. Fixture bytes were generated from the standard signature, not from SDK output.
2. `ABI hash`: `sha256(canonical signature 'approve(address,uint256)')=9f0bb8a9deafa4881b85d434c1fccdb064584a4809dbcba57a86f5b4c559246b`.
3. `retrieval date`: 2026-06-16.

Until those fields are filled by the fixture task, this design doesn't assert a verified upstream commit SHA or ABI hash. The `token` and `spender` values are caller INPUTS, not venue truth, not production truth, and not a source for selecting Polymarket addresses.

### Anti-circularity

The fixture oracle must be independently generated, frozen, and Grimoire-immutable before Grimoire implements the builder body. Grimoire may read the OpenSpec artifacts and frozen test expectations, but it must not edit the fixture, the frozen test, or the oracle generation record.

The verification test must assert both of these facts:

1. Byte-equality between builder output data and the frozen fixture bytes.
2. An independent ABI-decode of the produced calldata back into selector `0x095ea7b3`, `spender`, and `amount`.

The frozen vectors must include multiple cases with distinct dummy token and spender addresses. Amount edges must include `0`, a normal nonzero value, and `U256::MAX`. At least one HIDDEN post-run vector must be generated AFTER Grimoire's commit, using a dummy token, dummy spender, and amount not present in the frozen fixture. That hidden vector is for post-run verification only, so Grimoire can't tune the body to the visible fixture set.

### Integration

The builder output is `DepositWalletCall { target, value, data }`, matching `src/deposit_wallet/types.rs:28-37`.

The integration contract is:

1. `target == token`.
2. `value == 0`.
3. `data` is selector `0x095ea7b3` followed by ABI encoded `(spender, amount)`.
4. The builder must be fallible with `Result<_, RelayerError>`.
5. The returned call composes into `calls: Vec<DepositWalletCall>` for deposit-wallet batch construction.
6. Batch resource limits still apply, 256 calls and 1 MiB calldata, as defined in `src/deposit_wallet/signing.rs:25-26`.
7. The existing fallible batch boundary remains `try_build_wallet_batch_request_with_signature` in `src/deposit_wallet/requests.rs:35-68`.

This builder doesn't sign, fetch nonce, submit HTTP requests, poll transactions, or pick venue addresses. It only produces one call object that a later batch flow may include after that flow performs its own validation and signing.

### Implementation and Grimoire write scope

Grimoire's autonomous implementation authority is exactly the body of `build_erc20_approve_call` in `src/deposit_wallet/calldata.rs`. Grimoire must not autonomously change the signature, module boundaries, fixtures, tests, OpenSpec files, docs, policy files, Cargo metadata, auth/signing/nonce/http/submit modules, legacy operations, or request builders outside that function body.

That Grimoire authority limit is not a repository-wide PR file list. Human-authored OpenSpec, policy, fixture, frozen-test, Cargo, and documentation changes may be included when they define, authorize, or verify this capability before or alongside implementation. Those supporting human-authored changes expand the allowed PR scope only for this OpenSpec change; they do not expand Grimoire's autonomous write authority.

Any Grimoire-authored change outside the single approved builder body invalidates the Grimoire experiment verdict. The verdict may only state whether Grimoire followed the pinned OpenSpec inside the bounded path. It must not state production safety, live readiness, or broad Grimoire capability.
