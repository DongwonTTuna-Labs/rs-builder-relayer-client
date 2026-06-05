# Task 14 Startup Failure Remediation

## Failed Run

- Dry-run run: https://github.com/DongwonTTuna-Labs/rs-builder-relayer-client/actions/runs/26984003359
- Result observed before this remediation: `startup_failure` with zero jobs/logs/check-runs/artifacts, so no dry-run safety evidence was produced.

## Root Cause

The strongest confirmed static mismatch is the GitHub Actions permission ceiling between the RS reusable-workflow caller jobs and the pinned HSI reusable workflow.

- HSI reusable pin: `3540af40d3ef8614ec978a4de90caa69b47f3b99`
- HSI callee jobs request `actions: read`.
- RS caller jobs previously granted only `contents: write`, `pull-requests: write`, and `id-token: write`.
- GitHub reusable workflows cannot elevate `GITHUB_TOKEN` permissions beyond the caller job permission ceiling, so the callee's `actions: read` request can fail during startup validation before jobs are created.

Background findings also checked unsupported inputs/secrets and workflow output references as non-causes. HSI private reusable access appears organization-accessible, so the permission ceiling mismatch is the first minimal remediation.

## Changed Files

- `.github/workflows/codex-loop-review-adapter.yml`: added `actions: read` to the `codex-loop-review` reusable caller job permissions.
- `.github/workflows/codex-loop-manual-adapter.yml`: added `actions: read` to the `codex-loop-manual` reusable caller job permissions.
- `.github/workflows/codex-loop-dispatch-adapter.yml`: added `actions: read` to the `codex-loop-dispatch` reusable caller job permissions.
- `.omo/evidence/task-14-startup-failure-remediation.md`: recorded remediation evidence.
- `.omo/notepads/codex-loop-live-remediation/issues.md`: recorded Task 15 blocker until this remediation merges and dry-run rerun passes.

No pins, triggers, input forwarding, or secret mappings were intentionally changed. All three adapters still pin `3540af40d3ef8614ec978a4de90caa69b47f3b99` and explicitly map `CODEX_GITHUB_APP_ID` and `CODEX_GITHUB_APP_PRIVATE_KEY` from RS secrets.

## Validation

```bash
actionlint .github/workflows/codex-loop-review-adapter.yml .github/workflows/codex-loop-manual-adapter.yml .github/workflows/codex-loop-dispatch-adapter.yml
# passed with no output

git diff --check
# passed with no output

python3 static permission/pin/secret-mapping assertions
# static assertions passed for 3 adapter workflows

python3 changed-file secret-value scan
# secret-value scan passed for changed workflows/evidence/issue note

rg -n "actions: read|codex-loop-reusable.yml@3540af40d3ef8614ec978a4de90caa69b47f3b99|CODEX_GITHUB_APP_ID|CODEX_GITHUB_APP_PRIVATE_KEY|secrets: inherit" .github/workflows/codex-loop-review-adapter.yml .github/workflows/codex-loop-manual-adapter.yml .github/workflows/codex-loop-dispatch-adapter.yml
# confirmed actions: read, pinned HSI SHA, and explicit CODEX_GITHUB_APP_* secret mappings in all three adapters; no secrets: inherit matches
```

LSP diagnostics were attempted on changed YAML/Markdown files. YAML diagnostics could not run because `yaml-language-server` is not installed, and Markdown diagnostics could not run because no Markdown LSP is configured.

## Pull Request

PR URL: https://github.com/DongwonTTuna-Labs/rs-builder-relayer-client/pull/116

## Follow-up

Task 14 must be rerun after the remediation PR is merged because `workflow_dispatch` reads workflow definitions from the default branch. Task 15 live smoke remains blocked until the rerun dry-run passes.
