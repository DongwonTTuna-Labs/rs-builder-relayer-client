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
- Signed batch digest and a field-level submit summary are captured in redacted
  dry-run evidence. Raw signatures, auth material, raw signed payloads, and full
  replayable submit bodies must never be stored in logs, fixtures, PR comments,
  screenshots, or artifacts. Evidence may store a payload hash, target/value
  summaries, method selectors, owner/deposit-wallet addresses, nonce metadata,
  and explicit redaction markers.
- Polling handles new, executed, mined, confirmed, failed, invalid, and unknown
  states under the configured timeout/backoff policy. `STATE_NEW` and
  `STATE_EXECUTED` remain pending. `STATE_MINED` and `STATE_CONFIRMED` satisfy
  relayer polling completion; consumer code that requires stronger finality can
  keep polling or require `STATE_CONFIRMED` before relying on wallet effects.
- Ambiguous timeout does not duplicate submit. After ambiguous submit, unknown
  state, or timeout for an owner, the adapter must reconcile the known
  `transactionID` before re-signing or submitting another batch for that owner.
- If a submit timeout happens before a `transactionID` is known, the adapter
  must store redacted owner-scoped blocked state keyed by payload hash or
  equivalent idempotency evidence. That state prevents a second same-owner
  nonce fetch, signature, or submit until manual or authoritative reconciliation
  proves the original payload was not accepted or has reached a terminal
  outcome.
- Rollback disables relayer mutation without disabling read-only CLOB/account
  observations.

## Validation

- Consumer adapter compile/test checks in the consumer repo.
- This repo's standard validation commands from `docs/plans/README.md`.
- Owner-scoped concurrency tests prove a second same-owner batch is blocked or
  queued until the first transaction reaches a reconciled terminal outcome.
- Owner-scoped concurrency tests must use deterministic synchronization, such
  as barriers, channels, or a controlled mock relayer. The first batch must be
  held pending while the test proves the second same-owner batch does not call
  nonce fetch, signing, or submit before reconciliation.
- Pending-state tests prove `STATE_NEW` and `STATE_EXECUTED` keep polling under
  the timeout policy and do not trigger success, duplicate submit, or re-signing.
- Id-less submit timeout tests prove payload-hash blocked state is written,
  same-owner mutation is denied, and recovery requires explicit reconciliation.
- Dry-run evidence tests prove signatures, authorization headers, raw signed
  typed data, and replayable submit bodies are redacted while non-secret hashes
  and field-level summaries remain available for review.
- Manual live gate evidence is stored outside fixtures and without secrets.

## Residual Risk

- This PR may span this repo and the consumer repo. The implementation PR must
  state which repository owns each change.
- Tiny-value live validation remains operator-gated and must not run in CI.
