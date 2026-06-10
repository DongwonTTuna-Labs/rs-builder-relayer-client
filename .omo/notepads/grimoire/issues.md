# Issues — grimoire

## 2026-06-09 — grimoire attune task 1 live blockers

- Live `gh workflow run grimoire-attune.yml` was not executed: this task forbids dispatch unless already safe/authenticated, and the local environment has no `AI_RELAY_API_KEY` available for the ai-relay smoke. There is no run URL or live log yet; next verification requires manual `workflow_dispatch` after the repository secret is configured.
- Because task 1 also forbids `GITHUB_TOKEN`, GitHub App token, and PAT git/gh auth, the workflow uses tokenless checkout. That preserves the attune no-auth constraint, but private-repo checkout may fail before the control-plane checks unless the runner/repo can be checked out without credentials; later grimoire auth/trusted-controller tasks must solve authenticated private checkout separately.

## 2026-06-09 — grimoire attune task 2 config verification caveats

- Live successful ai-relay smoke was not run in this task because no `AI_RELAY_API_KEY` value was available for safe use; the task verified parse/config load, OMO primary agent discovery, and missing-key negative behavior instead.
- `opencode run --format json` with both `AI_RELAY_API_KEY` and `CODEX_LB_LOCAL_API_KEY` unset emitted a JSON API error for the missing key but returned process exit code 0; later workflow code must parse error events or require success sentinels.
- Pinned `bunx oh-my-openagent@4.8.1 doctor --status` was blocked locally by `Failed to link @ast-grep/cli: EEXIST`; package pin resolution and `bunx oh-my-openagent@4.8.1 --version` still confirmed the 4.8.1 pin, and `opencode agent list` was used for plugin/default-agent load evidence.

## 2026-06-09 - grimoire task 3 secret registration blockers

- `gh secret list` succeeded with local auth, but the repository secret list did not include `AI_RELAY_API_KEY`. Until that secret is registered externally, grimoire ai-relay model calls must fail closed.
- `gh secret list` did not include `GRIMOIRE_PAT`, and the local environment did not expose `CODEX_LOOP_PAT`. Until one PAT path is available in CI, grimoire checkout, push, comment, and label mutation must fail closed.
- Secret registration and PAT scope selection are external GitHub repository or organization operations. This docs-only task recorded the gap but did not create or modify any GitHub secrets.

## 2026-06-09 - grimoire task 4 live-run caveats

- Live Prometheus no-ask, Atlas auto-finish, and Momus always-on model execution were not run because local `AI_RELAY_API_KEY` is missing and task 3 recorded the repository relay secret as missing. The task 4 evidence is static/config-load evidence only, not a live ai-relay success.
- Local `GRIMOIRE_PAT` and `CODEX_LOOP_PAT` were also missing, so no GitHub mutation, workflow dispatch, or live PR loop was attempted for this config-only task.

## 2026-06-10 - grimoire task 5 live review blockers

- Live OMO team-mode/security-review execution was not run in this environment. The available tool surface did not expose team-mode tools, and the local environment still does not provide `AI_RELAY_API_KEY`; the task therefore records only the runtime contract plus local deterministic mock evidence.
- `.github/scripts/grimoire-review.sh` requires an explicit `GRIMOIRE_TEAM_MODE_ENABLED=1` readiness assertion before real mode can call `opencode run`. Set that only after CI healthcheck proves team-mode tools are available; otherwise real mode writes blocked JSON and exits non-zero.

## 2026-06-10 - grimoire task 6 live design blockers

- Live Prometheus design execution was not run because local `AI_RELAY_API_KEY` is still unavailable. The design stage therefore records deterministic mock evidence and a real-mode fail-closed contract, not a live ai-relay success.
- `.github/scripts/grimoire-design.sh` requires `GRIMOIRE_DESIGN_READY=1` and `GRIMOIRE_TEAM_MODE_ENABLED=1` before real mode can call `opencode run`; these readiness assertions should be set only after CI proves non-interactive Prometheus plus Metis/Oracle/Momus planning can run safely.

## 2026-06-10 - grimoire task 7 posting caveat

- Task 7 verified local render artifacts only. The script deliberately does not run `gh pr comment` because repository `GRIMOIRE_PAT` and local `CODEX_LOOP_PAT` remain unavailable from prior checks, and GitHub mutation paths must fail closed until a safe PAT path is explicitly present.

## 2026-06-10 - grimoire task 8 live fix blockers

- Live Atlas/Hephaestus fix execution was not run locally because `AI_RELAY_API_KEY`, `GRIMOIRE_PAT`, and `CODEX_LOOP_PAT` remain unavailable from prior checks. The fix stage therefore records deterministic mock evidence plus a real-mode fail-closed contract, not a live ai-relay success.
- `.github/scripts/grimoire-fix.sh` requires `GRIMOIRE_FIX_READY=1` and a non-empty declared PR-touched file set before real mode may call `opencode run`; missing or ambiguous scope writes blocked JSON and exits before mutation.

## 2026-06-10 - grimoire task 9 live verification blockers

- Live F1-F4 opencode verification was not run locally because `AI_RELAY_API_KEY` remains unavailable from prior checks. Task 9 therefore records deterministic mock evidence plus a real-mode fail-closed contract, not a live ai-relay success.
- `.github/scripts/grimoire-verify.sh` requires `GRIMOIRE_VERIFY_READY=1` before real mode may call `opencode run`; this should only be set after CI proves the non-interactive F1-F4 verification prompt and allowed read/QA command surface are safe.
- Missing or malformed `.omo/grimoire/verdict.json`, invalid enum values, absent notes, and any `REJECT` are deliberate non-approval states for Task 11. They must not be converted into success by driver fallback logic.

## 2026-06-10 - grimoire task 10 workflow blockers

- Live `.github/workflows/grimoire.yml` PR execution was not dispatched from this task. The workflow is event-driven on `pull_request`, and this task forbids live dispatch, comments, labels, push, or merge.
- Eligible ready PR runs are expected to fail closed until Task 11 adds executable `.github/scripts/grimoire-cast.sh`; the missing driver is an intentional blocker, not a successful review-loop result.
- Local/repo PAT availability remains externally configured: `GRIMOIRE_PAT` or runner `CODEX_LOOP_PAT` must exist before an eligible non-skipped run can checkout or mutate GitHub state. The workflow records no secret values and fails closed when both PAT sources are absent.

## 2026-06-10 - grimoire task 11 live blockers

- Live Task 11 execution was not run against a throwaway PR because local `AI_RELAY_API_KEY`, `GRIMOIRE_PAT`, and `CODEX_LOOP_PAT` remain unavailable from prior grimoire checks. The driver therefore records deterministic mock evidence and a real-mode missing-secret fail-closed blocker, not live ai-relay/GitHub success.
- Real non-dry-run spec-gap posting and bot push require a PAT source plus grounded PR metadata (`GRIMOIRE_PR_NUMBER` for comments and head repo/ref for pushes). Missing values are fail-closed states and must not be treated as successful comment or push evidence.
- The full synchronize/bot-commit multi-run policy remains Task 13. Task 11 only emits the approved bot commit path and no-op decision locally; it does not claim end-to-end live convergence through PR synchronize events.

## 2026-06-10 - grimoire task 11 continuation repair blockers

- Live boulder continuation was not run locally: safe real execution still needs `AI_RELAY_API_KEY`, a PAT source (`GRIMOIRE_PAT` or `CODEX_LOOP_PAT`) for later mutation paths, `opencode` availability, `GRIMOIRE_FIX_READY=1`, `GRIMOIRE_BOULDER_READY=1`, `GRIMOIRE_VERIFY_READY=1`, and grounded PR metadata.
- The repaired driver requires `.omo/boulder.json` completion proof before Task 9. Missing session id, missing/malformed boulder JSON, ambiguous active work, non-completed status, missing elapsed/session metadata, or failed `opencode run --continue` are all nonzero fail-closed states before commit/push/no-op termination.
- Prior session `ses_15240e663ffetMZkTyz6NSx4hU` was not reused for this repair path; local evidence uses deterministic fresh mock sessions such as `ses_mock_grimoire_task11_1`.

## 2026-06-10 - grimoire task 12 live blockers

- Live pull request execution was not run for Task 12. The requested proof is deterministic local evidence only; actual protected-path PR comments, labels, pushes, and workflow observations remain later E2E scope.
- Real base-controller fetch and PR diff computation still require a configured PAT source (`GRIMOIRE_PAT` or `CODEX_LOOP_PAT`) in CI. Missing PAT remains fail-closed before checkout, comment, commit, or push.
- Protected-path PRs intentionally stop before relay-key model execution in the workflow. That proves push 0 locally, but it also means no live read-only model review is claimed for protected PRs in this task.

## 2026-06-10 - grimoire task 12 Atlas repair notes

- RESOLVED: Atlas rejected the initial Task 12 evidence set because baseload and normal proof files were absent, and the protected proof could degrade to `status=blocked`/`action=halt` when the local controller fixture lacked complete executable/config material. who=Task 12 local fixture generation; what=incomplete base-controller material could mask `--protected-action read-only`; when=Atlas verification repair; why=controller validity is evaluated before protected-path action selection; how=fresh fixtures now copy executable base scripts plus `opencode.json` and `.opencode/oh-my-openagent.jsonc`, and the helper records `helper_executable` in controller checks.
- RESOLVED: `.github/scripts/grimoire-verify.sh` had SC2034 warnings for three absolute-path variables that were never consumed. who=verify script preamble; what=unused `fix_status_file`, `spec_sufficiency_file`, and `spec_gap_status_file`; when=Task 12 repair shellcheck gate; why=paths were normalized later inside the Python contract; how=removed only those assignments without broad shellcheck disables.
- STILL OPEN: no live PR scenario was run. Local evidence proves guard semantics, base-load selection, and normal pass-through only; live protected-path comments/labels/push observations remain Task 15-17 E2E scope and require configured relay/PAT prerequisites.

## 2026-06-10 - grimoire task 13 live blockers

- Live synchronize-loop evidence was not run against a throwaway PR because local/CI prerequisites remain unavailable or unasserted: `AI_RELAY_API_KEY`, `GRIMOIRE_PAT` or `CODEX_LOOP_PAT`, `GRIMOIRE_FIX_READY=1`, `GRIMOIRE_BOULDER_READY=1`, `GRIMOIRE_VERIFY_READY=1`, and grounded PR metadata. Task 13 therefore records deterministic local evidence only, not live GitHub success.
- No live bot commit, push, PR comment, label mutation, workflow dispatch, merge, or force-push was attempted. The local Task 13 fixtures use `--dry-run` and prove machine decisions/metadata without GitHub mutation.

## 2026-06-10 - grimoire task 13 executable-bit repair

- RESOLVED: Atlas hands-on CLI QA failed because `.github/scripts/grimoire-cast.sh` mode was `0o644`, causing direct execution to fail with `permission denied`. who=Task 13 file mode state; what=cast driver lacked owner executable bit despite workflow and CLI acceptance requiring direct execution; when=Atlas Task 13 verification; why=file content checks used `bash script` but direct executable QA requires mode; how=restored `0o755` on `.github/scripts/grimoire-cast.sh` and consistently restored `0o755` on shebang CLI peers `.github/scripts/grimoire-fix.sh` and `.github/scripts/grimoire-verify.sh`, matching the existing executable grimoire script pattern. Direct QA now observes `--help` exit 0, bad option exit 2, and bot synchronize `mock-noop --dry-run` exit 0 with `noop-approved` and `should_push=false`.

## 2026-06-10 - grimoire task 14 live evidence blockers

- Live PR-event retirement evidence was not run because Task 14 explicitly forbids live PR events, workflow dispatch, labels, comments, push, merge, and live `gh` workflow operations. The evidence therefore relies on deterministic local workflow parsing and static assertions.
- Repository/CI live prerequisites remain the same blockers recorded in earlier tasks: `AI_RELAY_API_KEY`, `GRIMOIRE_PAT` or `CODEX_LOOP_PAT`, grimoire readiness flags, and grounded PR metadata are not asserted locally. This task does not claim live grimoire review/autofix success.
- The Codex workflow files are still present by design as reversible disabled stubs. A future rollback could restore their previous bodies from git history, but that would deliberately reintroduce the retired Codex paths and should be guarded by actionlint and trigger assertions.

## 2026-06-10 - grimoire task 19 live label blockers

- Live `gh label create`, `gh pr edit --add-label`, and `gh pr edit --remove-label` were not run for Task 19. The current environment lacks `GRIMOIRE_PAT`, `CODEX_LOOP_PAT`, grounded PR metadata for a target PR, and the broader grimoire live readiness flags, so the task records deterministic local dry-run evidence only.
- Missing PAT or missing PR/repository metadata in real label mode is a fail-closed state. The helper may be used in dry-run mode for local evidence without secrets, but live mutation must wait for `GRIMOIRE_PAT` or `CODEX_LOOP_PAT` and grounded `GRIMOIRE_PR_NUMBER`/`GITHUB_REPOSITORY`.
- No `GITHUB_TOKEN`, GitHub App token helper, live PR comment, workflow dispatch, push, merge, or label mutation was attempted locally. The protected-path and spec-gap label transitions are represented by workflow/driver wiring plus local dry-run assertions until live E2E Tasks 15-17 can run.

## 2026-06-10 - grimoire task 19 repair finding

- RESOLVED: The initial Task 19 cast driver marked `✨ Cast` on the fixed-push branch. who=Task 19 driver integration; what=terminal label was applied after a nonterminal bot commit/push decision; when=Atlas repair verification; why=Task 13 requires fixed pushes to trigger `pull_request.synchronize` and a fresh re-review before terminal no-op; how=removed `mark_label_done` from the `fixed` branch and regenerated evidence that fails if a dry-run fixed path adds `✨ Cast` before terminal no-op.
- RESOLVED: Atlas could miss Task 19 docs when reading the file ending around Task 14 because the label lifecycle section was not present after the Task 14 retirement area. who=Task 19 documentation placement; what=the required label lifecycle details were not visible in the end-of-file context Atlas inspected; when=Task 19 repair; why=the section was inserted earlier in the cast-driver portion; how=added an explicit end-of-file `Task 19 PR label lifecycle` section documenting labels, colors, transitions, PAT-only auth, idempotency/no-churn, rollback/disable behavior, and live blockers.

## 2026-06-10 - grimoire task 18 live blockers

- STILL OPEN: Task 18 verified documentation completeness locally, not live PR execution. The local environment still lacks `AI_RELAY_API_KEY`, `GRIMOIRE_PAT`, `CODEX_LOOP_PAT`, `GRIMOIRE_DESIGN_READY=1`, `GRIMOIRE_FIX_READY=1`, `GRIMOIRE_BOULDER_READY=1`, `GRIMOIRE_VERIFY_READY=1`, and grounded PR metadata, so live review/design/fix/synchronize success remains Tasks 15-17 scope.
- STILL OPEN: No live `gh` mutation, workflow dispatch, push, label change, PR comment, or merge was run for Task 18. The off-switch examples in `docs/GRIMOIRE.md` are safe command shapes only and use placeholders instead of secret values or real repository targets.

## 2026-06-10 - grimoire task 15 live E2E blockers

- STILL OPEN: Task 15 draft-skip live E2E was not run. The required throwaway draft PR, pushed synchronize observation, live `gh run list` evidence, and proof of comment/push count 0 are unavailable in this environment.
- STILL OPEN: Task 15 happy-path live E2E was not run. The required ready PR with a small defect and sufficient spec, live run URL/log, `verdict.json`, check status, fixed-push evidence, and follow-up `pull_request.synchronize` re-review evidence are unavailable.
- STILL OPEN: Name-only local probe at 2026-06-09T22:11:43Z found `AI_RELAY_API_KEY`, `GRIMOIRE_PAT`, `CODEX_LOOP_PAT`, `GRIMOIRE_DESIGN_READY`, `GRIMOIRE_FIX_READY`, `GRIMOIRE_BOULDER_READY`, `GRIMOIRE_VERIFY_READY`, and grounded PR metadata env/fields unset or absent. No secret values were printed.
- STILL OPEN: No live `gh`, workflow dispatch, PR creation/edit, comment, label mutation, push, merge, or GitHub API mutation was run for Task 15; only local non-mutating script checks were run and recorded in `.omo/evidence/orl-task-15-draftskip.txt` and `.omo/evidence/orl-task-15-happy.txt`.

## 2026-06-10 - grimoire task 17 live E2E blockers

- STILL OPEN: Task 17 secret-leak live E2E was not run. The required placeholder sentinel injection, live workflow run URL/logs, PR comments, label/status artifacts, artifact bundle, and complete log/comment/artifact scan are unavailable in this environment.
- STILL OPEN: Task 17 parser-negative live E2E was not run. A local deterministic parser guard rejected missing verdict JSON and free-form approval prose, but live loop evidence with grounded PR artifacts is still unavailable.
- STILL OPEN: Task 17 hook-isolation live E2E was not run. Live noninteractive Prometheus `.md` design proof, separate Atlas/Sisyphus code-edit proof, and boulder/session artifacts proving the Prometheus markdown-only hook did not block fix execution are unavailable.
- STILL OPEN: Task 17 normal-control live E2E was not run. The required ready PR, live valid F1-F4 JSON artifact, driver decision artifact, run/check status, and no-secret scan are unavailable.
- STILL OPEN: Name-only local probes at 2026-06-09T22:35:20Z found `AI_RELAY_API_KEY`, `GRIMOIRE_PAT`, `CODEX_LOOP_PAT`, `GRIMOIRE_DESIGN_READY`, `GRIMOIRE_FIX_READY`, `GRIMOIRE_BOULDER_READY`, `GRIMOIRE_VERIFY_READY`, and grounded PR metadata env/fields unset or absent. No secret values were printed.
- STILL OPEN: No live `gh`, workflow dispatch, PR creation/edit, comment, label mutation, push, merge, or GitHub API mutation was run for Task 17. Evidence files are blocker artifacts only and must not be treated as live acceptance evidence.

## 2026-06-10 - grimoire task 16 live E2E blockers

- STILL OPEN: Task 16 spec-insufficient live E2E was not run. The required live pilot PR diff, workflow run URL/log, five-section PR spec-gap comment, check/Fizzled outcome, and proof of code diff 0 are unavailable in this environment.
- STILL OPEN: Task 16 no-op live E2E was not run. The required clean PR run, live commit-history comparison, and proof of zero autofix commits/pushes are unavailable in this environment.
- STILL OPEN: Task 16 protected-config live E2E was not run. The required protected-path PR, live trusted-controller halt/read-only comment, and proof of zero autonomous model/write/commit/push are unavailable in this environment.
- STILL OPEN: Task 16 positive-control live E2E was not run. A sufficient-spec control PR still needs the same live secrets, readiness flags, grounded PR metadata, run/check artifacts, and synchronize evidence if a fixed push occurs.
- STILL OPEN: Name-only local probe at 2026-06-09T22:35:32Z found `AI_RELAY_API_KEY`, `GRIMOIRE_PAT`, `CODEX_LOOP_PAT`, `GRIMOIRE_DESIGN_READY`, `GRIMOIRE_FIX_READY`, `GRIMOIRE_BOULDER_READY`, `GRIMOIRE_VERIFY_READY`, and grounded PR metadata env/fields unset or absent. No secret values were printed.
- STILL OPEN: No live `gh`, workflow dispatch, PR creation/edit, comment, label mutation, push, merge, or GitHub API mutation was run for Task 16. Evidence files are blocker artifacts only; local mock checks are explicitly labeled non-acceptance evidence.

## 2026-06-10 - F4 scope fidelity blocker: ignored `.opencode` generated dependency artifacts

- STILL OPEN: Final F4 scope scan found ignored generated dependency material under `.opencode`: `.opencode/node_modules` (3,443 files, 54,786,874 bytes), `.opencode/package.json`, `.opencode/package-lock.json`, and `.opencode/.gitignore`. The intended grimoire config scope only needs `.opencode/oh-my-openagent.jsonc` plus root `opencode.json`; leaving generated package-manager artifacts in the protected config tree is worktree contamination even though git ignores it.
- The refined active-file secret scan did not find token values in changed grimoire files or representative task evidence, and tracked/untracked nonignored scope remained limited to grimoire control-plane files. The blocker is generated-junk contamination, not Rust/domain behavior or a live QA blocker.


## 2026-06-10 - RESOLVED cleanup: F4 `.opencode` generated artifacts removed

- RESOLVED: Removed generated `.opencode` artifacts from the protected config tree: `.opencode/node_modules/`, `.opencode/package.json`, `.opencode/package-lock.json`, and `.opencode/.gitignore`.
- Preserved `.opencode/oh-my-openagent.jsonc` and root `opencode.json` by hash verification; `.opencode` now lists only `oh-my-openagent.jsonc`.
- Evidence: `.omo/evidence/final-f4-opencode-cleanup.txt` records before/after listing, exact deleted paths, config hashes, and verification commands.
- F4 must be rerun after cleanup. This cleanup does not resolve F1/F3 live E2E blockers and does not claim Final Wave approval.

## 2026-06-10 - Task 20 added + blocked: relay key dual-source (secret OR runner env)

- ROOT CAUSE (who/what/when/why/how): who=`.github/workflows/grimoire-attune.yml` and `.github/workflows/grimoire.yml` step `env:` mapping; what=`AI_RELAY_API_KEY: ${{ secrets.AI_RELAY_API_KEY }}` at 4 sites (attune 58,191; grimoire 242,269); when=discovered while planning live setup after user confirmed runner is online and the relay key is injected via self-hosted runner (local docker) env, not a GitHub Actions secret; why=when the secret is unset the expression resolves to an empty string and shadows the runner-inherited `AI_RELAY_API_KEY` within the step, so the smoke/driver fails closed even though the key exists in the runner env; how=mirror the `CODEX_LOOP_PAT` pattern — map the secret to `AI_RELAY_API_KEY_SECRET`, resolve `selected=${AI_RELAY_API_KEY_SECRET:-${AI_RELAY_API_KEY:-}}`, `::add-mask::`, then `export AI_RELAY_API_KEY="$selected"` for opencode child inheritance.
- User-confirmed operating facts recorded in plan Task 20: self-hosted runner is online; relay key arrives via runner docker env; this repo has zero `push`-triggered workflows (grimoire is non-draft `pull_request` only), so a branch push does not wake any in-repo workflow.
- PAT permissions answered (consultation, no file edit): fine-grained PAT on this repo = Contents R/W, Pull requests R/W, Issues R/W, Metadata R; Workflows scope NOT needed because trusted-controller halts on `.github/**`; classic PAT `repo` alone suffices without `workflow`.
- BLOCKED `[~]`: Task 20 needs `.yml` implementation, which Prometheus cannot perform (md-only). Implementation belongs to Sisyphus via `/start-work`. Live verification additionally needs `GRIMOIRE_PAT` (or runner `CODEX_LOOP_PAT`) and an actual `grimoire-attune.yml` dispatch / pilot PR E2E. These are user/executor actions.
- Dependency note: Task 20 blocks live Task 1 (attune) and Tasks 15-17 (E2E). Until Task 20 lands, runner-env relay key delivery makes attune/E2E fail closed, so F1/F3 cannot gather live acceptance evidence.
- No secret values, runner env values, token-bearing URLs, or live run URLs were recorded. Names and resolution logic only.

## 2026-06-10 - Prerequisite progress: GRIMOIRE_PAT registered

- DONE: `gh secret list` now shows `GRIMOIRE_PAT` registered as a repo secret (name-only verification; value not printed). PAT-only auth source for grimoire git/gh/checkout/push/comment/label is now satisfiable in CI.
- STILL runner-side: `AI_RELAY_API_KEY` is intentionally NOT a repo secret; it is injected via the self-hosted runner (local docker) env. This only works after Task 20 dual-source fix lands; until then attune/grimoire fail closed because the empty `secrets.AI_RELAY_API_KEY` mapping shadows the runner env.
- Existing `CODEX_APP_ID`/`CODEX_APP_PRIVATE_KEY` remain unused by grimoire (GitHub App = PAT-only violation per plan). No grimoire path consumes them.
- PAT scope correctness cannot be validated locally because the token value lives only in the CI secret; scope sufficiency (Contents R/W, Pull requests R/W, Issues R/W, Metadata R) is first exercised on the live attune dispatch / pilot PR run.
- Updated prerequisite matrix: [A AI_RELAY_API_KEY] runner-env, pending Task 20; [B GRIMOIRE_PAT] DONE; [C runner online] DONE (user-confirmed). Next gate = Task 20 implementation via `/start-work` (Sisyphus), then live attune (Task 1) + pilot PR E2E (Tasks 15-17), then F1/F3 re-audit.
- No secret values or token-bearing URLs recorded; names and status only.

## 2026-06-11 - grimoire task 1 live dispatch and local nested-run caveats

- STILL OPEN: live `workflow_dispatch` for `grimoire-attune.yml` was not run from this local implementation step because the workflow is not yet on the remote/default branch and this task did not authorize a push solely for dispatch. No live run URL, job log, or artifact is claimed.
- STILL OPEN: local nested `opencode run` smoke returned `Session not found` before producing model/tool evidence under both `opencode` `1.17.0` and a temp-installed `1.16.2`, even with isolated XDG config/data and the built-in build agent. `opencode agent list` still loads the OMO plugin successfully.
- The local nested-run failure is recorded as a blocker for local live smoke only, not as a workflow success or failure. The committed workflow contains the actual ai-relay/read/bash smoke and must be validated by a real Actions dispatch after remote availability and runner relay env prerequisites are in place.
- No live `gh`, workflow dispatch, PR mutation, label mutation, push, merge, or secret-bearing output was attempted for this task.

## 2026-06-11 - grimoire task 1 PAT checkout repair

- RESOLVED: tokenless checkout was rejected because private repo checkout must be PAT-only. who=`.github/workflows/grimoire-attune.yml` actions/checkout config; what=the Task 1 healthcheck used an empty checkout token while targeting a private repository; when=post-implementation verification after `ad482e2`; why=tokenless checkout is likely to fail before the healthcheck can run and does not prove the private-repo attune path works under the required PAT-only auth model; how=added a pre-checkout `GRIMOIRE_PAT_SECRET` presence check, masks the value before checkout, fails closed when missing, and checks out with `secrets.GRIMOIRE_PAT` while preserving `persist-credentials: false`.
- STILL OPEN: live `workflow_dispatch` was not run from this local repair because the task forbids push and no live dispatch was authorized. No live run URL, job log, model smoke success, GitHub mutation, or secret-bearing output is claimed.

## 2026-06-11 - grimoire task 1 live dispatch BLOCKED on default-branch requirement (human merge)

- User approved option 1 (push branch + dispatch attune). Orchestrator pushed `feat/grimoire-opencode-loop` to origin (SSH; no push-triggered workflow exists, so nothing auto-triggered).
- BLOCKER (who/what/when/why/how): who=GitHub Actions `workflow_dispatch` platform rule; what=`gh workflow run grimoire-attune.yml --ref feat/grimoire-opencode-loop` returns `HTTP 404: workflow grimoire-attune.yml not found on the default branch`; when=2026-06-11 after branch push; why=GitHub only allows `workflow_dispatch` for workflows whose file exists on the repository default branch (`main`); how=resolve by merging the grimoire workflow to `main` (human-only; agents never merge), after which `--ref` dispatch against any branch becomes possible.
- Consequence: Task 1's live healthcheck acceptance (Scenario "헬스체크 성공": `gh workflow run` job conclusion=success, 6 PASS checks) is BLOCKED until a human merges the attune workflow onto `main`. Local/static implementation + verification for Task 1 is complete and committed (`ad482e2`, `e6e9b3a`). Task 1 marked `[~]` in plan.
- Same default-branch constraint blocks live E2E tasks 15/16/17 and the live-CI-attune step until the grimoire branch is merged to `main` by a human.
- No secret values, tokens, or runner env values were printed. `gh auth status` token was redacted. SSH was used for the branch push; no `GITHUB_TOKEN`/App token was used.

## 2026-06-11 - grimoire task 1 live dispatch BLOCKED on default-branch requirement

- CONTEXT: user explicitly approved option 1 (push `feat/grimoire-opencode-loop` + dispatch attune). Branch was pushed to origin via SSH (no PAT/GITHUB_TOKEN in local git); push created a brand new remote branch and triggered zero in-repo workflows (this repo has no `push`-triggered workflow and `grimoire.yml` does not exist yet).
- ROOT CAUSE (who/what/when/why/how): who=`gh workflow run grimoire-attune.yml --ref feat/grimoire-opencode-loop`; what=returned `HTTP 404: workflow grimoire-attune.yml not found on the default branch`; when=after pushing the feature branch; why=GitHub only exposes `workflow_dispatch` for workflow files that exist on the repository default branch (`main`); how=the attune workflow currently lives only on `feat/grimoire-opencode-loop`, not on `main`.
- BLOCKER (human action required): live attune dispatch + healthcheck success evidence (Task 1 acceptance Scenario "헬스체크 성공", and Task 20 live runner-env relay smoke) cannot be produced until `grimoire-attune.yml` is present on the default branch `main`. Putting it on `main` requires a human-merged PR (agents never merge / never push main). After the workflow is on `main`, `gh workflow run grimoire-attune.yml --ref feat/grimoire-opencode-loop` becomes dispatchable against the feature branch.
- DECISION: Task 1 stays `[~]` (implementation + local/static evidence complete; live dispatch blocked on human merge to default branch). Proceeding with downstream tasks that do not require live dispatch (repo `opencode.json`, OMO mapping, CI overrides, stage scripts, workflow, driver). Live E2E (Tasks 15-17) and live attune remain gated on the same default-branch availability + runner relay env.
- No secret values, tokens, or run URLs recorded. gh local auth uses the developer account; the PAT-only rule governs CI workflow auth, not local developer git/gh.

## 2026-06-10T17:18:38Z - grimoire task 4 live Prometheus smoke blocker

- STILL OPEN: local live Prometheus no-ask smoke did not produce a transcript. ROOT CAUSE (who/what/when/why/how): who=`opencode run --agent prometheus --format json` launched against an isolated temporary project copy; what=the command exceeded the 180000 ms tool timeout before returning a transcript; when=2026-06-10T17:18:38Z during Task 4 verification; why=the local nested opencode/model path did not complete in this environment even though `AI_RELAY_API_KEY` was present by name, so no evidence can prove no Question/interview wait locally; how=record static/config-load evidence now and defer live behavioral proof to live CI after default-branch workflow availability and runner prerequisites are in place.
- No secret values, token-bearing URLs, workflow dispatch, PR mutation, label mutation, push, merge, or generated `.opencode` artifacts were retained. The timed-out live smoke is a blocker for local behavioral proof only, not a claim that CI autonomous overrides failed.
