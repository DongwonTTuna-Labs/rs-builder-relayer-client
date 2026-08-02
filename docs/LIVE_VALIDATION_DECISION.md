# Live Validation Decision

<!-- live-validation-status: blocked -->

## Decision

Live validation was not performed.

The status is a lifecycle fact, not a success verdict. `blocked` means execution
never started and no mutation-endpoint request was sent. `stopped` means
execution started and a request may have been sent, but `STATE_CONFIRMED` was
not observed. `executed` means `STATE_CONFIRMED` was observed, regardless of
whether later balance, allowance, or rollback checks passed.

Using `executed` for the confirmation observation avoids a fourth lifecycle
status when a later balance, allowance, or rollback check fails. Those later
results belong in the `final verdict` and `rollback state` evidence rows.

## What was not done

- No production `RelayerMutationPermit` was created.
- No submit was attempted.
- No request was sent to a relayer mutation endpoint.

## Blockers

1. No operator approval reference exists because no approval record was
   created. This prevents GATE 1 of `docs/MANUAL_LIVE_GATE_RUNBOOK.md` from
   being satisfied.
2. An approved network, funded deposit wallet, and owner-signer credential
   were not provided to this work environment. This is the intended result of
   the secret-handling invariant, not a defect. It prevents the approved
   network scope in GATE 1 and the identity review in GATE 2 from being
   satisfied.
3. The tiny-value bounds table in `docs/MANUAL_LIVE_GATE_RUNBOOK.md` has blank
   values for maximum amount, target market, and permitted call type. The
   runbook states that a blank cell blocks GATE 1 approval, so GATE 1 cannot be
   satisfied.

## What this means for the release

- Production mutation is not claimed as a validated feature.
- The documentation does not overstate live support.
- Dry-run paths and read paths are unaffected by this decision.

## Existing evidence for the disabled default

The existing test
`src/deposit_wallet/http/tests.rs::mutation_is_default_deny_for_wallet_create_and_wallet_batch_before_http`
constructs a client with `DepositWalletRelayerClient::new`. It supplies
otherwise valid `Live` permits with matching operations, matching owners,
chain 137, and `u64::MAX` expiry, then observes rejection for both wallet-create
and wallet-batch submissions and zero requests at the loopback server.

The distinguishing evidence for default denial is the error message
`relayer mutation is disabled for this client`. A blocked error alone is not
default-deny evidence: scope and expiry failures from
`validate_mutation_permit` also make
`is_deposit_wallet_mutation_blocked()` return true.

The existing integration target `tests/mutation_rollback_boundary_test.rs`
covers the path after `disable_mutation()`: live mutation is rejected before
network dispatch, while read-permit validation remains active.

## Operator evidence template

| Field | Value | Redaction |
| --- | --- | --- |
| operator approval reference | UNFILLED | Record only a non-secret record identifier; omit protected contents. |
| approved network | UNFILLED | Record only the approved network name and chain number. |
| approved max amount | UNFILLED | Record only the approved bounded amount and unit. |
| approved market | UNFILLED | Use an approved market label or abbreviated identifier. |
| approved call type | UNFILLED | Record only the approved call-type name. |
| fresh nonce check | UNFILLED | Record the observation result without a replayable request. |
| permit mode and scope | UNFILLED | Record mode and scope names; abbreviate addresses to the first 6 and last 4 characters. |
| permit expiry | UNFILLED | Record a bounded time or expiry classification without permit internals. |
| submit time | UNFILLED | Record a coarse UTC time without authentication material. |
| transactionID | UNFILLED | Keep only a non-replayable abbreviated correlation value. |
| terminal state | UNFILLED | Record only the normalized state name, never the raw response. |
| balance and allowance observation | UNFILLED | Record only redacted before-and-after classifications and bounded amounts. |
| rollback state | UNFILLED | Record only the mutation-latch and reconciliation classification. |
| final verdict | UNFILLED | Record only one allowed verdict value. |

Allowed `final verdict` values are `CONFIRMED`, `STOPPED`, `ROLLED_BACK`, and
`RECONCILIATION_REQUIRED`.

## What this record does not prove

The audit guarantees one canonical status marker, the required presence or
absence of status-specific sections, the designated literal in `## Decision`,
the limited evidence-table shape, and the absence of the specified secret
patterns. More precisely, it fails when the status and canonical Decision
sentence, or the status and designated section/table schema, disagree.

It does not prove:

- that the contents of `## What was not done`, `## Why the run was stopped`, or
  `## Blockers` agree with the status;
- that `## Blockers` contains any item or that an item cites a GATE;
- that prose other than the canonical sentence agrees with the status;
- anything about fenced-code content, which is a non-normative example outside
  the rule guarantees;
- arbitrary Markdown rendering: the CommonMark-parsed record is constrained to
  a narrow subset of paragraphs, headings, lists, code blocks, tables, inline
  code, and the one-line status-marker HTML comment; headings are limited to
  exactly one H1 on the document's first line followed only by H2 headings on
  the known section allowlist, and lists cannot be nested; headings and tables
  must be top-level blocks and cannot appear inside a list item or table cell;
  the canonical Decision sentence must be a paragraph rather than a list item;
  raw HTML, images, links, emphasis, and all other syntax are rejected rather
  than interpreted; another lifecycle status's canonical sentence is forbidden
  in every paragraph, heading, and table cell, while the secret scan bypasses
  the parser and applies to the complete raw source;
- that the status wording in `README.md` agrees with this marker;
- that filled values reflect actual observations, because only their form is
  checked;
- that blocker descriptions are factually true;
- that evidence stored outside this repository contains no secret; or
- the absence of unprefixed hexadecimal, UUID, or base64 forms.

## How to supersede this record

An actual validation must change the status marker and fill every evidence
Value cell. Changing only the marker or only the fields fails the audit. The
status-specific sections must also be updated: a `stopped` record removes
`## What was not done` and `## Blockers` and adds
`## Why the run was stopped`; an `executed` record removes all three of those
conditional sections.

`## What was not done` is forbidden for `stopped` because its claims say that
no permit was created, no submit was attempted, and no mutation request was
sent. Representative stopped outcomes such as `missing transactionID` or
`timeout` occur after a submit attempt. Keeping both would allow a record to
claim that no request was sent after a live request may have occurred.

Use exactly one lifecycle marker in the superseding record:

```text
<!-- live-validation-status: blocked -->
<!-- live-validation-status: stopped -->
<!-- live-validation-status: executed -->
```
