# PR 04: Consumer Adapter And Live Gate

## Summary

Document and implement the consumer integration boundary for this relayer fork.
The consumer may call this crate only through its relayer adapter and only after
the required dry-run and manual live gates are satisfied.

## In Scope

- Add consumer-facing integration guidance for `pm-adapters/relayer_http`.
- Define how the consumer maps this crate's DTOs into its own port/domain types.
- Add dry-run evidence requirements for nonce, signed batch, submit body, and
  transaction polling.
- Add manual live gate instructions with stop conditions and rollback steps.
- Require commit-SHA pinning or local path dependency only.

## Out Of Scope

- No strategy, risk, actor, or CLOB order implementation in this crate.
- No direct import of this crate by consumer domain, strategy, risk, or actor
  state modules.
- No live run by default.
- No branch-based production dependency.

## Target Consumer Boundary

- `pm-adapters/relayer_http` may depend on this crate.
- `pm-ports` exposes consumer-owned `RelayerPort` traits and data types.
- `pm-domain`, `pm-strategy`, `pm-risk`, `pm-actors`, and `pm-app` must not
  expose this crate's DTOs.
- CLOB order signing/posting remains owned by the official Rust CLOB SDK
  adapter with deposit wallet funder and `POLY_1271`.

## Gate Requirements

- Address identity separation is configured and visible in redacted logs.
- `WALLET-CREATE` is disabled unless deployment is explicitly needed.
- Fresh `WALLET` nonce is fetched immediately before signing.
- `WALLET` signing/submission is serialized per owner through an in-flight owner
  lock, nonce lease, or actor queue. A second batch for the same owner must not
  fetch/sign/submit with a nonce while another owner-scoped batch is unresolved.
- Signed batch digest and submit body are captured in redacted dry-run evidence.
- Polling handles new, executed, mined, confirmed, failed, invalid, and unknown
  states under the configured timeout/backoff policy. `STATE_NEW`,
  `STATE_EXECUTED`, and `STATE_MINED` remain pending; `STATE_CONFIRMED` is the
  only success state for relying on deposit-wallet action effects.
- Ambiguous timeout does not duplicate submit. After ambiguous submit, unknown
  state, or timeout for an owner, the adapter must reconcile the known
  `transactionID` before re-signing or submitting another batch for that owner.
- Rollback disables relayer mutation without disabling read-only CLOB/account
  observations.

## Validation

- Consumer adapter compile/test checks in the consumer repo.
- This repo's standard validation commands from `docs/plans/README.md`.
- Owner-scoped concurrency tests prove a second same-owner batch is blocked or
  queued until the first transaction reaches a reconciled terminal outcome.
- Pending-state tests prove `STATE_NEW`, `STATE_EXECUTED`, and `STATE_MINED`
  keep polling under the timeout policy and do not trigger success, duplicate
  submit, or re-signing.
- Manual live gate evidence is stored outside fixtures and without secrets.

## Residual Risk

- This PR may span this repo and the consumer repo. The implementation PR must
  state which repository owns each change.
- Tiny-value live validation remains operator-gated and must not run in CI.
