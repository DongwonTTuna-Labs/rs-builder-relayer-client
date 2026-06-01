# Tasks: Codex Review LGTM Loop Smoke

## OpenSpec Artifacts

- [x] Add `openspec/config.yaml`.
- [x] Add proposal, design, tasks, and spec artifacts for `codex-review-lgtm-loop-smoke`.
- [x] Link this change from the PR body.

## Intentionally Missing Implementation

- [ ] Add `docs/CODEX_REVIEW_LGTM_LOOP.md` with the documented OpenSpec-backed LGTM loop.

## Expected Automation Work

- [ ] Codex Review detects the missing docs file from this OpenSpec change.
- [ ] Codex Review creates a stage03 plan that only allows `docs/CODEX_REVIEW_LGTM_LOOP.md`.
- [ ] Codex Review dry-runs a docs-only patch while side-effect gates are disabled.
- [ ] After human inspection, a controlled push run may add the docs file through the GitHub App token path.
- [ ] A follow-up run stops without repeating the same docs patch.
