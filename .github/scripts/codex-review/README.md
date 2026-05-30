# Codex Review V3

This directory contains the implementation surface for the v3 Codex review workflow.

The live workflow should stay thin. It should call commands from this package, pass artifacts between jobs, and keep privileged GitHub write operations outside model jobs.

Model jobs execute trusted helper code from the workflow checkout. When a model
needs to inspect PR-head files, the workflow checks out the PR head into a
separate `workspace` directory and points prompts at that path; `PYTHONPATH`
continues to reference the trusted workflow checkout.

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
may include `tests`. Stage03/Stage05 entries must be exact commands, with one
command per entry and no markdown, backticks, prose, or combined shell strings.
Stage06 deduplicates those entries and partitions them in
`stage06-fix-merge.json`: `validation_commands` contains only Stage07
push-safe commands, while `deferred_validation_commands` preserves full
PR-head validation commands such as Python test discovery, `cargo test`, and
`cargo clippy` for non-privileged CI or final human verification evidence.

Stage07 requires non-empty `validation_commands`. After applying the candidate
patch and running `git diff --check`, the trusted push job runs each validation
command from a narrow allowlist of trusted-ref workflow helper commands or fixed
safe commands before committing or pushing. The privileged write job must not run
PR-head-controlled code, including Python test discovery, `cargo test`, or
`cargo clippy`. Unsupported commands are not passed to Stage07; if every planned
validation is deferred, Stage06 falls back to `git diff --check` for push-safety.
Missing, unsupported, or failing Stage07 validation commands block the push.

The trusted push job rechecks the changed file set after validation and before
commit. The staged diff must still match the Stage06 `touched_files` list, and
there must be no unstaged or untracked changes; only that revalidated file list
is staged for commit.

## Non-Negotiable Rules

- Do not directly merge a PR.
- Do not run PR-head-controlled code in a privileged write path.
- Do not preserve the old workflow shape for compatibility.
- Do not treat local tests as final E2E verification.
- Do not mark the full goal complete without real OIDC mint, relay exchange, reviewer execution, and GPT Pro LGTM evidence.
