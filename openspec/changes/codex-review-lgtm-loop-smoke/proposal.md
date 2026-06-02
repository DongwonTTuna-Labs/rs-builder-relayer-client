# Codex Review LGTM Loop Smoke

## Motivation

Codex Review v3 now has an OpenSpec context collector, rootless model execution through
`openai/codex-action`, stage04 routing that can promote executable OpenSpec-backed
plans, stage05-stage08 autofix plumbing, and stage09 issue fallback. This change creates
a repository-native OpenSpec artifact that can be used to verify the whole PR feedback
loop from the current `main` workflow.

The smoke must prove that the review automation can treat OpenSpec artifacts as the
source of truth, find a missing docs-only implementation task, design a bounded fix,
prepare an autofix patch, and push the fix through the GitHub App
token path without a manual enable variable.

## Goals

- Provide a concrete OpenSpec change under `openspec/changes/codex-review-lgtm-loop-smoke`.
- Make the PR body link to that OpenSpec change so `openspec-context.json` is populated.
- Leave `docs/CODEX_REVIEW_LGTM_LOOP.md` absent in the initial PR on purpose.
- Require Codex Review to create that missing docs file as the safe implementation fix.
- Keep the smoke docs-only so exported Rust interfaces, workflow behavior, and
  venue-facing behavior remain unchanged.

## Non-Goals

- Do not change GitHub Actions workflow logic in this smoke PR.
- Do not change exported Rust interfaces, generated fixtures, venue-facing execution
  logic, or deposit-wallet live execution gates.
- Do not require push or issue-fallback enable variables in repository contents.
- Do not merge the smoke PR automatically.

## Expected Initial Gap

The initial PR intentionally does not include `docs/CODEX_REVIEW_LGTM_LOOP.md`.
Codex Review should identify that gap from `tasks.md` and the spec delta, then produce
a docs-only candidate fix.
