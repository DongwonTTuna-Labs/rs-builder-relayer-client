# Issues

## 2026-06-05 Task 6: HSI dry-run evidence and PR
- Markdown LSP diagnostics could not run because no `.md` LSP server is configured; `actionlint`, `git diff --check`, Python static assertions, and content scans were used instead.
- `DRY-RUN NOT TRIGGERED`: active HSI manual/debug and repository-dispatch entries require prior state pointers or default-branch context, so there was no safe no-state branch dry-run entry to materialize jobs without faking evidence.
- No remaining Task 6 blocker after PR creation. USER must merge GATE B; executor did not merge.

## 2026-06-05 Task: session-start
- No task execution has been verified yet for this plan.
- Human gates exist after Task 1 (GATE A), Task 6 (GATE B), and Task 9 (GATE C). Executor must not merge PRs.
- `yaml-language-server` may be unavailable; use `actionlint` and static assertions for workflow validation.

## 2026-06-05 Task: 1-rs-revert-727
- `yaml-language-server` was not installed locally, so LSP diagnostics could not run for the three YAML workflow files; `actionlint` and static `rg` assertions passed instead.
- Pre-existing dirty notes under `.omo/notepads/codex-loop-live-remediation/` blocked branch switching and were stashed as `pre-task-preserve-live-remediation-notes`; they were unrelated to this RS Task 1 revert.


## 2026-06-05 Task 2: HSI reusable serial rewrite
- `yaml-language-server` is unavailable in this environment, so YAML LSP diagnostics could not run; `actionlint` and Python static assertions were used instead.
- Task 2 intentionally leaves live model invocation, continuation dispatch, and App-token push unimplemented because those are assigned to later tasks.

## 2026-06-05 Task 3: HSI run-stage model invocation
- `yaml-language-server` remains unavailable, so YAML LSP diagnostics could not run for the changed HSI workflow; `actionlint`, `git diff --check`, and Python static assertions passed instead.
- Task 3 intentionally does not implement `push`/`commit-push` or GitHub App token minting; fix now emits `validated-fix.json` and related state for Task 4.

## 2026-06-05 Task 4: HSI real gated fix-push
- `yaml-language-server` remains unavailable, so YAML LSP diagnostics could not run for the changed HSI workflow; `actionlint`, `git diff --check`, and Python static assertions passed instead.
- Broken commit `3540af40d3ef8614ec978a4de90caa69b47f3b99` exists on GitHub but was not present in the local HSI fetch namespace; mechanics were inspected via GitHub API patch output and local helper source, then ported without its separate-job topology.
- Task 4 intentionally leaves global live gating/duplicate-marker/final redispatch tightening to Task 5 per plan; this task only makes the existing `push` stage perform real gated helper push inside `run-stage`.

## 2026-06-05 Task 4 correction: final blockers/issues
- No remaining Task 4 blocker after final verification.
- The first observed HSI commit for Task 4 had the right subject but extra commit body text; it was amended to the exact required single-message commit before evidence was written.
- `yaml-language-server` remains unavailable; final validation uses `actionlint`, `git diff --check`, helper command availability checks, and Python static assertions.

## 2026-06-05 Task 5: HSI live gating and bounded redispatch
- `yaml-language-server` remains unavailable for the changed HSI workflow; `actionlint`, `git diff --check`, and Python static assertions passed instead.
- No remaining Task 5 blocker after final verification. Task 6 still owns HSI PR creation; no PR was opened or merged here.
## 2026-06-05 Gate blocker: Task 7 cannot start
- Task 7 was marked `[~]` in `.omo/plans/codex-loop-serial-redesign.md` because required human merge gates are still open.
- RS GATE A PR #117 is OPEN and unmerged.
- HSI GATE B PR #21 is OPEN and unmerged.
- Task 7 requires the merged HSI SHA from GATE B; using PR head SHA or branch ref would violate the SHA-pin/no PR-head rule.
- Executor did not merge either PR.
## 2026-06-05 PR #117 conflict resolution
- HSI GATE B PR #21 is now MERGED at `b85e316c9023e1cd1995983dea0430a47648dc78` on home-server-infra `main`.
- RS PR #117 conflict was resolved and pushed as head `567bd9b2f2befc432446ceb50caadbe9f1ffac48`; PR #117 is OPEN and no longer DIRTY (`mergeStateStatus=UNSTABLE`).
- PR #117 diff is now only the three adapter workflows and preserves the temporary 727 noise-stop content: 727 pin, `secrets: inherit`, no `actions: read`, no explicit App-secret mapping, no `enable_live_autofix`.
- Full validation passed after conflict resolution: actionlint, static assertions, `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo test --workspace --all-features`, `git diff --check`.
- Next external gate: USER merges PR #117. After that, Task 7 can re-pin adapters to HSI merge SHA `b85e316c9023e1cd1995983dea0430a47648dc78` in a separate RS PR.

## 2026-06-05 Task 10 dry-run structural failure
- GATE C was verified: RS PR #118 is MERGED at `ccd08dfe107472aa64811673f09fa8e510af81a6`, and all three `main` codex-loop adapters pin HSI SHA `b85e316c9023e1cd1995983dea0430a47648dc78`.
- PR #98 pre-dispatch head was `a04e7eeb6598ea2d1c69837aaaa9ee02c97d216d`; marker `docs/CODEX_PUSH_SMOKE.md` was absent and spec `docs/CODEX_PUSH_SMOKE.spec.md` was present.
- Dispatched only dry-run manual adapter run `27001657903` with `dry_run=true`, `enable_live_autofix=false`, `stage=review`, `iteration=0`, and correlation `codex-serial-98-dryrun-20260605T072652Z`.
- Run `27001657903` completed `failure` with zero jobs/check runs (`jobs: []`, `latest_check_runs_count: 0`) and no logs, so Task 10 failed structurally and Task 11 remains blocked.
- Side-effect checks over available logs/metadata found no `openai/codex-action`, no relay-token mint, no App-token mint, no `commit-push`, no `repository_dispatch`, and PR #98 head remained unchanged at `a04e7eeb6598ea2d1c69837aaaa9ee02c97d216d`.
- Evidence written: `.omo/evidence/serial-redesign/task-10-head-before.txt` and `.omo/evidence/serial-redesign/task-10-dryrun-error.md`.

## 2026-06-05 Task 10 remediation follow-up
- `yaml-language-server` is still unavailable for YAML LSP diagnostics; `actionlint .github/workflows/codex-loop-manual-adapter.yml` and `git diff --check` passed instead.
- Branch dispatch did prove the changed branch workflow file can materialize jobs: run `27002333429` completed success with five reusable jobs present.
- This remediation does not remove the user merge gate. After the PR is merged to `main`, Task 10 should be retried on `main` before Task 11 proceeds.

