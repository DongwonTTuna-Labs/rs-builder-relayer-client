# Design: Codex Review LGTM Loop Smoke

## Current Main Workflow Baseline

The merged Codex Review v3 orchestrator runs from `.github/workflows/codex-review-orchestrator.yml`
on `pull_request_target` and `workflow_dispatch`. The workflow checks out trusted base
code, checks out helper code from `github.workflow_sha`, builds event and PR context,
and now collects OpenSpec context into:

- `codex-review-artifacts/event/openspec-context.json`
- `codex-review-artifacts/event/openspec-context.md`
- the appended repository docs context consumed by model stages

Model jobs use `openai/codex-action` with relay-provided `codex-args` and
`AI_RELAY_API_KEY`. They do not use `openai-api-key`, `responses-api-endpoint`, or the
Responses proxy path that required sudo hardening. Trusted write stages mint GitHub App
installation tokens only when side-effect gates are enabled.

## Smoke Flow

1. The PR body links `openspec/changes/codex-review-lgtm-loop-smoke`.
2. `bootstrap_event` resolves PR metadata and reads this OpenSpec change from the PR head
   when the trusted base checkout does not contain the new files yet.
3. Stage01 reviewers compare the PR diff and OpenSpec context and find that
   `docs/CODEX_REVIEW_LGTM_LOOP.md` is required but absent.
4. Stage02 tech lead must route the implementable docs-only finding to design rather
   than use `needs_human` as a generic escape.
5. Stage03 creates a closed candidate design plan with:
   - `openspec_backed: true`
   - `edit_sequence` limited to `docs/CODEX_REVIEW_LGTM_LOOP.md`
   - `tests` that include docs-safe validation and repository-required checks
   - `acceptance_criteria` derived from this OpenSpec change
6. Stage04 must approve an executable OpenSpec-backed plan for fix unless a real
   non-executable blocker exists.
7. Stage05 prepares agent tasks and prompts.
8. Stage06 merges model patch output.
9. Stage07 validates the patch without any write token and, if `CODEX_REVIEW_ENABLE_PUSH`
   is true, pushes only through a GitHub App token.
10. Stage08 records reentry after a successful push.
11. Stage09 creates or dry-runs an idempotent issue fallback only for missing OpenSpec,
    fork mutation, human-only blockers, or no-diff repeat failures.

## Required Fix Artifact

The missing implementation artifact is `docs/CODEX_REVIEW_LGTM_LOOP.md`. The generated
file must explain the intended OpenSpec-backed loop in repository documentation form:

- OpenSpec artifacts are the source of truth for the loop.
- Review must compare PR diff against proposal, design, tasks, and specs.
- Implementable OpenSpec-backed findings must flow to stage05 instead of stopping at
  generic human approval.
- Actual branch mutation is limited to same-repository PRs and GitHub App token writes.
- Side-effect gates remain off by default.
- Rootless Codex action execution must avoid proxy sudo paths.
- Issue fallback is the terminal path for non-executable work.

## Safety Boundaries

- The smoke PR must not modify `.github/workflows`, `setup/codex-review`, `src`, or test
  fixtures.
- The only expected autofix target is `docs/CODEX_REVIEW_LGTM_LOOP.md`.
- Push is allowed only after dry-run artifacts show a docs-only patch.
- If the PR comes from a fork, the workflow must not mutate the branch and must route to
  stage09 issue fallback.
- Sensitive credentials, OIDC tokens, relay credentials, and GitHub App credentials
  must not be printed, committed, or pasted.
- The generated smoke document must use neutral wording for credential and
  low-level transaction safety, rather than enumerating guard-triggering examples.

## Acceptance Criteria

- `openspec-context.json` has `present: true` and includes this change directory.
- Stage04 produces or validates `approved_for_fix` for the docs-only plan.
- Stage05 prepare is not skipped for a same-repository PR.
- Stage07 dry-run patch touches only `docs/CODEX_REVIEW_LGTM_LOOP.md`.
- With push enabled after review, the bot commits the missing docs file and the follow-up
  run terminates as LGTM, noop, or no-fix rather than repeating the same patch.
