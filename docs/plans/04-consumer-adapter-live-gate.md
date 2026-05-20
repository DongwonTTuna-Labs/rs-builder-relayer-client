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
- Signed batch digest and submit body are captured in redacted dry-run evidence.
- Polling handles mined, confirmed, failed, invalid, and unknown states.
- Ambiguous timeout does not duplicate submit.
- Rollback disables relayer mutation without disabling read-only CLOB/account
  observations.

## Validation

- Consumer adapter compile/test checks in the consumer repo.
- This repo's standard validation commands from `docs/plans/README.md`.
- Manual live gate evidence is stored outside fixtures and without secrets.

## Residual Risk

- This PR may span this repo and the consumer repo. The implementation PR must
  state which repository owns each change.
- Tiny-value live validation remains operator-gated and must not run in CI.
