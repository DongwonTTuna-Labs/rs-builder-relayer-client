# Codex Push Smoke

This file documents the Codex Review autofix push smoke for the OpenSpec-backed `codex-push-smoke` change.

The smoke checks that Codex Review can compare the OpenSpec task and requirement context with the PR tree, notice that `docs/CODEX_PUSH_SMOKE.md` is missing, and route that finding into an implementation fix.

The expected fix is deliberately small: create this Markdown file as a docs-only additive change, without modifying or deleting existing files. The push stage then commits and pushes that new-file patch to prove that the autofix loop can carry a benign documentation fix through to the PR branch.
