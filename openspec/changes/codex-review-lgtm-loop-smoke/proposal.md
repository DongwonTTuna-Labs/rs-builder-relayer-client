# Codex Review LGTM Loop Smoke

## Summary

Add an OpenSpec-backed smoke change that verifies Codex Review v3 can use PR title/body and linked OpenSpec artifacts as the source of truth for a review-to-fix loop.

The smoke intentionally leaves one safe documentation task incomplete: `docs/CODEX_REVIEW_LGTM_LOOP.md` is specified here but is not included in this PR. Codex Review should detect that mismatch, design a docs-only fix, and route the work into the autofix stages.

## Goals

- Prove the merged Codex Review workflow reads OpenSpec context from the PR body.
- Prove Stage01 through Stage04 model outputs include repository inspection evidence from `pr-head`.
- Prove the review/design pipeline promotes an executable OpenSpec-backed docs-only plan instead of stopping at generic human review.
- Prove Stage05 and later stages can prepare a fix for the missing docs artifact without touching Rust public API or workflow files.

## Non-Goals

- Do not change Rust crate public API.
- Do not change GitHub Actions workflow behavior in this smoke PR.
- Do not add live credentials, secrets, relay configuration, or runner configuration.
- Do not create `docs/CODEX_REVIEW_LGTM_LOOP.md` in the initial PR; it is the deliberate missing implementation target.

## Scope

The only expected human-authored files in this PR are OpenSpec artifacts under `openspec/`. The missing implementation is a single docs file:

- `docs/CODEX_REVIEW_LGTM_LOOP.md`

## Smoke Signal

Codex Review should compare this OpenSpec change to the PR tree and identify that `tasks.md` requires `docs/CODEX_REVIEW_LGTM_LOOP.md`, but the file is absent.
