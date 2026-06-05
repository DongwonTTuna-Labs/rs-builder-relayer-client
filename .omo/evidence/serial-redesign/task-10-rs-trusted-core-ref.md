# Task 10 RS Trusted Core Ref Evidence

Date: 2026-06-05
Branch: `ci/codex-loop-manual-dispatch-fix`
RS PR: https://github.com/DongwonTTuna-Labs/rs-builder-relayer-client/pull/119

## HSI PR #22 Verification

- HSI PR #22: https://github.com/DongwonTTuna-Labs/home-server-infra/pull/22
- PR state: `MERGED`
- Merge commit from `gh pr view 22 --repo DongwonTTuna-Labs/home-server-infra --json state,mergeCommit,baseRefName,url`: `95686f21da9e839bff1956dd0809cdfc02e3529c`
- GitHub commit API resolved `repos/DongwonTTuna-Labs/home-server-infra/commits/95686f21da9e839bff1956dd0809cdfc02e3529c` and returned the same 40-hex SHA.
- HSI `main` branch API returned head SHA `95686f21da9e839bff1956dd0809cdfc02e3529c`, confirming the merge commit is on the default branch.

## Files Changed

- `.github/workflows/codex-loop-review-adapter.yml`
- `.github/workflows/codex-loop-manual-adapter.yml`
- `.github/workflows/codex-loop-dispatch-adapter.yml`
- `.omo/evidence/serial-redesign/task-10-rs-trusted-core-ref.md`
- `.omo/notepads/codex-loop-serial-redesign/learnings.md`
- `.omo/notepads/codex-loop-serial-redesign/issues.md`

## Adapter Static Assertions

Python static assertions passed for all three adapters:

- Each adapter uses `DongwonTTuna-Labs/home-server-infra/.github/workflows/codex-loop-reusable.yml@95686f21da9e839bff1956dd0809cdfc02e3529c`.
- Each adapter passes `trusted_core_ref: 95686f21da9e839bff1956dd0809cdfc02e3529c` in `with:`.
- Obsolete refs `b85e316c9023e1cd1995983dea0430a47648dc78`, `727f713d`, and `3540af40` are absent from the three adapter files.
- `secrets: inherit` is absent.
- Explicit `CODEX_GITHUB_APP_ID` and `CODEX_GITHUB_APP_PRIVATE_KEY` secret mapping is present in all three adapters.
- `actions: read` is present in all three adapter job permission ceilings.
- Manual adapter retained `fromJSON(format('{0}', inputs.<name>))` conversions for `pr_number`, `iteration`, `max_iterations`, `dry_run`, and `enable_live_autofix`.
- Review adapter remains automatic dry-run with `dry_run: true`.

## Local Validation

Passed:

- `actionlint .github/workflows/codex-loop-*.yml`
- Python static adapter assertions
- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --all-features` with 116 tests/doc-tests passing and no failures
- `git diff --check`

LSP limitation:

- `lsp_diagnostics` for all three YAML workflow files could not run because `yaml-language-server` is not installed. The configured server is `yaml-ls`, but the command was unavailable.

## PR #119 Branch Update

- Pushed branch `ci/codex-loop-manual-dispatch-fix` to update PR #119.
- PR #119 remained `OPEN` after the push.
- PR #119 head after workflow update push: `20ec177f412ff690c164174eb2c44ecfc32e9c14`.
- No merge was performed.

## Automatic Review Adapter Runtime

Run: https://github.com/DongwonTTuna-Labs/rs-builder-relayer-client/actions/runs/27004405557

- Trigger: `pull_request_target` on PR #119 after pushing commit `20ec177f412ff690c164174eb2c44ecfc32e9c14`.
- Conclusion: `failure`.
- Jobs materialized: `Validate Inputs` success, `Trust And Stale Guard` success, `Setup Codex Relay` failure, `Finalize Loop` success, `Run Stage` skipped.
- Failing step: `Setup Codex Relay / Checkout trusted Codex loop core`.
- Log evidence showed the called workflow line as `Uses: DongwonTTuna-Labs/home-server-infra/.github/workflows/codex-loop-reusable.yml@b85e316c9023e1cd1995983dea0430a47648dc78`, not the branch-updated `95686f21...` ref.
- Log inputs did not include `trusted_core_ref`, and checkout attempted `ref: ccd08dfe107472aa64811673f09fa8e510af81a6`.
- Failure text: `remote: Repository not found.` and `fatal: repository 'https://github.com/DongwonTTuna-Labs/home-server-infra/' not found`, with git exit code 128.

Root-cause record:

- Who: GitHub Actions `pull_request_target` automatic review adapter execution for RS PR #119, using the workflow definition from the base branch rather than the PR branch update.
- What: The automatic run still executed the old HSI reusable pin `b85e316c...` without `trusted_core_ref`, then the HSI checkout attempted the RS merge/base SHA `ccd08dfe...` as an HSI ref and failed.
- When: 2026-06-05T08:30:29Z to 2026-06-05T08:31:57Z, run `27004405557`.
- Why: `pull_request_target` resolves the caller workflow from the protected/base branch context, so this PR's branch workflow edits are not exercised by the automatic PR run until merged or otherwise present on base.
- How: Run logs show `Uses: ...@b85e316c...`, no `trusted_core_ref` input, and `actions/checkout` fetching `DongwonTTuna-Labs/home-server-infra` at RS SHA `ccd08dfe...` before failing.

Conclusion: the branch diff is updated correctly, but the requested automatic PR runtime proof cannot be green on PR #119 because the automatic `pull_request_target` run did not use the PR branch's adapter changes.

## Branch Manual Dry-Run Runtime

Run: https://github.com/DongwonTTuna-Labs/rs-builder-relayer-client/actions/runs/27004426779

Inputs:

- `pr_number=98`
- `head_sha=a04e7eeb6598ea2d1c69837aaaa9ee02c97d216d`
- `dry_run=true`
- `enable_live_autofix=false`
- `stage=review`
- `iteration=0`
- `base_ref=main`
- `correlation_id=codex-serial-98-dryrun-trusted-core-20260605T083102Z`

Results:

- Conclusion: `success`.
- Jobs materialized: `Validate Inputs` success, `Trust And Stale Guard` success, `Setup Codex Relay` skipped, `Finalize Loop` success, `Run Stage` skipped.
- Finalize environment confirmed `INPUT_TRUSTED_CORE_REF=95686f21da9e839bff1956dd0809cdfc02e3529c`.
- The dry-run stopped as `untrusted-requester`; no live continuation dispatch, App-token mint, relay token mint, model action, or push path ran.

## PR #98 Head Comparison

- Before manual dry-run: `a04e7eeb6598ea2d1c69837aaaa9ee02c97d216d`
- After manual dry-run: `a04e7eeb6598ea2d1c69837aaaa9ee02c97d216d`
- Result: unchanged; no PR #98 branch mutation observed.
