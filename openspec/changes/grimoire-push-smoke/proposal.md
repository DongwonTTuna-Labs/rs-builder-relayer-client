# Change: Grimoire Push Smoke

## Why
We need one live positive smoke proving the merged Grimoire reusable control plane can perform a benign docs-only scoped autofix from a normal same-repo pull request, push exactly one bot commit, and re-review on `pull_request.synchronize`.

## What Changes
- Add an OpenSpec-backed smoke fixture for `grimoire-push-smoke`.
- Add a deterministic directive at `docs/GRIMOIRE_PUSH_SMOKE.spec.md`.
- Intentionally leave `docs/GRIMOIRE_PUSH_SMOKE.md` absent so Grimoire can create it.

## Non-Goals
- Do not test negative paths such as `grimoire:disabled`, protected-path halt, or spec-gap halt.
- Do not change source code, workflows, configuration, credentials, relayer behavior, signing, nonce handling, authentication, or live-capable venue behavior.
- Do not merge this smoke PR.

## Success Signal
Grimoire creates exactly one new file, `docs/GRIMOIRE_PUSH_SMOKE.md`, with the canonical content from `docs/GRIMOIRE_PUSH_SMOKE.spec.md`; pushes exactly one commit with message `chore(grimoire): apply scoped cast fix`; then a synchronize re-review reaches terminal `✨ Cast`.
