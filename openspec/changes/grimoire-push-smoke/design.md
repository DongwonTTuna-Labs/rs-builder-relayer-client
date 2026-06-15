# Design: Grimoire Push Smoke

## Scope
This change is a smoke fixture only. The active scope is limited to proving the Grimoire reusable control-plane live positive path on a benign documentation addition.

## Expected Automation Behavior
1. Review/design should notice that `docs/GRIMOIRE_PUSH_SMOKE.spec.md` requires `docs/GRIMOIRE_PUSH_SMOKE.md`.
2. Design should classify the missing marker as in scope for this smoke.
3. Fix should create only `docs/GRIMOIRE_PUSH_SMOKE.md` with the canonical content in `docs/GRIMOIRE_PUSH_SMOKE.spec.md`.
4. Verify should approve only if the patch is additive, docs-only, and limited to the marker file.
5. Cast should push exactly one scoped bot commit and rely on `pull_request.synchronize` for re-review.
6. After the marker exists, re-review should clear-noop and terminal-cast.

## Explicitly Forbidden
Grimoire must not modify source code, workflows, configuration, credentials, relayer behavior, signing, nonce handling, authentication, live-capable venue behavior, OpenSpec task checkboxes, or any file other than `docs/GRIMOIRE_PUSH_SMOKE.md`.

## Completion Rule
The smoke requirement is complete when `docs/GRIMOIRE_PUSH_SMOKE.md` exists with exact canonical content. The unchecked task in `tasks.md` is an input signal, not a file Grimoire should edit.
