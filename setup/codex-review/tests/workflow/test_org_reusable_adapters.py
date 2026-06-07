"""Contract tests for the thin org-reusable Codex loop adapters.

The three adapters (review / manual / dispatch) forward typed inputs to the
home-server-infra reusable core, pinned to a single 40-char commit SHA. After
the trusted core was promoted to a private SHA-pinned composite action, the
adapters no longer carry a read token or a trusted_core_ref, and loop state
stays a core concern: adapters only forward state pointers when a prior run
produced them, and never fabricate a seed artifact.
"""
from __future__ import annotations

import re
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[4]
WORKFLOWS = ROOT / ".github" / "workflows"

REVIEW = "codex-loop-review-adapter.yml"
MANUAL = "codex-loop-manual-adapter.yml"
DISPATCH = "codex-loop-dispatch-adapter.yml"
ADAPTERS = (REVIEW, MANUAL, DISPATCH)

REUSABLE = "DongwonTTuna-Labs/home-server-infra/.github/workflows/codex-loop-reusable.yml"
SHA_RE = re.compile(r"^[0-9a-f]{40}$")


def _text(name: str) -> str:
    return (WORKFLOWS / name).read_text(encoding="utf-8")


def _doc(name: str) -> dict:
    return yaml.safe_load(_text(name))


def _on(doc: dict) -> dict:
    # PyYAML parses the bare key ``on:`` as the boolean True (YAML 1.1).
    return doc.get("on", doc.get(True))


def _only_job(name: str) -> dict:
    jobs = _doc(name)["jobs"]
    assert len(jobs) == 1, f"{name}: expected exactly one (thin) job"
    return next(iter(jobs.values()))


def _pinned_sha(name: str) -> str:
    uses = _only_job(name)["uses"]
    repo, _, sha = uses.partition("@")
    assert repo == REUSABLE, f"{name}: unexpected reusable target {repo}"
    return sha


def test_all_adapters_pin_core_at_main():
    # Consumers track the core at @main (no SHA pin / re-pin churn).
    for name in ADAPTERS:
        assert _pinned_sha(name) == "main", f"{name}: must pin the core @main"


def test_no_trusted_core_ref_or_read_token_anywhere():
    for name in ADAPTERS:
        text = _text(name)
        assert "trusted_core_ref" not in text, f"{name}: trusted_core_ref must be gone"
        assert "CODEX_TRUSTED_CORE_READ_TOKEN" not in text, f"{name}: read token must be gone"


def test_no_dry_run_or_enable_live_autofix_flags():
    # The loop is always-live; the dry-run scaffolding was removed from the core,
    # so adapters must not forward or expose those flags.
    for name in ADAPTERS:
        text = _text(name)
        assert "dry_run" not in text, f"{name}: dry_run flag must be gone"
        assert "enable_live_autofix" not in text, f"{name}: enable_live_autofix flag must be gone"


def test_adapters_pass_no_secrets():
    # Credentials live on the runner env; adapters map no secrets.
    for name in ADAPTERS:
        assert "secrets" not in _only_job(name), f"{name}: must not map any secrets"
        assert "${{ secrets." not in _text(name), f"{name}: no secrets.* references"


def test_adapters_drop_max_iterations():
    for name in ADAPTERS:
        assert "max_iterations" not in _text(name), f"{name}: max_iterations must be gone"


def test_review_adapter_is_thin_and_omits_state_pointers():
    job = _only_job(REVIEW)
    with_block = job["with"]
    # Initial PR entry: no prior state, so no pointers are forwarded and the
    # core bootstraps empty state. The adapter must not fabricate a seed.
    assert "state_run_id" not in with_block
    assert "state_artifact_name" not in with_block
    assert with_block["stage"] == "review"
    text = _text(REVIEW)
    assert "upload-artifact" not in text, "review adapter must not create a seed state artifact"
    assert "loop-state.v1" not in text and "schema_version" not in text


def test_manual_adapter_exposes_optional_state_pointers_and_forwards_them():
    doc = _doc(MANUAL)
    inputs = _on(doc)["workflow_dispatch"]["inputs"]
    for field in ("state_run_id", "state_artifact_name"):
        assert field in inputs, f"manual adapter must expose optional {field} input"
        assert inputs[field].get("required") in (False, None)
        assert inputs[field].get("default", "") == ""
    with_block = _only_job(MANUAL)["with"]
    assert with_block["state_run_id"] == "${{ inputs.state_run_id }}"
    assert with_block["state_artifact_name"] == "${{ inputs.state_artifact_name }}"


def test_manual_adapter_full_stage_enum():
    inputs = _on(_doc(MANUAL))["workflow_dispatch"]["inputs"]
    assert inputs["stage"]["options"] == ["review", "design", "fix", "push"]


def test_dispatch_adapter_forwards_payload_state_pointers():
    with_block = _only_job(DISPATCH)["with"]
    assert with_block["state_run_id"] == "${{ github.event.client_payload.state_run_id }}"
    assert with_block["state_artifact_name"] == "${{ github.event.client_payload.state_artifact_name }}"
