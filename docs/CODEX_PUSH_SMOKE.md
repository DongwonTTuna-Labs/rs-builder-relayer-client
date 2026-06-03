# Codex Push Smoke

This document is the intentionally missing docs artifact for the Codex Review autofix push smoke.

The smoke verifies that Codex Review can compare an OpenSpec-backed PR against the PR tree, detect that `docs/CODEX_PUSH_SMOKE.md` is required but absent, and route that finding into the design and autofix stages.

The expected fix plan is deliberately narrow: create this single documentation file as an additive docs-only change. The plan must not modify or delete existing files, and it must not touch Rust source, workflows, schemas, configuration, credentials, relayer settings, or live-capable behavior.

After semantic safety accepts the benign docs addition, the push stage commits the new file to the PR branch. That commit is the smoke signal that the review, planning, autofix, safety, and push flow can complete for a minimal missing-file task.
