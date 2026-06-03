# Codex Push Smoke

## Summary

Add an OpenSpec-backed smoke change that verifies the Codex Review autofix loop can implement a missing documentation file and push the fix commit.

The smoke intentionally leaves one safe documentation task incomplete: `docs/CODEX_PUSH_SMOKE.md` is specified here but is deliberately absent from this PR. Codex Review should detect the missing file, design a docs-only additive fix, create the file, and push the commit.

## Goals

- Prove the autofix stage produces an applicable, additive docs patch (a new file).
- Prove semantic safety approves a benign docs addition.
- Prove the push stage commits the fix to the PR branch.

## Non-Goals

- Do not change Rust crate public API.
- Do not change GitHub Actions workflow behavior.
- Do not add credentials, secrets, relay configuration, or runner configuration.
- Do not delete or modify any existing file; the only change is to add one new docs file.

## Scope

The only human-authored files in this PR are OpenSpec artifacts under `openspec/`. The missing implementation is a single docs file to be CREATED by the autofix loop:

- `docs/CODEX_PUSH_SMOKE.md`

## Smoke Signal

Codex Review should compare this OpenSpec change to the PR tree and identify that `tasks.md` requires `docs/CODEX_PUSH_SMOKE.md`, but the file is absent, then create it (an additive, docs-only fix) and push the commit.
