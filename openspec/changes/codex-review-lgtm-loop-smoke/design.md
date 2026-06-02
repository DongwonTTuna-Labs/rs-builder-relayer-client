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
installation tokens directly when a write-capable stage is reached; push and issue
fallback are default actual write paths rather than repo-variable-gated dry-runs.

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
9. Stage07 validates the patch without any write token, then the trusted push job
   always mints a GitHub App token and attempts the same-repository PR push.
10. Stage08 records reentry after a successful push.
11. Stage09 creates or updates an idempotent issue fallback, without a separate
    enable flag, for missing OpenSpec, fork mutation, human-only blockers, or
    no-diff repeat failures.

## Required Fix Artifact

The missing implementation artifact is `docs/CODEX_REVIEW_LGTM_LOOP.md`. The generated
file must explain the intended OpenSpec-backed loop in repository documentation form:

- OpenSpec artifacts are the source of truth for the loop.
- Review must compare PR diff against proposal, design, tasks, and specs.
- Implementable OpenSpec-backed findings must flow to stage05 instead of stopping at
  generic human approval.
- Actual branch mutation is limited to same-repository PRs and GitHub App token writes.
- Push and issue fallback are default write paths once the trusted stage is reached; no separate enable variable is required.
- Rootless Codex action execution must avoid proxy sudo paths.
- Issue fallback is the terminal path for non-executable work.

## Safety Boundaries

- The smoke PR must not modify `.github/workflows`, `setup/codex-review`, `src`, or test
  fixtures.
- The only expected autofix target is `docs/CODEX_REVIEW_LGTM_LOOP.md`.
- Push is allowed only through a repository-scoped GitHub App installation token and same-repository PR head validation.
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
- Stage07 validation confirms the patch touches only `docs/CODEX_REVIEW_LGTM_LOOP.md`.
- After validation, the bot commits the missing docs file through the GitHub App
  token path and the follow-up run terminates as LGTM, noop, or no-fix rather than
  repeating the same patch.
