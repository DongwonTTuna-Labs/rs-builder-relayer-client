# Issues

## 2026-06-05 Task: session-start
- No task execution has been verified yet for this plan.
- Human gates exist after Task 1 (GATE A), Task 6 (GATE B), and Task 9 (GATE C). Executor must not merge PRs.
- `yaml-language-server` may be unavailable; use `actionlint` and static assertions for workflow validation.

## 2026-06-05 Task: 1-rs-revert-727
- `yaml-language-server` was not installed locally, so LSP diagnostics could not run for the three YAML workflow files; `actionlint` and static `rg` assertions passed instead.
- Pre-existing dirty notes under `.omo/notepads/codex-loop-live-remediation/` blocked branch switching and were stashed as `pre-task-preserve-live-remediation-notes`; they were unrelated to this RS Task 1 revert.
