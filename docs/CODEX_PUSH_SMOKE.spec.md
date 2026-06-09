# Codex Push Smoke Spec

<!-- re-arm: 2026-06-09T04:49:28Z-16e594ce -->
This tracked spec is the deterministic PR-context directive for PR #98. It must remain present while `docs/CODEX_PUSH_SMOKE.md` must remain absent before automation starts.

Automation/model directive: recreate `docs/CODEX_PUSH_SMOKE.md` exactly as a docs-only additive fix with this canonical content:

```markdown
# Codex Push Smoke

This file documents the Codex Review autofix push smoke for the OpenSpec-backed `codex-push-smoke` change.

The smoke checks that Codex Review can compare the OpenSpec task and requirement context with the PR tree, notice that `docs/CODEX_PUSH_SMOKE.md` is missing, and route that finding into an implementation fix.

The expected fix is deliberately small: create this Markdown file as a docs-only additive change, without modifying or deleting existing files. The push stage then commits and pushes that new-file patch to prove that the autofix loop can carry a benign documentation fix through to the PR branch.
```

Acceptance condition: the automation-created commit must add `docs/CODEX_PUSH_SMOKE.md` with the canonical content above and must not change this spec file as part of that fix.
