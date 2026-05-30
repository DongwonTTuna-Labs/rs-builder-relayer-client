# Codex Review V3

This directory contains the implementation surface for the v3 Codex review workflow.

The live workflow should stay thin. It should call commands from this package, pass artifacts between jobs, and keep privileged GitHub write operations outside model jobs.

## Current Scope

V3 provides:

- deterministic JSON artifact IO
- fail-closed contract validation helpers
- an explicit OIDC relay action contract representation
- CLI entrypoints for stage00 through stage08
- a thin GitHub Actions workflow that passes JSON artifacts between stages

## OIDC Contract

The only authentication path reused from the legacy/current workflow is the short-lived OIDC relay credential flow:

```text
DongwonTTuna-Labs/home-server-infra/.github/actions/setup-codex-relay@main
```

The workflow must grant `id-token: write` only to jobs that exchange GitHub OIDC for a relay credential.

Relay tokens and raw OIDC JWTs must be masked and must not appear in artifacts, logs, prompts, or model outputs.

## Stage Commands

The intended stage command names are:

```text
stage00-resolve-gate
stage00-lifecycle
stage01-review
stage02-techlead
stage03-design
stage04-design-chief
stage05-fix-dispatch
stage06-fix-merge
stage07-push
stage08-reentry
```

## Stage00 Lifecycle Gate

`stage00-resolve-gate` writes the initial unresolved-thread gate. `stage00-lifecycle`
then consumes that gate with `thread-inventory.json` and writes
`stage00-lifecycle.json`.

Unforced unresolved threads are recorded as `deferred_thread_ids` and keep
`can_continue` true so stage01 can review them as normal work. Forced or
human-blocking gates stay blocking and keep `can_continue` false.

The workflow uploads this artifact as `codex-v3-stage00-lifecycle`.

Stage01 model prompts must include both `stage00-lifecycle.json` and
`thread-inventory.json` so unresolved thread lifecycle state is reviewed as part
of the current PR context instead of being left as an unused artifact.

## Review Summary Comment

Stage02 writes a marker-based sticky PR comment from the trusted workflow path
after `stage01-review.json` and `stage02-techlead.json` are validated. Model jobs
must not post comments directly.

## Validation Command Contract

Stage05 fix dispatch tasks may include a `test_plan`, and individual fix outputs
may include `tests`. Stage06 deduplicates those entries into
`validation_commands` in `stage06-fix-merge.json`.

Stage07 requires non-empty `validation_commands`. After applying the candidate
patch and running `git diff --check`, the trusted push job runs each validation
command from an allowlist before committing or pushing. Missing, unsupported, or
failing validation commands block the push.

## Non-Negotiable Rules

- Do not directly merge a PR.
- Do not run PR-head-controlled code in a privileged write path.
- Do not preserve the old workflow shape for compatibility.
- Do not treat local tests as final E2E verification.
- Do not mark the full goal complete without real OIDC mint, relay exchange, reviewer execution, and GPT Pro LGTM evidence.
