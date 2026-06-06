# Task 10 RS read-token mapping evidence

## Scope
- Branch: `ci/codex-loop-trusted-core-read-token`
- Base: `origin/main` at `f788dd484f92c0d14d2727ab09faf70e621e1927`
- PR: https://github.com/DongwonTTuna-Labs/rs-builder-relayer-client/pull/120
- PR state at creation: `OPEN`, not merged
- Recorded workflow head SHA at PR creation: `d341326aaf77fd3531f1ca32c1a68b4778fdf04b`
- Note: this evidence file is added after PR creation, so the final PR head advances beyond the recorded workflow head.

## HSI verification
- HSI PR #23: https://github.com/DongwonTTuna-Labs/home-server-infra/pull/23
- HSI PR #23 state: `MERGED`
- HSI PR #23 merge commit: `97321913ee853aff0ef1476490899b133f968523`
- HSI commit resolution: `gh api repos/DongwonTTuna-Labs/home-server-infra/commits/97321913ee853aff0ef1476490899b133f968523 --jq .sha` returned `97321913ee853aff0ef1476490899b133f968523`.

## Changed files
- `.github/workflows/codex-loop-review-adapter.yml`
- `.github/workflows/codex-loop-manual-adapter.yml`
- `.github/workflows/codex-loop-dispatch-adapter.yml`
- `.omo/evidence/serial-redesign/task-10-rs-read-token-mapping.md`
- `.omo/notepads/codex-loop-serial-redesign/issues.md`
- `.omo/notepads/codex-loop-serial-redesign/learnings.md`

## Adapter assertions
Python static assertions passed for all three adapter workflows:
- `uses:` is `DongwonTTuna-Labs/home-server-infra/.github/workflows/codex-loop-reusable.yml@97321913ee853aff0ef1476490899b133f968523`.
- `trusted_core_ref` is `97321913ee853aff0ef1476490899b133f968523`.
- Old HSI SHA `95686f21da9e839bff1956dd0809cdfc02e3529c` is absent.
- `secrets: inherit` is absent.
- `actions: read` remains present.
- Existing explicit App secret mappings remain present:
  - `CODEX_GITHUB_APP_ID: ${{ secrets.CODEX_APP_ID }}`
  - `CODEX_GITHUB_APP_PRIVATE_KEY: ${{ secrets.CODEX_APP_PRIVATE_KEY }}`
- New read-token mapping is present:
  - `CODEX_TRUSTED_CORE_READ_TOKEN: ${{ secrets.CODEX_TRUSTED_CORE_READ_TOKEN }}`
- Manual stage options remain exactly `review`, `design`, `fix`, `push`.
- Manual number/boolean conversions remain `fromJSON(format('{0}', inputs.<name>))` for `pr_number`, `iteration`, `max_iterations`, `dry_run`, and `enable_live_autofix`.
- Review adapter remains dry-run only and does not pass `enable_live_autofix`.
- Manual live defaults remain off.
- Dispatch dry-run default remains `${{ github.event.client_payload.dry_run != false }}`.

## Validation results
- YAML LSP diagnostics: attempted on all three changed workflow files; `yaml-language-server` is not installed, so diagnostics could not run.
- Markdown LSP diagnostics: attempted on changed notepad files; no `.md` LSP server is configured.
- `actionlint .github/workflows/codex-loop-*.yml`: passed.
- Python static adapter assertions: passed.
- `cargo fmt --all --check`: passed.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: passed.
- `cargo test --workspace --all-features`: passed, 118 tests passed plus doc-tests with 0 tests.
- `git diff --check`: passed before evidence creation; rerun after final evidence commit.

## Secret-name check
- Command used: `gh secret list --repo DongwonTTuna-Labs/rs-builder-relayer-client --json name --jq 'any(.[]; .name == "CODEX_TRUSTED_CORE_READ_TOKEN")'`
- Result: `false`
- Interpretation: RS repository secret `CODEX_TRUSTED_CORE_READ_TOKEN` is absent by name.
- No secret value was requested, read, or printed.

## Runtime status
- Task 10 main dry-run was not dispatched in this task.
- Runtime Task 10 remains blocked until this PR is merged and the RS repository secret `CODEX_TRUSTED_CORE_READ_TOKEN` is configured.
- Task 11 live remains blocked until Task 10 dry-run succeeds on main.
