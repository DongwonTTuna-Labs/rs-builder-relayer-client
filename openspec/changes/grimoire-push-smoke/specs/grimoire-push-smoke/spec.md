## ADDED Requirements

### Requirement: Grimoire push smoke marker
The repository SHALL contain `docs/GRIMOIRE_PUSH_SMOKE.md` with the exact canonical Markdown content specified in `docs/GRIMOIRE_PUSH_SMOKE.spec.md`.

#### Scenario: Grimoire recreates the missing marker
- **GIVEN** a same-repo non-draft pull request for the `grimoire-push-smoke` change
- **AND** `docs/GRIMOIRE_PUSH_SMOKE.spec.md` exists
- **AND** `docs/GRIMOIRE_PUSH_SMOKE.md` is absent before Grimoire runs
- **WHEN** Grimoire runs review, design, fix, verify, and cast
- **THEN** Grimoire creates exactly `docs/GRIMOIRE_PUSH_SMOKE.md` with the canonical content from `docs/GRIMOIRE_PUSH_SMOKE.spec.md`
- **AND** Grimoire does not modify source code, workflows, configuration, credentials, relayer behavior, signing, nonce handling, authentication, or live-capable venue behavior
