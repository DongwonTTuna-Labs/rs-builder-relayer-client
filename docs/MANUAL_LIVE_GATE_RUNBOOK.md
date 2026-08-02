# Manual Live Gate Runbook

This runbook defines the operator-controlled procedure for the first
tiny-value live validation of the deposit-wallet relayer path. Every recorded
artifact must remain redacted, and every gate requires a distinct operator
decision before the next step begins.

## Scope and non-goals

This procedure covers one bounded, tiny-value validation of the deposit-wallet
relayer `WALLET-CREATE` or `WALLET` path after all fork acceptance work is
complete. It covers scope confirmation, identity review, dry-run review,
permit issuance, an explicitly approved submit, result classification, and
rollback.

It does not cover CLOB orders, trading strategy, automated execution, or
routine bot operations. Those remain governed by the consumer repository's
`docs/rust-migration/OPERATIONS_RUNBOOK.md` and
`docs/rust-migration/LIVE_SAFETY.md`; this document supplements those runbooks
only for the deposit-wallet relayer live gate. PBRSDK-27 owns execution of this
procedure. PBRSDK-26 performs no live call.

PBRSDK-27 lifecycle status record: `docs/LIVE_VALIDATION_DECISION.md` (`blocked`, `stopped`, `executed`).

## Preconditions

Do not start this procedure until all 10 items in
[`DEPOSIT_WALLET_RELAYER_DESIGN.md`'s Live enablement checklist](DEPOSIT_WALLET_RELAYER_DESIGN.md#live-enablement-checklist)
are checked. This runbook does not repeat or replace that checklist.

The campaign also has two live gate blockers recorded by
`SM-CALLDATA-CTF-ROUTES`:

- `src/contracts.rs` contains an incorrect legacy
  `MERGE_POSITIONS_SELECTOR` value.
- `src/operations/redeem.rs::redeem_neg_risk_positions` misuses an argument in
  the legacy route.

The reviewed deposit-wallet path does not use either legacy route. They remain
in the same repository, however, so the operator must confirm in the GATE 1
record that this validation cannot select either legacy path. Any uncertainty
stops the procedure before mutation.

## Secret handling

Never put a secret value in this document or in evidence, logs, PR text,
screenshots, terminal captures, or chat. Inject each configured value only
through its secret-manager reference. Record only the following key names, not
their values:

| Key name | Handling rule |
| --- | --- |
| `RELAYER_API_KEY` | Inject only by secret-manager reference; never leave the value in logs, PRs, or screenshots. |
| `RELAYER_API_KEY_ADDRESS` | Inject only by secret-manager reference; never leave the value in logs, PRs, or screenshots. |
| `DEPOSIT_WALLET_OWNER_PRIVATE_KEY` | Inject only by secret-manager reference; never leave the value in logs, PRs, or screenshots. |
| `DEPOSIT_WALLET_OWNER_ADDRESS` | Inject only by secret-manager reference; never leave the value in logs, PRs, or screenshots. |
| `DEPOSIT_WALLET_ADDRESS` | Inject only by secret-manager reference; never leave the value in logs, PRs, or screenshots. |

Treat addresses as sensitive evidence too: use only an abbreviated form such
as `0x1234...ABCD`, never a complete address. Do not attach private-key or API
key values, `Bearer` header values, PEM blocks, signatures, typed signing
payloads, full calldata, or a replayable submit body. If an operator interface
cannot suppress those values, stop before capturing evidence.

## Identity separation check

1. Resolve the relayer-auth identity, owner signer, and deposit-wallet/funder
   from their separate secret-manager references inside the protected runtime.
   Do not copy their values into the gate record.
2. Construct `DepositWalletIdentityConfig::try_new` with the reviewed contract
   configuration. It verifies that the supplied wallet matches the address
   derived from the owner. A construction failure is a stop condition.
3. Record only the redacted identity summary and the overlap keys returned by
   `overlaps()`.
4. Review every reported overlap. `overlaps()` reports equal identity pairs;
   it does not reject them. The operator must explicitly decide whether each
   overlap is intended for this deployment and record that decision at GATE 2.
   An unexplained overlap stops the procedure.

## Approval gates

No gate authorizes a later gate. The named operator must make and record a new
decision at every step.

**GATE 1 — requires operator approval**

- What is approved: the network, one deposit-wallet relayer validation, the
  pre-filled tiny-value bounds, and exclusion of both legacy drifted routes.
- Risk without approval: the operation could target the wrong network, market,
  call path, or value boundary.
- Record location: the protected live-gate record; PR evidence contains only a
  redacted record reference and decision time.

**GATE 2 — requires operator approval**

- What is approved: the chain configuration, owner-derived wallet/funder
  relationship, relayer-auth separation, and every reported identity overlap.
- Risk without approval: the wrong signer, wallet, funder, or authentication
  identity could be used.
- Record location: the protected identity/config review; PR evidence contains
  only redacted identities and overlap keys.

**GATE 3 — requires operator approval**

- What is approved: the complete PBRSDK-24 dry-run artifact schema v1, bounded
  calls, nonce/deadline context, signing summary, submit summary, polling plan,
  rollback plan, and redaction result.
- Risk without approval: a malformed, stale, over-broad, or disclosure-bearing
  payload could advance to live authorization.
- Record location: the protected dry-run review; the PR receives only the
  redacted artifact described below.

**GATE 4 — requires operator approval**

- What is approved: creation of one scoped, unexpired `Live`
  `RelayerMutationPermit` for the reviewed operation, owner, chain, evidence,
  and approval record.
- Risk without approval: mutation authority could be issued without a human
  decision bound to the reviewed dry-run.
- Record location: the protected permit-issuance record; PR evidence records
  only mode, scope, expiry status, and reference byte lengths.

**GATE 5 — requires operator approval**

- What is approved: one submit attempt by the PBRSDK-27 operator within the
  approved scope and time window.
- Risk without approval: a real mutation could be dispatched or duplicated
  without final operator intent.
- Record location: the protected execution record; the PR receives only
  redacted dispatch and observation evidence.

**GATE 6 — requires operator approval**

- What is approved: the final classification as confirmed, stopped, rolled
  back, or reconciliation-required, together with the cited observations.
- Risk without approval: a pending, failed, ambiguous, or unrecognized result
  could be treated as success or followed by another mutation.
- Record location: the protected final-decision record; the PR receives a
  redacted verdict and evidence references only.

`RelayerMutationPermit::try_new` requires both `evidence_ref` and
`operator_approval_ref` and rejects an empty reference, a reference containing
control characters, or one longer than 256 bytes
(`src/deposit_wallet/http/mutation.rs`). A permit therefore cannot be created
without an approval-record reference. This is only a syntactic capability
boundary: the code does not verify that the reference identifies a genuine
approval, and even the one-character reference `"x"` passes validation. The
GATE records and operator procedure, not the constructor, establish the
reference's truthfulness.

## Tiny-value bounds

The operator must fill every value below before GATE 1. A blank cell blocks
approval.

| Bound | Operator-approved value | Approval record reference |
| --- | --- | --- |
| Maximum amount |  |  |
| Target market |  |  |
| Permitted call type |  |  |

## Stop conditions

### source drift

- Detection: compare the current dependency revision, wire-shape documentation,
  source-matrix rows, and reviewed fixtures with the GATE 3 artifact. Any
  mismatch is drift.
- Immediate action: stop mutation, preserve redacted observations, and go
  immediately to [Rollback](#rollback).

### unexpected state

- Detection: the relayer returns a transaction state that is not documented by
  the reviewed state policy or fixtures.
- Immediate action: do not classify success or retry; go immediately to
  [Rollback](#rollback) and begin read-only reconciliation.

### unknown state

- Detection: the fork classifies the observed transaction state as `Unknown`.
- Immediate action: stop all further mutation and go immediately to
  [Rollback](#rollback) with the redacted raw-state classification preserved.

### timeout

- Detection: the approved finite polling policy reaches its attempt or elapsed
  bound without `STATE_CONFIRMED`.
- Immediate action: do not resubmit; go immediately to [Rollback](#rollback)
  and reconcile the existing intent first.

### missing transactionID

- Detection: dispatch may have occurred, but the submit result contains no
  required transaction identifier.
- Immediate action: treat the outcome as ambiguous, do not resubmit, and go
  immediately to [Rollback](#rollback) for reconciliation.

### wrong owner/funder

- Detection: the redacted observed owner or funder does not match the GATE 2
  summary, or owner-to-wallet derivation validation fails.
- Immediate action: stop observation of that scope as an approval source and go
  immediately to [Rollback](#rollback); do not issue a corrected mutation.

### redaction failure

- Detection: manual review or the bounded audit finds a secret value, complete
  address, full calldata, signature, authentication material, or replayable
  submit body in an artifact.
- Immediate action: stop publication, quarantine the artifact in the protected
  evidence system, and go immediately to [Rollback](#rollback). Do not attach
  the artifact to a PR.

## Rollback

1. Call `disable_mutation()` on the active client immediately. The latch is
   one-way for that client instance and every clone because they share the same
   `Arc<AtomicBool>` (`src/deposit_wallet/http.rs`).
2. Keep read-only observation available under its existing authorization. The
   latch does not remove address derivation or read-permit validation, so use
   those read-only capabilities to preserve and reconcile the observed state.
3. If a submit may already be in flight, never resubmit automatically. First
   reconcile the existing intent using authoritative polling when an identifier
   is known and the established ambiguous-outcome process otherwise.
4. Treat the rollback boundary precisely. The latch covers only the active
   client instance and its clones. The public
   `DepositWalletRelayerClient::new_with_mutation_enabled` constructor can
   create another live-enabled client, including in the same process.
   Persistent rollback therefore depends on operator-controlled deployment and
   process policy preventing a new client, not on the type system.
5. Resume only through a newly recorded `operator_approval_ref` and a complete
   repeat of GATE 1 through GATE 6. Prior approval and prior permit authority
   cannot authorize the new client or a new submit.

## Redacted evidence checklist

Attach only checked, redacted evidence to the PR. The checklist maps directly
to the PBRSDK-24 dry-run artifact schema v1.

- [ ] **Artifact identity:** include `schema_version`, `operation`, and `mode`;
  must not include a credential value or an unreviewed mode.
- [ ] **Identity and chain:** include `identity` only in its abbreviated form
  plus `chain_id`; must not include a complete relayer-auth, owner, or wallet
  address.
- [ ] **Freshness context:** include the reviewed `nonce` and
  `deadline_unix` fields from the redacted artifact; must not include a private
  key, signing seed, or protected runtime configuration.
- [ ] **Call summary:** include `calls` with only the schema's redacted targets,
  call kinds, counts, and lengths; must not include a complete target address,
  full calldata, or a replayable call payload.
- [ ] **Signing summary:** include `signing` only in its schema-defined redacted
  form; must not include a signature value, typed signing payload, private key,
  or authentication material.
- [ ] **Submit summary:** include `submit_body` only in its schema-defined
  redacted form; must not include a replayable body, full address, full calldata,
  signature, or authentication header.
- [ ] **Polling and post-submit observation:** include `polling` states,
  bounded attempts, timing classification, and redacted correlation; must not
  include secrets, complete addresses, full calldata, or signatures.
- [ ] **Rollback evidence:** include `rollback` with the mutation-disable
  decision and continued read-only observation status; must not include a new
  submit instruction or credential material.
- [ ] **Redaction evidence:** include `redaction` with only audit outcome counts
  and categories; must not echo any matched value into the report.
- [ ] **Permit summary:** include operation/mode/chain/expiry status and only
  the byte lengths of `evidence_ref` and `operator_approval_ref`; must not
  include either reference's contents or any permit Debug output copied from an
  unreviewed environment.
- [ ] **Complete dry-run artifact:** confirm all schema v1 fields above are
  present and were reviewed at GATE 3; must not substitute screenshots or raw
  terminal output for the redacted artifact.
- [ ] **Final verdict and basis:** include the GATE 6 classification and
  redacted evidence links; must not claim success from a pending, ambiguous,
  stopped, or reconciliation-required observation.

## What this runbook does not prove

This document is a procedure, not a safety guarantee. It does not prove actual
relayer behavior, nonce validity at dispatch time, market state, finality,
consumer wiring, or the absence of concurrent external mutations. PBRSDK-26
contains no live execution evidence.

The permit constructor proves only that bounded reference strings were
supplied. It does not prove that `operator_approval_ref` names a genuine
operator decision; that limitation remains an operational control.

The audit in `tests/live_gate_runbook_test.rs` checks only this runbook for
`0x`/`0X`-prefixed ASCII hexadecimal runs and the specified literal PEM and
bearer-token markers. It cannot detect a 40-hex value without the prefix, a
UUID-shaped key, or a base64 value. Passing the audit mechanically confirms
only that the designated value shapes are absent from this file; it does not
prove that the document or external evidence contains no secret.
