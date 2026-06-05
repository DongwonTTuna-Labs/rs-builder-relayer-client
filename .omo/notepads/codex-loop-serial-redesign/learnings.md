# Learnings

## 2026-06-05 Task 6: HSI dry-run evidence and PR
- Opened HSI PR https://github.com/DongwonTTuna-Labs/home-server-infra/pull/21 from `feat/codex-loop-serial` to `main`; head SHA is `8386d87fcf826e835b11c4322e1d246306029878`.
- Added HSI documentation for the serial stage sequence `review -> design -> fix -> push -> loop`, live enablement via `dry_run: false` plus `enable_live_autofix: true`, secret names only, simple five-job topology, and rollback by flag-off/revert/re-pin.
- Final reusable validation passed with `actionlint`, `git diff --check`, Python static assertions, and docs secret-value scan.
- Dry-run execution was not triggered because no safe no-state HSI dry-run dispatch entry exists for this branch; manual and repository-dispatch entries require prior state pointers or default-branch dispatch context.

## 2026-06-05 Task: session-start
- Plan `codex-loop-serial-redesign` selected and active in `.omo/boulder.json`.
- Wave 1 is the only immediate parallel batch: Task 1 in RS repo and Task 2 in HSI checkout.
- Plan states RS Task 1 is a temporary noise-stop and must only touch the 3 adapter workflow files.
- Plan states HSI Task 2 must restore simple 5-job reusable topology from `727f713d`, with no matrix/fromJson fan-out.

## 2026-06-05 Task: 1-rs-revert-727
- PR #115 merge commit `f5eafd478627b033d7fac7e817d0fe61c06cdf9e` has first parent `dcb4185ec9acd999b1459c6d1e7b8c37a5d43ea3`; that parent is the source of truth for the temporary 727 adapter revert.
- Restoring the three RS adapter workflows from `dcb4185e` brings back `codex-loop-reusable.yml@727f713d2fc497b735ac17b575b74e282b09830e` and removes the abandoned `3540af40` pin.
- The restored pre-115 adapter shape uses `secrets: inherit` and no `actions: read`; later live-entry/explicit-secret changes are intentionally excluded for the noise-stop.


## 2026-06-05 Task 2: HSI reusable serial rewrite
- Replaced the HSI reusable workflow with a simple serial topology based on `727f713d`: `validate -> trust-and-stale-guard -> setup-relay -> run-stage -> finalize`.
- Static assertions in HSI confirmed exactly 5 top-level jobs, zero `fromJson`, and zero `strategy:` blocks.
- The rewritten contract includes optional `state_run_id` / `state_artifact_name`, `dry_run` default `true`, `enable_live_autofix` default `false`, and optional GitHub App secrets.
- Stage vocabulary for this redesign is `review|design|fix|push`; live model/push behavior remains skeleton-only for later tasks.

## 2026-06-05 Task 3: HSI run-stage model invocation
- `run-stage` now remains the only stage-executing job and branches internally for `review`, `design`, and `fix`; the 5-job topology is unchanged and static assertions still show zero `strategy:` and zero `fromJson`.
- Live stage model calls use `openai/codex-action@v1` with `responses-api-endpoint` from `setup-relay`; relay token minting is guarded by `dry_run == false`, `enable_live_autofix == true`, and `relay_configured == true`.
- Dry-run stage paths are helper-only: review/design/fix use codex-review deterministic default/result helpers and no token/model/push path.
- Stage handoff artifacts are deterministic: review produces `codex-loop-review-state-*`, design consumes review and produces `codex-loop-design-state-*`, fix consumes design and produces `codex-loop-fix-state-*` shaped for Task 4 push validation.

## 2026-06-05 Task 4: HSI real gated fix-push
- `run-stage` now implements `push` internally without adding jobs: it verifies the Task 3 fix-state bundle, consumes `dispatch-ledger.json`, `merged-fix.json`, `validated-fix.json`, and `semantic-safety.json`, and gates live push on validated fix readiness.
- GitHub App installation token minting uses `codex-review auth app-token --mode push` only on the live push path with `dry_run == false`, `enable_live_autofix == true`, trusted/non-fork/non-stale guards, and `needs_push_commit == true`.
- Real push uses `codex-review push commit-push`; commit author/trailers remain helper-produced, and workflow success requires helper `pushed == true`, `verified == true`, non-empty `updated_head_sha`, and `remote_head_sha == updated_head_sha`.
- Static assertions confirmed exactly five jobs, no `strategy:`, no `fromJson`, reachable `auth app-token`/`push commit-push`, and dry-run exclusion for token/push.

## 2026-06-05 Task 4 correction: final commit/evidence
- Prior Task 4 evidence/notepad text existed but the HSI checkout still required inspection and finalization; the dirty workflow diff was verified, tightened for helper failure convergence, and committed.
- Final HSI commit is `1d072f3d96924a0060ce474e67533207deb33620` with exact message `feat(codex-loop): real gated fix-push with attributable autofix commit`; `6aa29a1` is an ancestor.
- Evidence files written: `.omo/evidence/serial-redesign/task-4-push.txt` and `.omo/evidence/serial-redesign/task-4-dryrun-safe.txt`.

## 2026-06-05 Task 5: HSI live gating and bounded redispatch
- Finalized HSI reusable live gates without changing the five-job topology: `validate`, `trust-and-stale-guard`, `setup-relay`, `run-stage`, and `finalize` remain the only jobs, with no `strategy:` and no `fromJson`.
- All live paths now require both `dry_run == false` and `enable_live_autofix == true` plus trusted/non-fork/non-stale prerequisites: relay token minting, every `openai/codex-action@v1` call, GitHub App token minting, `commit-push`, and live continuation dispatch.
- `finalize` now emits actual live continuation via GitHub `repository_dispatch` endpoint after `guard-dispatch`, appends the bounded dispatch ledger, carries state artifact metadata and updated post-push head SHA, and stops on dry-run/live-disabled/terminal/max-iteration/duplicate-marker conditions.
- Final Task 5 HSI commit is `f4449863030cca0efbb2191fcdbe98f93559e9e5` with exact message `feat(codex-loop): gate live behind flags + preserve guards + bounded redispatch`; evidence files written: `.omo/evidence/serial-redesign/task-5-gating.txt` and `.omo/evidence/serial-redesign/task-5-guards.txt`.

## 2026-06-05 Task 7: RS adapter re-pin
- Re-pinned the three RS codex-loop adapters from the temporary 727 scaffold SHA to HSI merge commit `b85e316c9023e1cd1995983dea0430a47648dc78` after confirming HSI PR #21 and RS PR #117 were merged.
- Reusable caller jobs now use explicit `CODEX_GITHUB_APP_ID` and `CODEX_GITHUB_APP_PRIVATE_KEY` mappings instead of `secrets: inherit`, with `actions: read` added to the existing permission ceiling.

## 2026-06-05 Task 10 remediation: manual dispatch typing
- Manual adapter zero-job startup was reproduced on `main` as a caller-boundary type issue candidate because only `workflow_dispatch` inputs were forwarded directly into HSI `workflow_call` number/boolean inputs; automatic review adapter had already materialized jobs with the same HSI SHA.
- Remediation branch `ci/codex-loop-manual-dispatch-fix` wraps `pr_number`, `iteration`, `max_iterations`, `dry_run`, and `enable_live_autofix` with `fromJSON(format('{0}', inputs.<name>))` while leaving string inputs unchanged.
- Branch dry-run run `27002333429` materialized five reusable jobs and completed successfully, proving the zero-job startup failure is fixed on the remediation branch; `Setup Codex Relay` and `Run Stage` were skipped by reusable gating, not by graph materialization failure.

## 2026-06-05 Task 10 trusted core ref update
- RS PR #119 branch adapters now pin HSI PR #22 merge SHA `95686f21da9e839bff1956dd0809cdfc02e3529c` in `uses:` and pass the same value as `trusted_core_ref`.
- Manual `workflow_dispatch` number/boolean conversions remained intact with `fromJSON(format('{0}', inputs.<name>))`; review remains `dry_run: true`, and manual live defaults remain off.
- Branch manual dry-run `27004426779` materialized reusable jobs and carried `INPUT_TRUSTED_CORE_REF=95686f21da9e839bff1956dd0809cdfc02e3529c`; it completed success after stopping as `untrusted-requester`, with PR #98 head unchanged.
