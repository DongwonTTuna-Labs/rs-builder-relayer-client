## Why

PR #37 already implemented the deposit-wallet relayer HTTP client and owner mutation state behavior, but there is no formal OpenSpec baseline for that behavior. This change records the current implemented contract so later PR 03 and PR 04 work can build from a clear spec without importing stale documentation or aspirational APIs.

## What Changes

- Add OpenSpec capability `deposit-wallet-relayer-client` with ADDED Requirements that document the current PR #37 behavior.
- Formalize the relayer URL, auth redaction, mutation permit, nonce lease, submit ordering, transaction read, wire DTO, EIP-712 signing, owner state, ambiguous submit, and reconciliation contracts as documentation only.
- Treat implemented Rust code, existing tests, and fixtures as the normative source of truth. Existing docs and plans are background only when they diverge from code, tests, or fixtures.
- Create a spec baseline future PR 03 and PR 04 changes can reference, without implementing those future changes in this OpenSpec change.
- Make no Rust, runtime, fixture, test, Cargo, dependency, or production behavior changes.

## Capabilities

### New Capabilities

- `deposit-wallet-relayer-client`: Documents the implemented deposit-wallet relayer HTTP client and mutation state contracts from PR #37, including current request validation, mutation gating, nonce lease handling, submit safety, transaction read semantics, credential redaction, signing validation, and reconciliation behavior.

### Modified Capabilities

None. There is no existing OpenSpec capability baseline for this behavior, and this change does not modify any existing spec-level requirements.

## Impact

- Affected OpenSpec change: `formalize-deposit-wallet-relayer-client`.
- Affected capability: `deposit-wallet-relayer-client`.
- Affected code: none.
- Affected tests or fixtures: none.
- Affected runtime behavior: none.
- Affected Cargo metadata or dependencies: none.
- Affected production readiness: none. This change documents PR #37 behavior only and does not make live production deposit-wallet execution ready or supported.

## Non-goals

- pUSD or CTF calldata builders are out of scope. PR 03 owns that future surface.
- Consumer adapter wiring and any live gate are out of scope. PR 04 owns that future surface.
- CLOB order flow is out of scope. Orders remain separate from this relayer client baseline.
- Transaction poll loop or poll policy behavior is out of scope and not implemented by PR #37. This proposal must not turn `poll_transaction` or bounded polling plans into a requirement.
- Stale docs or aspirational APIs are out of scope when they diverge from code, tests, or fixtures. This includes an owner-signer constructor shape and standalone `get_wallet_nonce(owner)` API shape that are not the implemented PR #37 contract.
- Live production execution, production readiness, or operator approval for deposit-wallet mutation is out of scope.
- Code, test, fixture, Cargo, dependency, source documentation, OpenSpec sync, OpenSpec archive, and runtime behavior changes are out of scope.
