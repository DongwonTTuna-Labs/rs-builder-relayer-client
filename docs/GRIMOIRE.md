# Grimoire Architecture And Operations Guide

Current state: this guide reflects the local workflow, script contracts, and static evidence in this branch. It doesn't claim live end-to-end PR convergence, live attune success, live label mutation, live PR comments, live model success, or a live run URL. Task 19 label behavior has deterministic local dry-run evidence; live label mutation still needs later E2E evidence. Live Tasks 15, 16, and 17 remain gated by default-branch workflow availability, PAT and relay readiness, explicit grimoire readiness flags, and a throwaway PR run.

`docs/GRIMOIRE.md` was first created for Task 3 secret and model mapping. That content is now part of the full guide below, including Task 20 dual-source `AI_RELAY_API_KEY` handling and PAT-only GitHub auth.

## Flow

Grimoire is the active pull-request review loop for this pilot repo. The loop is controlled by `.github/workflows/grimoire.yml` and the trusted base copy of `.github/scripts/grimoire-cast.sh`.

The intended end-to-end path is:

1. A non-draft PR fires `pull_request` on `opened`, `ready_for_review`, `synchronize`, or `reopened`.
2. The workflow resolves PAT-only GitHub auth, fetches trusted base-controller material, marks `🔮 Casting…` from that trusted helper path, collects changed files, and checks protected paths before model credentials are used.
3. The review stage runs read-only review and writes `.omo/ci/review-findings.json`.
4. The design stage binds findings to OpenSpec evidence and writes `.omo/ci/spec-sufficiency.json` plus `.omo/ci/design-plan.md`.
5. If specs are insufficient, the spec-gap stage writes `.omo/ci/spec-gap-comment.md` and `.omo/ci/spec-gap-status.json`, marks `💨 Fizzled`, then the driver halts without code edits, commits, pushes, or model guessing.
6. If specs are sufficient, the fix stage writes `.omo/ci/fix-status.json` and `.omo/ci/fix-handoff-prompt.md`, using the spec-bound plan and scope guard.
7. Atlas starts or continues the grimoire boulder work until `.omo/boulder.json` records completed grimoire work for the same session.
8. Runtime verification writes `.omo/grimoire/verdict.json` with F1 through F4 results.
9. If Task 8 status is `clear-noop` and the F1 through F4 JSON all approve, the loop marks `✨ Cast` and ends without a commit or push.
10. If Task 8 status is `fixed` and the F1 through F4 JSON all approve, the driver makes one scoped bot commit and pushes it to the PR head. This is nonterminal and keeps `🔮 Casting…` in place.
11. The bot push creates a `pull_request.synchronize` event. Grimoire re-runs the full review path on the new head instead of trusting the previous fix.

A fixed push is a continuation signal, not success. Terminal success is the later no-op pass with all four verification fields set to `APPROVE` in JSON.

## Agent Mapping

Review uses team-mode style lenses in read-only form. The current local contract records security, correctness, maintainability, and repo-policy findings in `.omo/ci/review-findings.json`. Real mode must not edit files, commit, push, comment, change labels, or mutate GitHub during review.

Design is Prometheus plus OpenSpec. Prometheus stays on markdown planning, and the stage must cite enough OpenSpec evidence before work can continue. Metis, Oracle, and Momus are represented as planning pressure in the OMO configuration and prompt policy. The design output is a plan or a halt note, never an implementation patch.

Fix execution belongs to Atlas and its executor path, with Sisyphus or Hephaestus available for actual code work under Atlas control. The fix stage first checks Task 6 and Task 7 artifacts, then prepares the Atlas handoff. It treats PR-touched files, declared direct test/docs/spec extras, and cited OpenSpec paths as the allowed mutation surface.

Boulder continuation is Atlas-driven. The driver starts grimoire work with `/start-work`, continues the same session, and accepts completion only when `.omo/boulder.json` identifies completed grimoire work with matching session and elapsed metadata.

Verification is F1 through F4 at runtime:

- `f1_oracle`: plan and requirement compliance.
- `f2_quality`: workflow, script, and implementation quality.
- `f3_real_qa`: runtime or CI evidence quality.
- `f4_scope`: scope fidelity and mutation boundaries.

The verifier writes those fields as machine enums in `.omo/grimoire/verdict.json`. Model prose is not a control signal.

## Triggers

`.github/workflows/grimoire.yml` uses `pull_request` only. The allowed actions are `opened`, `ready_for_review`, `synchronize`, and `reopened`.

The workflow has no `pull_request_target` trigger. It also has no `push` trigger. The manual healthcheck workflow is separate: `.github/workflows/grimoire-attune.yml` uses `workflow_dispatch` and doesn't run the PR review loop.

Draft PRs are skipped at the job boundary and checked again before secrets or checkout. Two safe off-switches also stop the loop before secrets and checkout:

- Repository variable `GRIMOIRE_DISABLED` set to `1`.
- PR label `no-grimoire`.

Safe usage examples, not run by this document:

```bash
# Repository or environment operator example, uses a placeholder repo.
gh variable set GRIMOIRE_DISABLED --repo OWNER/REPO --body 1

# PR operator example, uses a placeholder PR number.
gh pr edit 123 --repo OWNER/REPO --add-label no-grimoire
```

Safe local checks that perform no live GitHub mutation:

```bash
# Confirm the workflow has the variable and label guards.
rg -n 'GRIMOIRE_DISABLED|no-grimoire|pull_request_target' .github/workflows/grimoire.yml

# Confirm this repo has no push-triggered grimoire workflow.
rg -n '^  push:|pull_request_target' .github/workflows/*.yml
```

Remove the off-switch only after the reason is resolved and the next run is allowed by the operator. Removing it is also a live GitHub mutation, so this guide records only the shape, not a run.

## Termination

Termination is jq-only over `.omo/grimoire/verdict.json`. The driver gets the predicate from `.github/scripts/grimoire-verify.sh --jq-expression` and requires:

- `schema_version` equals `1`.
- `stage` equals `grimoire-verify`.
- `notes` exists and has per-lens objects.
- `f1_oracle`, `f2_quality`, `f3_real_qa`, and `f4_scope` are all exactly `APPROVE`.
- `approved` is `true`.

Missing verdict files, malformed JSON, missing keys, invalid enum values, and any `REJECT` fail closed. Free-form approval text from a model, comment, log line, or summary is ignored.

The loop has no semantic iteration cap. It uses a wall-clock liveness timeout and heartbeat for stuck boulder work or persistent non-approval. That timeout is a safety guard, not a success condition.

No empty commit is allowed. If Task 8 reports `clear-noop`, the driver records `noop-approved` only after F1 through F4 approve, then exits without commit or push. If Task 8 reports `fixed`, the driver stages only declared scoped paths and refuses an empty staged diff. A real fixed push is nonterminal because it must trigger `pull_request.synchronize` and a fresh re-review.

## OpenSpec Binding

OpenSpec is the authority source for grimoire design. The design stage consumes review findings and OpenSpec evidence from non-archived files under `openspec/specs` and `openspec/changes`, unless an archived spec is passed explicitly.

Spec sufficiency means the design artifact can tie each intended fix to concrete OpenSpec evidence. Binding evidence is written in `.omo/ci/spec-sufficiency.json` through fields such as `spec_sufficient`, `bindings`, `missing`, `safety_default_gaps`, `suggested_spec_patch`, `plan_path`, and `halt_reason`.

If evidence is missing or ambiguous, the system fails closed. The spec-gap renderer writes a five-section comment artifact with Summary, Intended Work, Missing OpenSpec Evidence, Suggested Spec Items, and How To Rerun. The status artifact sets `should_comment=true`, `should_halt=true`, `github_mutation_performed=false`, and `no_code_or_push_action=true` for local render paths. The real PR comment path still requires PAT auth and grounded PR metadata.

Manual response to a spec gap should update OpenSpec first, usually through the project OpenSpec workflow, then rerun grimoire by updating the PR. The correct response is not to let the model infer the missing contract.

## Security Model

GitHub auth is PAT-only for grimoire. Checkout, git fetch, git push, `gh`, comments, and labels must use `GRIMOIRE_PAT` first or runner-provided `CODEX_LOOP_PAT` second. The workflow must not use `GITHUB_TOKEN` or a GitHub App token for grimoire GitHub operations.

Minimal fine-grained PAT scopes for this repository are:

- Contents: read/write.
- Pull requests: read/write.
- Issues: read/write.
- Metadata: read.

The Workflows scope is not needed because trusted-controller halts on `.github/**` changes. For a classic PAT, `repo` alone is enough for this repo. Don't add workflow permission for grimoire.

Trusted-controller protects the control plane before model credentials are used. The workflow fetches the base repo at the expected base SHA into runner temp with PAT auth, verifies trusted helper scripts and config, then runs the base copy of the controller and driver. PR-head controller edits are not trusted for the current run.

Protected paths are:

- `.github/**`.
- `.opencode/**`.
- `opencode.json`.
- Any root or nested `AGENTS.md`.
- `docs/SECURITY.md`.
- `docs/REVIEW_CHECKLIST.md`.
- `docs/FORKED_RELAYER_CRATE.md`.
- `docs/PUBLISHING_DISABLED.md`.

When protected paths are touched, the controller writes status `protected`, sets read-only mode, disables model execution, writes, commits, pushes, and general GitHub mutation, and records `push_attempts=0`. The protected-path comment helper renders the reason from trusted base material. In live post mode it still uses the resolved PAT path, not the default token.

Secret handling is source-label only. The resolver may log labels such as `secrets.AI_RELAY_API_KEY`, `runner_env.AI_RELAY_API_KEY`, `secret`, or `runner-env`. It must never log raw values, lengths, prefixes, hashes, fingerprints, private keys, token-bearing URLs, or transformed secret material.

Task 20 relay auth is dual-source. `AI_RELAY_API_KEY_SECRET` receives the GitHub secret `secrets.AI_RELAY_API_KEY` when present. If that is empty, the workflow uses the self-hosted runner environment variable `AI_RELAY_API_KEY`. The selected value is masked, exported as `AI_RELAY_API_KEY`, and consumed by `opencode.json` through `{env:AI_RELAY_API_KEY}`. Missing both sources fails closed before opencode or model calls.

The ai-relay provider is configured in `opencode.json` with `@ai-sdk/openai`, the relay base URL in source config, and models including `gpt-5.5`. All CI agents and categories use `ai-relay/gpt-5.5`. Local Anthropic mappings for `sisyphus` and `prometheus` are replaced in CI by `ai-relay/gpt-5.5` with the `xhigh` variant.

Variant tiers in CI:

- `xhigh`: heavy agents plus `visual-engineering`, `ultrabrain`, `deep`, `artistry`, and `unspecified-high`.
- `medium`: `librarian`, `explore`, `sisyphus-junior`, `quick`, and `unspecified-low`.
- `high`: `writing`.

## Operations

Use the off-switches before live grimoire starts when a PR must not run automation. Set `GRIMOIRE_DISABLED=1` for repo-wide pause, or add `no-grimoire` to a single PR. The examples in Triggers are command shapes only. They are not evidence that this task changed repository variables or PR labels.

### PR Label Lifecycle

Task 19 labels are display-only status, not durable loop state. The driver and workflow never decide review, fix, push, or termination behavior from labels; machine decisions still come from trusted-controller JSON, Task 6/7/8 artifacts, `.omo/boulder.json`, and F1 through F4 verdict JSON.

The helper `.github/scripts/grimoire-labels.sh` manages exactly these grimoire labels and preserves unrelated human labels:

- `🔮 Casting…`: color `#7c3aed`, description `Grimoire review/autofix loop is running.`
- `✨ Cast`: color `#10b981`, description `Grimoire review/autofix loop completed cleanly.`
- `💨 Fizzled`: color `#6b7280`, description `Grimoire review/autofix loop halted or failed closed.`

Live mode is PAT-only. The helper derives `GH_TOKEN` for each `gh` call from `GRIMOIRE_PAT` first or `CODEX_LOOP_PAT` second, ensures those three label definitions exist before PR label edits, and fails closed before `gh` if PAT, repository, or PR metadata is missing. It does not use the default Actions token as a fallback.

The transition contract is check-before-change and idempotent:

- `running` adds `🔮 Casting…` only when none of the terminal grimoire labels is already present. Repeated `running` calls do nothing when `🔮 Casting…` is already present, and they do not re-add it over `✨ Cast` or `💨 Fizzled`.
- `done` removes `🔮 Casting…` and `💨 Fizzled`, then adds `✨ Cast` once.
- `fizzled` removes `🔮 Casting…` and `✨ Cast`, then adds `💨 Fizzled` once.

Workflow placement is intentionally narrow: draft/off-switch/no-grimoire skips still happen before secrets and checkout; `running` is invoked only after PAT auth and trusted base-controller material are available; protected-path halt invokes `fizzled` from trusted base material before model/write/fix/commit/push; and the cast driver invokes `running`, terminal `done`, and fail-closed `fizzled` without marking `done` on a fixed-push nonterminal branch.

Current Task 19 evidence is local/dry-run only in `.omo/evidence/orl-task-19-transition.txt`, `.omo/evidence/orl-task-19-nochurn.txt`, and `.omo/evidence/orl-task-19-fizzled.txt`. No live `gh label create`, PR label edit, issue event timeline, or live workflow run is claimed here.

For a label issue, prefer the normal off-switches above rather than treating labels as control state. If a live label mutation fails, keep the PR loop fail-closed and inspect the local `.omo/ci/grimoire-label-*.json` artifact or the workflow step log without printing token values.

For a spec-gap halt, read `.omo/ci/spec-gap-comment.md`, update the missing OpenSpec material, and push the spec change through normal review. Grimoire should then re-run from `synchronize`. Don't bypass the halt by editing generated artifacts.

For a protected-path halt, review the protected controller or security change manually. If the automation change is intended, merge or otherwise update trusted base material first, then rerun grimoire from that trusted base. Don't trust PR-head workflow, script, or opencode config edits to authorize themselves.

Rollback options are operational, not automatic:

- Add `no-grimoire` to the PR for a single-PR pause.
- Set `GRIMOIRE_DISABLED=1` for a repo-wide pause.
- Revert the grimoire workflow, script, or config change through the normal human-reviewed process.
- Keep Codex retired unless a separate reviewed rollback explicitly restores it.

Evidence locations:

- `.omo/evidence/`: task evidence and deterministic local checks.
- `.omo/ci/`: stage JSON, design plans, spec-gap comments, fix status, and driver decisions.
- `.omo/grimoire/verdict.json`: F1 through F4 runtime verdict.
- `.omo/boulder.json`: Atlas boulder continuation state.
- `.omo/notepads/grimoire/`: accumulated learnings and open caveats.

Live blockers must stay honest. At the time of this guide, no current evidence in this task proves a live end-to-end PR loop, live attune model smoke, live spec-gap PR comment, live protected-path comment, live label mutation, or live fixed-push and synchronize re-review. Those claims need current run URLs, logs, artifacts, and PR state from Tasks 15 through 17 or later live gates.

Credential rotation:

- Rotate `AI_RELAY_API_KEY` upstream, then update either the GitHub secret or the self-hosted runner environment source. Restart or reload the runner when runner env changes. Record only the selected source label.
- Rotate `GRIMOIRE_PAT` by minting a repo-scoped least-privilege PAT, updating `GRIMOIRE_PAT` or runner `CODEX_LOOP_PAT`, and validating checkout, comment, label, and push paths in a live gate before revoking the old token.

## Migration

Grimoire is the active PR review path for this pilot repo. Current `.github/workflows` contains `grimoire.yml` for pull-request review and `grimoire-attune.yml` for manual healthcheck. Codex workflow entrypoints are absent in the current tree.

The migration goal is not to rebuild the old Codex loop in another shell. The retired Codex path used bespoke review, design, fix, dispatch, and terminal logic. Grimoire instead uses opencode plus OMO agent roles, OpenSpec binding, trusted-controller checks, and the F1 through F4 JSON verdict contract.

Do not add new Codex workflow entrypoints, `openai/codex-action`, `setup-codex-review`, or branch-pinned production dependencies as part of grimoire operation. If rollback ever restores Codex material, that should be a separate human-reviewed change with actionlint, auth review, and a clear reason.

Codex-to-opencode migration status is local and static in this branch until live gates run. The current branch documents the active target path and local contracts, but it doesn't prove live PR convergence yet.
