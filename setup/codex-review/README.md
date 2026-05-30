# Codex Review V3

This directory contains the implementation surface for the v3 Codex review workflow.

The live workflow should stay thin. It should call commands from this package, pass artifacts between jobs, and keep privileged GitHub write operations outside model jobs.

## Phase 1 Scope

Phase 1 provides:

- deterministic JSON artifact IO
- fail-closed contract validation helpers
- an explicit OIDC relay action contract representation
- dry-run CLI entrypoints for stage00 through stage08

Phase 1 does not replace `.github/workflows/codex-pr-review.yml` yet.

## OIDC Contract

The only authentication path reused from the legacy/current workflow is the short-lived OIDC relay credential flow:

```text
DongwonTTuna-Labs/home-server-infra/.github/actions/setup-codex-relay@main
```

The workflow must grant `id-token: write` only to jobs that exchange GitHub OIDC for a relay credential.

Relay tokens and raw OIDC JWTs must be masked and must not appear in artifacts, logs, prompts, or model outputs.

## Stage Commands

The intended stage command names are:

```text
stage00-resolve-gate
stage01-review
stage02-techlead
stage03-design
stage04-design-chief
stage05-fix-dispatch
stage06-fix-merge
stage07-push
stage08-reentry
```

In Phase 1 these commands support only `--dry-run`.

## Non-Negotiable Rules

- Do not directly merge a PR.
- Do not run PR-head-controlled code in a privileged write path.
- Do not preserve the old workflow shape for compatibility.
- Do not treat local tests as final E2E verification.
- Do not mark the full goal complete without real OIDC mint, relay exchange, reviewer execution, and GPT Pro LGTM evidence.
