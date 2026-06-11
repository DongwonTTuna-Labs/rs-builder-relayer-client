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

## 2026-06-11 - grimoire task 5 live review readiness caveat

- STILL OPEN: who=`.github/scripts/grimoire-review.sh` real-mode readiness gate and local Task 5 environment; what=real mode wrote a blocked review artifact and exited nonzero before `opencode run`; when=Task 5 verification; why=`GRIMOIRE_TEAM_MODE_ENABLED=1` was not asserted, and live Team Mode readiness must be proven before any model review is trusted; how=set the readiness flag only after CI proves team-mode tools are available, then rerun real mode with sanitized artifact evidence. This task records deterministic local evidence only and does not claim live model or Team Mode success.

## 2026-06-11 - grimoire task 6 live design readiness caveat

- STILL OPEN: who=`.github/scripts/grimoire-design.sh` real-mode readiness gate and local Task 6 environment; what=real mode wrote `spec_sufficient=false` blocked JSON and exited nonzero before `opencode run`; when=Task 6 verification; why=`AI_RELAY_API_KEY`, `GRIMOIRE_DESIGN_READY=1`, and `GRIMOIRE_TEAM_MODE_ENABLED=1` were deliberately unset in the fail-closed proof, and live Prometheus design readiness must be proven before model planning is trusted; how=set readiness only after CI proves non-interactive Prometheus plus OpenSpec binding can run safely, then rerun with sanitized artifact evidence.
- STILL OPEN: no live Prometheus/OpenSpec model design success is claimed for Task 6. Deterministic local evidence proves contract behavior only; live design remains gated by default-branch workflow availability, relay auth, opencode availability, and explicit readiness flags.
- No secret values, token-bearing URLs, workflow dispatch, PR mutation, label mutation, push, merge, or GitHub API mutation was run or recorded for Task 6.

## 2026-06-11 - grimoire task 11 evidence repair

- RESOLVED: Previous Task 11 evidence was missing and a prior attempt wrote mock boulder state into real `.omo/boulder.json`. who=Task 11 local mock evidence generation; what=mock boulder state could replace the active repo boulder file; when=Task 11 repair verification; why=mock continuation used the real boulder path instead of an isolated mock path; how=current `.github/scripts/grimoire-cast.sh` keeps `REAL_BOULDER_JSON=.omo/boulder.json` and `MOCK_BOULDER_JSON=.omo/ci/grimoire-cast-mock-boulder.json`, all mock modes were rerun in a temp fixture, and repo `.omo/boulder.json` remained `active_work_id=grimoire-1e4ba548` before and after verification.
- STILL OPEN: live-small Task 11 remains blocked because no live PR run URL, Actions log, or verdict artifact exists, and this repair task forbids live GitHub mutations. Local evidence is deterministic fixture evidence only.

## 2026-06-10T19:46:00Z - grimoire task 9 live verification readiness caveat

- STILL OPEN: who=`.github/scripts/grimoire-verify.sh` real-mode readiness gate and current Task 9 environment; what=real mode writes a REJECT/blocked verdict before model calls when Task 7/8 prerequisite artifacts, relay auth, opencode, or readiness are absent; when=Task 9 implementation and local verification; why=the active tree currently has Task 5/6 stage scripts but no Task 7 `.omo/ci/spec-gap-status.json` or Task 8 `.omo/ci/fix-status.json` producer in scope, and live F1-F4 approval must not run before those machine contracts exist; how=keep local mock/validation evidence as contract proof now, set `GRIMOIRE_VERIFY_READY=1` only after Task 7/8 artifacts plus relay/opencode readiness are proven, then rerun real mode with sanitized evidence.
- No live F1-F4 model verification, GitHub workflow dispatch, PR comment, label mutation, push, merge, or GitHub API mutation was attempted for Task 9.

## 2026-06-10T20:34:39Z - grimoire task 8 live fix readiness caveat

- STILL OPEN: who=`.github/scripts/grimoire-fix.sh` real-mode readiness gate and current local Task 8 environment; what=the real-mode fixture wrote `status=blocked` before any model/code authorization when `AI_RELAY_API_KEY` and `GRIMOIRE_FIX_READY=1` were unset; when=Task 8 implementation verification; why=live Atlas/Sisyphus|Hephaestus mutation must not run until relay auth, opencode availability, readiness, PR-touched metadata, and declared post-fix changed-file metadata are all grounded; how=set `GRIMOIRE_FIX_READY=1` only after CI proves the fix prompt/scope guard safe, provide sanitized PR-touched and post-fix changed-file metadata from the driver, then rerun real mode with evidence.
- STILL OPEN: who=Task 8 local verification scope; what=no live Atlas/Sisyphus/Hephaestus fix, no PR comment, no label change, no workflow dispatch, no commit, no push, no merge, and no GitHub API mutation were attempted; when=Task 8 implementation verification; why=this task is a CI control-plane stage implementation with deterministic local evidence only; how=Task 11+ live driver/E2E work must run the handoff under PAT/relay readiness and then feed the post-fix changed-file list back through this scope/no-op contract.

## 2026-06-11 - grimoire task 10 live workflow caveats

- STILL OPEN: live `.github/workflows/grimoire.yml` PR execution was not run for Task 10. This task produced local deterministic evidence only and did not create/edit PRs, dispatch workflows, post comments, change labels, push, merge, or call GitHub API mutation.
- STILL OPEN: eligible ready PR runs are expected to fail closed until Task 11 adds executable `.github/scripts/grimoire-cast.sh`; the missing-driver failure is intentional and must not be interpreted as a successful review/autofix loop.
- STILL OPEN: the workflow file is currently on the feature branch, not merged to the default branch. Any live PR-triggered proof still depends on the normal GitHub Actions availability path for workflow files and the external PAT/runner relay prerequisites.
- STILL OPEN: checkout/comment/push/label paths require `GRIMOIRE_PAT` or runner `CODEX_LOOP_PAT` in CI. The workflow records only source selection and masks values; no secret value or token-bearing URL was printed or recorded locally.

## 2026-06-11 - grimoire task 11 executable-bit repair

- RESOLVED: Atlas temp-fixture verification failed with `permission denied` because `.github/scripts/grimoire-cast.sh` was mode `644`. who=Task 11 file metadata; what=cast driver lacked executable bits for direct `./.github/scripts/grimoire-cast.sh --help`; when=Task 11 mode repair verification; why=previous evidence used shell/bash execution paths but Atlas copies with `cp -p` and runs the script directly; how=restored mode `755`, verified repo-root direct help exit 0, `cp -p` temp-fixture direct help exit 0, `shellcheck` exit 0, and `.omo/boulder.json` remained `active_work_id=grimoire-1e4ba548` with `status=active`.

## 2026-06-11 - grimoire task 11 mock boulder and staging repair

- RESOLVED: Previous Task 11 mock modes wrote real `.omo/boulder.json`, contaminating active Atlas state with `active_work_id=grimoire-task11-mock`, and the previous repair session made no effective changes. who=Task 11 cast driver and prior repair attempt; what=mock boulder writes targeted the production boulder path and fixed mutation used broad worktree staging; when=verified during Task 11 defect repair after two failed sessions; why=`BOULDER_JSON` was hard-coded to `.omo/boulder.json` for both real and mock modes, and `git add --all` ignored Task 8 `changed_files`; how=`.github/scripts/grimoire-cast.sh` now selects `REAL_BOULDER_JSON=.omo/boulder.json` for real mode and `MOCK_BOULDER_JSON=.omo/ci/grimoire-cast-mock-boulder.json` for mock modes unless `GRIMOIRE_BOULDER_JSON` is explicitly set, restores `.omo/boulder.json` to active `grimoire-1e4ba548`, rejects undeclared working-tree changes with `mutation-scope-blocked`, and stages only declared scoped paths.

## 2026-06-11 - grimoire task 12 trusted-controller implementation caveats

- RESOLVED: the inherited trusted-controller draft failed local CLI smoke on macOS Bash 3.2 because empty arrays were expanded under `set -u`. who=`.github/scripts/grimoire-trusted-controller.sh` argument wrapper; what=`changed_list_paths[@]`/`changed_files[@]` expansion aborted with `unbound variable`; when=Task 12 protected fixture smoke; why=Bash 3.2 treats empty array expansion under nounset differently from newer Bash; how=guarded both loops with explicit length checks before expansion.
- RESOLVED: executable-bit preservation mattered for base-controller loading. who=file metadata after edits; what=`grimoire-trusted-controller.sh` and `grimoire-cast.sh` could be copied as `644`, making base material fail `*_executable` checks; when=Task 12 smoke fixture copied scripts with `cp -pR`; why=content edits reset mode in this environment; how=restored both scripts to mode `755` and kept evidence fixtures copying executable bits.
- STILL OPEN: no live protected-path PR or normal PR workflow run was executed for Task 12. The new evidence is deterministic local proof only; live run URL/log/comment/label/push observations remain downstream E2E scope after the workflow exists on the default branch and CI secrets/readiness are configured.
- STILL OPEN: the workflow uses read-only GitHub pull-request files API through the selected PAT to collect changed paths. This is not a GitHub mutation, but live availability still depends on `GRIMOIRE_PAT` or runner `CODEX_LOOP_PAT` being configured and scoped correctly in CI.

## 2026-06-11 - grimoire task 12 protected reason-comment repair

- RESOLVED: Task 12 protected-path acceptance required a reason comment, but the first trusted-controller implementation only halted before model/driver execution. who=Task 12 trusted-controller workflow branch; what=protected status produced no reason-comment path; when=protected-path acceptance verification; why=workflow stopped after `status=protected` without rendering/posting a trusted-base comment artifact; how=added trusted-base `.github/scripts/grimoire-protected-comment.sh`, required it during base-controller load, invoked it only when controller outputs `status=protected`, and updated deterministic evidence to prove the five-section comment artifact, missing-token post block, and normal empty/noop artifact.
- STILL OPEN: no live protected-path PR comment was posted during this repair. Local proof intentionally used dry-run/artifact and missing-token post-block assertions only; a live comment URL/run URL remains downstream E2E evidence scope.

## 2026-06-11 - grimoire task 13 live synchronize caveats

- STILL OPEN: live multi-cycle `pull_request.synchronize` evidence was not run. The current proof is deterministic local fixture evidence only; no live run URL, check status, bot commit SHA, PR history, or workflow log proves an actual GitHub re-review yet.
- STILL OPEN: live Task 13 remains gated by default-branch workflow availability plus CI relay/PAT/readiness prerequisites (`AI_RELAY_API_KEY`, `GRIMOIRE_PAT` or `CODEX_LOOP_PAT`, `GRIMOIRE_FIX_READY=1`, `GRIMOIRE_BOULDER_READY=1`, `GRIMOIRE_VERIFY_READY=1`, and grounded PR metadata).
- No live GitHub mutation was attempted for Task 13: no workflow dispatch, PR create/edit/comment/label, push, merge, secret change, or token-bearing output. The local proof uses `--dry-run` and temp fixtures under `/var/folders/vz/hx33c759727ftq88cxbgp8r40000gn/T/opencode` only.

## 2026-06-11 - grimoire task 14 static-retirement caveats

- STILL OPEN: no live PR-event retirement evidence was run for Task 14 because this task explicitly forbids workflow dispatch, live PR events, comments, labels, pushes, merges, and GitHub state mutation. Evidence is deterministic local/static proof only.
- The detailed Task 14 checklist still names the seven legacy Codex workflow files, but current tree truth follows the plan refresh note: those paths are already absent from `.github/workflows`, so there was nothing present to convert into a retired stub.
- `grimoire-attune.yml` remains `workflow_dispatch` for manual healthcheck, so the "sole active PR review path" assertion means only `grimoire.yml` has `pull_request`; it does not mean every non-PR maintenance workflow is disabled.
- No home-server-infra checkout or shared reusable workflow was inspected or changed. If a future rollback restores any Codex workflow from git history, actionlint plus static trigger assertions must be rerun before enabling it.

## 2026-06-11 - grimoire task 20 live caveats

- STILL OPEN: no live `grimoire-attune.yml` workflow_dispatch or live `grimoire.yml` PR run was executed for Task 20. Evidence is local shell simulation and static workflow checks only; no live run URL, Actions log, model smoke, PR comment, label mutation, push, merge, or GitHub API mutation is claimed.
- STILL OPEN: live runner-env relay proof still depends on the workflow being available on the default branch and the self-hosted runner injecting `AI_RELAY_API_KEY` at runtime. The local runner-env simulation proves the resolver contract only, not a live ai-relay response.
- RESOLVED LOCALLY: the previous empty-secret shadowing root cause is removed in workflow code by using `AI_RELAY_API_KEY_SECRET` for the GitHub secret source and falling back to the inherited `AI_RELAY_API_KEY` environment variable. Local evidence records only source labels and redacted markers; no secret value, sentinel literal, length, prefix, hash, or fingerprint is stored.

## 2026-06-11 - grimoire task 18 live and documentation caveats

- STILL OPEN: Task 18 verification is deterministic local documentation linting only. It does not prove live grimoire PR execution, live attune, live model success, live comments, live labels, live push, or live synchronize re-review.
- STILL OPEN: live proof still needs default-branch workflow availability, configured PAT path, runner relay env or GitHub relay secret, `GRIMOIRE_DESIGN_READY=1`, `GRIMOIRE_FIX_READY=1`, `GRIMOIRE_BOULDER_READY=1`, `GRIMOIRE_VERIFY_READY=1`, and grounded PR metadata.
- CURRENT DOC SCOPE: the guide records Task 19 labels as outside current completed behavior because the current `.github/scripts` inventory has no `grimoire-labels.sh`; future label implementation must update the guide and evidence if labels become active.
- No live `gh`, workflow dispatch, PR create/edit/comment, label mutation, push, merge, secret change, or GitHub API mutation was run for Task 18.

## 2026-06-11 - grimoire task 19 local/live caveats

- STILL OPEN: no live `gh label create`, `gh pr edit --add-label`, `gh pr edit --remove-label`, issue event timeline query, workflow dispatch, PR edit, push, merge, or secret change was run for Task 19. Evidence is deterministic local dry-run/local-state proof only.
- STILL OPEN: live label mutation still depends on default-branch workflow availability, a configured PAT source (`GRIMOIRE_PAT` or `CODEX_LOOP_PAT`), grounded repo/PR metadata, and later E2E Tasks 15-17. Missing any of those must fail closed before `gh`.
- RESOLVED LOCALLY: Task 18's doc caveat that no label helper exists is superseded by `.github/scripts/grimoire-labels.sh`, but only the local contract is proven in this task. Documentation now says live label mutation remains future E2E, not completed live behavior.
- No secret values, PAT values, token-bearing URLs, label event API output, or fabricated live GitHub state were recorded.

## 2026-06-11 - grimoire task 15 E2E preflight BLOCKED

- RESULT: BLOCKED for both draft-skip and happy-path live E2E evidence. who=Task 15 local verification preflight; what=live PR/Actions scenarios were not executed; when=2026-06-11T14:37:39Z; why=the default branch `main` does not expose `.github/workflows/grimoire.yml`, required readiness flags are unset, and grounded PR metadata is absent; how=read-only/name-only `gh`, env, and `GIT_MASTER=1 git` probes were run before any mutation and returned `safe_to_mutate=false`.
- Absent preconditions by name only: `.github/workflows/grimoire.yml` on default branch, `.github/workflows/grimoire-attune.yml` on default branch, `GRIMOIRE_DESIGN_READY`, `GRIMOIRE_FIX_READY`, `GRIMOIRE_BOULDER_READY`, `GRIMOIRE_VERIFY_READY`, `GRIMOIRE_PR_NUMBER`, `GRIMOIRE_HEAD_REPO`, `GRIMOIRE_HEAD_REF`, `GRIMOIRE_HEAD_SHA`, `GRIMOIRE_BASE_REPO`, `GRIMOIRE_BASE_REF`, `GITHUB_EVENT_PATH`, `GITHUB_REPOSITORY`, and `GITHUB_RUN_ID`.
- Source presence by name only: `gh` authenticated; repo secret name `GRIMOIRE_PAT` present; local env `AI_RELAY_API_KEY` present by name; repo secret name `AI_RELAY_API_KEY` missing; local env `GRIMOIRE_PAT` and `CODEX_LOOP_PAT` missing. No values, lengths, prefixes, hashes, fingerprints, or token-bearing URLs were recorded.
- No live `gh` mutation, workflow dispatch, PR creation/edit, branch push, label mutation, comment, secret mutation, merge, or home-server-infra access was attempted. Evidence files written: `.omo/evidence/orl-task-15-draftskip.txt` and `.omo/evidence/orl-task-15-happy.txt`.

## 2026-06-11 - grimoire task 16 E2E preflight BLOCKED

- RESULT: BLOCKED for spec-insufficient, no-op, protected-config, and sufficient-spec control live E2E evidence. who=Task 16 local verification preflight; what=live PR/Actions scenarios were not executed; when=2026-06-11T14:49:43Z; why=the default branch `main` does not expose `.github/workflows/grimoire.yml` or `.github/workflows/grimoire-attune.yml`, required readiness flags are unset, grounded PR metadata is absent, and local PAT env sources are absent; how=read-only/name-only `gh`, env, and `GIT_MASTER=1 git` probes were run before any mutation and returned `safe_to_mutate=false`.
- Absent preconditions by name only: `.github/workflows/grimoire.yml` on default branch, `.github/workflows/grimoire-attune.yml` on default branch, `GRIMOIRE_DESIGN_READY`, `GRIMOIRE_FIX_READY`, `GRIMOIRE_BOULDER_READY`, `GRIMOIRE_VERIFY_READY`, `GRIMOIRE_PR_NUMBER`, `GRIMOIRE_HEAD_REPO`, `GRIMOIRE_HEAD_REF`, `GRIMOIRE_HEAD_SHA`, `GRIMOIRE_BASE_REPO`, `GRIMOIRE_BASE_REF`, `GITHUB_EVENT_PATH`, `GITHUB_REPOSITORY`, and `GITHUB_RUN_ID`.
- Source presence by name only: `gh` authenticated; repo secret name `GRIMOIRE_PAT` present; local env `AI_RELAY_API_KEY` present by name; repo secret name `AI_RELAY_API_KEY` missing; local env `GRIMOIRE_PAT` and `CODEX_LOOP_PAT` missing; local env `GITHUB_TOKEN` and `GH_TOKEN` absent. No values, lengths, prefixes, hashes, fingerprints, token-derived data, or token-bearing URLs were recorded.
- Policy decision: throwaway branch push/PR creation/workflow dispatch/comment/label mutation is not safe because live E2E cannot be bound to a default-branch grimoire workflow or grounded PR metadata. No live `gh` mutation, workflow dispatch, PR creation/edit, branch push, label mutation, comment, secret mutation, merge, or home-server-infra access was attempted. Evidence files written: `.omo/evidence/orl-task-16-specgap.txt`, `.omo/evidence/orl-task-16-noop.txt`, `.omo/evidence/orl-task-16-protected.txt`, and `.omo/evidence/orl-task-16-control.txt`.

## 2026-06-11 - grimoire task 17 E2E preflight BLOCKED

- RESULT: BLOCKED for secret-leak, parser-negative live loop, hook-isolation live sessions, and normal-control live E2E evidence. who=Task 17 local verification preflight; what=live PR/Actions scenarios were not executed; when=2026-06-11T15:48:56Z; why=`.github/workflows/grimoire-attune.yml` is absent on default branch `main`, grimoire readiness flags are not ready, grounded PR metadata is absent, and local PAT env sources are absent; how=read-only/name-only `gh`, env, and `GIT_MASTER=1 git` probes were run before any mutation and returned live mutation blocked.
- Current default-branch workflow truth by name only: `.github/workflows/grimoire.yml` on default branch status: present; `.github/workflows/grimoire-attune.yml` on default branch status: absent.
- Readiness flags by name only: `GRIMOIRE_DESIGN_READY` status: not ready; `GRIMOIRE_FIX_READY` status: not ready; `GRIMOIRE_BOULDER_READY` status: not ready; `GRIMOIRE_VERIFY_READY` status: not ready.
- Grounded metadata by name only: `GRIMOIRE_PR_NUMBER`, `GRIMOIRE_HEAD_REPO`, `GRIMOIRE_HEAD_REF`, `GRIMOIRE_HEAD_SHA`, `GRIMOIRE_BASE_REPO`, `GRIMOIRE_BASE_REF`, `GITHUB_EVENT_PATH`, `GITHUB_REPOSITORY`, and `GITHUB_RUN_ID` status: absent.
- Source presence by name only: `gh` authenticated; repo secret name `GRIMOIRE_PAT` status: present; repo secret name `AI_RELAY_API_KEY` status: absent; local env `AI_RELAY_API_KEY` status: present by name only; local env `GRIMOIRE_PAT`, `CODEX_LOOP_PAT`, `GITHUB_TOKEN`, and `GH_TOKEN` status: absent. No values, lengths, prefixes, hashes, fingerprints, token-derived data, or token-bearing URLs were recorded.
- Local-safe probes were run only in temp fixtures: parser negatives failed closed, mock all-APPROVE JSON and mock no-op control were accepted, a synthetic sentinel scanner found zero target matches after its positive-control file was deleted, and hook isolation was only partially proven by static/mock boundaries. Evidence files written: `.omo/evidence/orl-task-17-secret.txt`, `.omo/evidence/orl-task-17-parser.txt`, `.omo/evidence/orl-task-17-hook.txt`, and `.omo/evidence/orl-task-17-control.txt`.
- No live `gh` mutation, workflow dispatch, PR creation/edit, branch push, label mutation, comment, secret mutation, merge, or home-server-infra access was attempted. Task 17 evidence must not be treated as live acceptance evidence until the missing live preconditions are satisfied.
