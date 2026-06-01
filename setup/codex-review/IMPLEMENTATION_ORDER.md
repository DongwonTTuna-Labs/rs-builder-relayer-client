# Implementation Order

## Phase 1: Foundation

- `config.py`
- `env.py`
- `paths.py`
- `artifacts.py`
- `schema.py`
- `github_output.py`
- `security/redaction.py`

## Phase 2: GitHub trusted boundary

- `github/client.py`
- `github/pull_requests.py`
- `github/review_threads.py`
- `github/comments.py`
- `github/issues.py`
- `github/markers.py`
- `security/provenance.py`
- `security/checkout.py`

## Phase 3: Stage00 resolve gate

- `stage00_resolve_gate/collect.py`
- `stage00_resolve_gate/prompt.py`
- `stage00_resolve_gate/validate.py`
- `stage00_resolve_gate/apply.py`
- `stage00_resolve_gate/route.py`

## Phase 4: Review + Techlead

- `stage01_review/*`
- `stage02_techlead/*`

## Phase 5: Design + Design Chief

- `stage03_design/*`
- `stage04_design_chief/*`

## Phase 6: Fix + Merge + Push

- `stage05_fix_dispatch/*`
- `stage06_fix_merge/*`
- `stage07_push/*`
- `stage08_reentry/*`

## Phase 7: Workflow and tests

- `.github/workflows/codex-review-orchestrator.yml`
- `tests/workflow/*`
- `tests/unit/*`
