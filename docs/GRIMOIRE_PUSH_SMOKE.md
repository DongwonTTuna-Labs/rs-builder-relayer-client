# Grimoire Push Smoke

This file documents the Grimoire reusable control-plane push smoke for the OpenSpec-backed `grimoire-push-smoke` change.

The smoke checks that Grimoire can compare the OpenSpec task and requirement context with the PR tree, notice that `docs/GRIMOIRE_PUSH_SMOKE.md` is missing, and route that finding through design, fix, verify, and scoped push.

The expected fix is deliberately small: create this Markdown file as a docs-only additive change, without modifying or deleting existing files. The push stage then commits and pushes that new-file patch to prove the reusable Grimoire control plane can carry a benign documentation fix through to the PR branch and trigger a `pull_request.synchronize` re-review.

This smoke must not modify source code, workflows, configuration, credentials, relayer behavior, signing, nonce handling, authentication, or live-capable venue behavior.
