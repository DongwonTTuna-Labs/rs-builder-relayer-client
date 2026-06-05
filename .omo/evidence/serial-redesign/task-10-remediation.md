# Task 10 Remediation: Manual Adapter Zero-Job Startup

Date: 2026-06-05
Branch: `ci/codex-loop-manual-dispatch-fix`
Base: `origin/main` at PR #118 merge commit `ccd08dfe107472aa64811673f09fa8e510af81a6`
Workflow: `.github/workflows/codex-loop-manual-adapter.yml`

## Root Cause

Hypothesis implemented and branch-validated: the manual `workflow_dispatch` adapter forwarded number/boolean dispatch inputs directly into the SHA-pinned HSI reusable workflow. GitHub accepted the dispatch event but rejected the reusable `workflow_call` graph before job materialization when those forwarded values did not satisfy the called workflow's declared number/boolean input types.

- Who: GitHub Actions workflow graph validator at the reusable workflow call boundary.
- What: `inputs.pr_number`, `inputs.iteration`, `inputs.max_iterations`, `inputs.dry_run`, and `inputs.enable_live_autofix` were forwarded without explicit number/boolean conversion.
- When: Task 10 dry-run on `main`, run `27001657903`, failed in about three seconds with `jobs: []` and no logs.
- Why: `workflow_dispatch` input delivery and `workflow_call` input validation have different typing semantics; the reusable expects typed `number`/`boolean` values.
- How: server-side graph validation failed before any reusable job could be materialized.

## Exact Change

Caller-only minimal fix in `.github/workflows/codex-loop-manual-adapter.yml`:

- Converted numeric inputs with `fromJSON(format('{0}', inputs.<name>))`: `pr_number`, `iteration`, `max_iterations`.
- Converted boolean inputs with `fromJSON(format('{0}', inputs.<name>))`: `dry_run`, `enable_live_autofix`.
- Preserved string inputs as-is: `head_sha`, `base_ref`, `stage`, `correlation_id`, `requested_by`.
- Preserved HSI SHA pin `b85e316c9023e1cd1995983dea0430a47648dc78`.
- Preserved explicit App secret mapping and `actions: read` permission.
- Preserved manual dry-run default `true` and live autofix default `false`.

## Static Validation

- `actionlint .github/workflows/codex-loop-manual-adapter.yml`: passed with no output.
- `lsp_diagnostics` on `.github/workflows/codex-loop-manual-adapter.yml`: attempted, but `yaml-ls` is configured and `yaml-language-server` is not installed in this environment.
- `git diff --check`: passed with no output.

## Branch Dry-Run Dispatch

Command shape used: `gh workflow run .github/workflows/codex-loop-manual-adapter.yml --ref ci/codex-loop-manual-dispatch-fix` with PR #98, head SHA `a04e7eeb6598ea2d1c69837aaaa9ee02c97d216d`, `base_ref=main`, `stage=review`, `iteration=0`, `max_iterations=5`, `dry_run=true`, and `enable_live_autofix=false`.

Run URL: https://github.com/DongwonTTuna-Labs/rs-builder-relayer-client/actions/runs/27002333429

Final run metadata:

- Event: `workflow_dispatch`
- Branch: `ci/codex-loop-manual-dispatch-fix`
- Head SHA: `45cf5d317d34541a4cea3c4afe16bb3cbc478cd9`
- Status/conclusion: `completed` / `success`

Materialized jobs:

- `Codex loop manual (dry-run default) / Validate Inputs`: success
- `Codex loop manual (dry-run default) / Trust And Stale Guard`: success
- `Codex loop manual (dry-run default) / Setup Codex Relay`: skipped
- `Codex loop manual (dry-run default) / Run Stage`: skipped
- `Codex loop manual (dry-run default) / Finalize Loop`: success

Important observation: the branch dispatch did exercise the changed branch workflow file and no longer failed with zero jobs. The later `Setup Codex Relay` and `Run Stage` jobs were skipped by the reusable's gating, while `Finalize Loop` succeeded. This is enough to validate the startup-materialization remediation, not a full Task 10 stage execution retry on `main`.

Side-effect check: PR #98 remained open at head SHA `a04e7eeb6598ea2d1c69837aaaa9ee02c97d216d` after the dry-run. Live continuation steps in `Finalize Loop` were skipped, including App-token mint and `repository_dispatch`.

## Next Gate

Open an RS PR from `ci/codex-loop-manual-dispatch-fix` to `main`. Do not merge in this task. After user merge, retry Task 10 dry-run on `main` before unblocking Task 11.
