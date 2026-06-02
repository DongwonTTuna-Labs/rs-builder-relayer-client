# Design

## Current Architecture Under Test

Codex Review v3 is a pull-request-target workflow that runs trusted workflow/helper code from `main` while checking out the PR head separately for model inspection and autofix preparation.

The expected trust split is:

- Trusted workflow/helper checkout: base `main` / workflow SHA.
- PR inspection checkout: `pr-head`, using the PR head repository and SHA.
- Model execution: `openai/codex-action` with rootless direct-provider relay arguments.
- Write operations: GitHub App token only, never broad `GITHUB_TOKEN` write permission.

## OpenSpec Context Collection

The bootstrap stage should parse the PR title/body and find the same-repo OpenSpec path:

```text
openspec/changes/codex-review-lgtm-loop-smoke
```

It should collect:

- `openspec/config.yaml`
- `openspec/changes/codex-review-lgtm-loop-smoke/proposal.md`
- `openspec/changes/codex-review-lgtm-loop-smoke/design.md`
- `openspec/changes/codex-review-lgtm-loop-smoke/tasks.md`
- `openspec/changes/codex-review-lgtm-loop-smoke/specs/codex-review-lgtm-loop/spec.md`

## Review Expectations

Stage01 reviewers should inspect `pr-head`, not only the PR diff summary. Evidence should mention actual repository paths that informed the result, such as this OpenSpec change and relevant workflow/helper files.

The expected finding is that the OpenSpec task requiring `docs/CODEX_REVIEW_LGTM_LOOP.md` is incomplete.

## Design And Routing Expectations

Stage02 should route the missing docs artifact to design/fix instead of generic human review.

Stage03 should produce a closed candidate plan with:

- `edit_sequence` limited to `docs/CODEX_REVIEW_LGTM_LOOP.md`
- validation commands appropriate for docs/OpenSpec-only work
- acceptance criteria derived from the OpenSpec requirement
- `inspection_evidence` proving the model inspected repository files under `pr-head`

Stage04 should approve an executable OpenSpec-backed docs-only plan unless it finds a real non-executable blocker.

## Autofix Expectations

Stage05 should prepare a fix task whose allowed files are limited to:

- `docs/CODEX_REVIEW_LGTM_LOOP.md`

Stage06 and Stage07 should preserve semantic safety. If live push is enabled by repository policy, the patch should only add or update that docs file. If push is disabled or unavailable, Stage09 should create an idempotent issue with the OpenSpec source, attempted stages, and required follow-up.

## Rootless Runtime Constraint

The smoke must not require runner image Codex CLI installation or sudo access. The action should keep using direct provider relay arguments:

- `codex-args`
- `AI_RELAY_API_KEY`

It must not use:

- `openai-api-key`
- `responses-api-endpoint`
- Responses API proxy startup
- `sudo chmod`
- `sudo chown`
