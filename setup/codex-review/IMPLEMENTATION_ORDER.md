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

- `resolve_gate/collect.py`
- `resolve_gate/prompt.py`
- `resolve_gate/validate.py`
- `resolve_gate/apply.py`
- `resolve_gate/route.py`

## Phase 4: Review + Techlead

- `review/*`
- `techlead/*`

## Phase 5: Design + Design Chief

- `design/*`
- `design_chief/*`

## Phase 6: Fix + Merge + Push

- `fix_dispatch/*`
- `fix_merge/*`
- `push/*`
- `reentry/*`

## Phase 7: Workflow and tests

- `.github/workflows/codex-review-orchestrator.yml`
- `tests/workflow/*`
- `tests/unit/*`
