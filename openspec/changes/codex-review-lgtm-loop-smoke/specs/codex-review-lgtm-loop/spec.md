# Codex Review LGTM Loop

## ADDED Requirements

### Requirement: OpenSpec-Backed Review Source

Codex Review SHALL treat same-repo OpenSpec artifacts linked from the PR title or body as authoritative source material for review and fix planning.

#### Scenario: PR body links an OpenSpec change path

- **GIVEN** a PR body contains `openspec/changes/codex-review-lgtm-loop-smoke`
- **WHEN** Codex Review bootstraps PR context
- **THEN** it SHALL collect the proposal, design, tasks, specs, and OpenSpec config artifacts into review context
- **AND** Stage01 reviewers SHALL compare the PR tree against those artifacts.

### Requirement: Repository Inspection Evidence

Codex Review SHALL require Stage01 through Stage04 model outputs to include concrete repository inspection evidence.

#### Scenario: model reports no findings

- **GIVEN** a Stage01 reviewer emits zero findings
- **WHEN** the result is validated
- **THEN** the result SHALL still include non-empty `inspection_evidence`
- **AND** each evidence item SHALL name a `pr-head` repository path, purpose, and observation.

### Requirement: Executable OpenSpec Plans

Codex Review SHALL promote executable OpenSpec-backed plans into the fix pipeline instead of stopping at generic human review.

#### Scenario: a docs-only OpenSpec task is incomplete

- **GIVEN** OpenSpec tasks require `docs/CODEX_REVIEW_LGTM_LOOP.md`
- **AND** the file is missing from the PR tree
- **WHEN** Stage02 through Stage04 route the finding
- **THEN** Stage03 SHALL produce an edit plan constrained to that docs file
- **AND** Stage04 SHALL approve the plan unless a real non-executable blocker exists.

### Requirement: App Token Writes Only

Codex Review SHALL perform PR mutation, issue creation, and review posting only with scoped GitHub App tokens.

#### Scenario: an autofix commit is pushed

- **GIVEN** an OpenSpec-backed fix plan is approved
- **AND** repository policy allows push
- **WHEN** Stage07 pushes a commit
- **THEN** the write SHALL use a scoped GitHub App token
- **AND** the workflow SHALL NOT grant broad write permissions to `GITHUB_TOKEN`.

### Requirement: Rootless Codex Action Runtime

Codex Review SHALL use `openai/codex-action` without requiring sudo or a long-lived Codex CLI installation on the self-hosted runner.

#### Scenario: a model step runs on the home server runner

- **GIVEN** the workflow receives relay credentials through `setup-codex-relay`
- **WHEN** `openai/codex-action` runs a model step
- **THEN** it SHALL use direct provider `codex-args` and `AI_RELAY_API_KEY`
- **AND** it SHALL NOT use `openai-api-key`, `responses-api-endpoint`, proxy startup, `sudo chmod`, or `sudo chown`.

### Requirement: Stale Run Cancellation

Codex Review SHALL cancel older in-progress runs for the same PR when a new push starts a replacement run.

#### Scenario: a PR branch receives a new commit

- **GIVEN** a Codex Review run is in progress for the PR
- **WHEN** a new commit is pushed to the same PR branch
- **THEN** GitHub Actions SHALL cancel the stale run
- **AND** the new run SHALL continue with the latest PR head SHA.
