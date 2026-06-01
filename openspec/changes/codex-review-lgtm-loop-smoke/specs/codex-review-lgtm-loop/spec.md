# codex-review-lgtm-loop Specification

## ADDED Requirements

### Requirement: OpenSpec context is the review source of truth

Codex Review SHALL treat a linked OpenSpec change as authoritative context for the PR.
The OpenSpec context includes `proposal.md`, `design.md`, `tasks.md`, `specs/**/*.md`,
and repository OpenSpec config.

#### Scenario: PR body links an OpenSpec change path

- **GIVEN** a same-repository PR body contains `openspec/changes/codex-review-lgtm-loop-smoke`
- **WHEN** `bootstrap_event` runs
- **THEN** `openspec-context.json` has `present: true`
- **AND** `openspec-context.md` renders the proposal, design, tasks, and spec content

### Requirement: Review detects OpenSpec task incompletion

Codex Review SHALL compare the PR diff with OpenSpec tasks and specs. If a required
artifact is absent, the review SHALL produce an actionable finding instead of LGTM.

#### Scenario: Required loop documentation is absent

- **GIVEN** `tasks.md` requires `docs/CODEX_REVIEW_LGTM_LOOP.md`
- **AND** the PR diff does not add that file
- **WHEN** stage01 and stage02 run
- **THEN** the missing docs file is treated as an implementable OpenSpec-backed finding
- **AND** the finding is routed to design instead of generic `needs_human`

### Requirement: Executable OpenSpec-backed plans continue to autofix

Stage04 SHALL approve an OpenSpec-backed plan when it has a concrete edit sequence,
tests, acceptance criteria, and no real execution blocker.

#### Scenario: Docs-only plan has no blocker

- **GIVEN** stage03 emits an OpenSpec-backed plan for `docs/CODEX_REVIEW_LGTM_LOOP.md`
- **AND** the plan has tests and acceptance criteria
- **AND** the plan has no `execution_blockers`
- **WHEN** stage04 validates the chief decision
- **THEN** the route is `run_stage05`

### Requirement: Same-repository autofix remains bounded

Codex Review SHALL prepare and validate autofix work for same-repository PRs only.
The smoke fix SHALL be limited to `docs/CODEX_REVIEW_LGTM_LOOP.md`.

#### Scenario: Side-effect gates are disabled

- **GIVEN** `CODEX_REVIEW_ENABLE_PUSH` is not `true`
- **WHEN** stage07 validates the merged fix
- **THEN** no commit is pushed
- **AND** the artifact shows a dry-run patch for `docs/CODEX_REVIEW_LGTM_LOOP.md`

#### Scenario: Push gate is enabled after dry-run review

- **GIVEN** the dry-run patch only touches `docs/CODEX_REVIEW_LGTM_LOOP.md`
- **AND** `CODEX_REVIEW_ENABLE_PUSH` is temporarily set to `true`
- **WHEN** stage07 commit and push runs
- **THEN** it uses a GitHub App installation token
- **AND** it does not rely on `GITHUB_TOKEN` write permissions

### Requirement: Rootless model execution is preserved

Codex Review SHALL run model steps without the Responses proxy credential path that
requires sudo hardening.

#### Scenario: Model job sets up relay

- **GIVEN** a model job calls `setup-codex-relay`
- **WHEN** `openai/codex-action` runs
- **THEN** the action receives `codex-args` and `AI_RELAY_API_KEY`
- **AND** it does not receive `openai-api-key`
- **AND** it does not receive `responses-api-endpoint`
- **AND** logs do not contain `Start Responses API proxy`, `sudo chmod`, or `sudo chown`

### Requirement: Non-executable work routes to issue fallback

Codex Review SHALL use stage09 issue fallback when automation cannot safely mutate the
PR branch or when OpenSpec context is missing.

#### Scenario: Fork PR cannot be mutated

- **GIVEN** the PR head repository differs from the base repository
- **WHEN** an OpenSpec-backed fix is required
- **THEN** Codex Review does not push to the fork branch
- **AND** stage09 prepares an idempotent issue fallback
