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


def test_no_adapter_forwards_stage_or_state_pointers():
    # The core runs the whole loop in one run, so adapters no longer select a
    # stage or carry cross-run state pointers. None of the three may forward
    # `stage`, `state_run_id`, or `state_artifact_name`.
    for name in ADAPTERS:
        with_block = _only_job(name)["with"]
        for field in ("stage", "state_run_id", "state_artifact_name"):
            assert field not in with_block, f"{name}: must not forward {field}"


def test_review_adapter_is_thin_and_omits_seed_state():
    text = _text(REVIEW)
    assert "upload-artifact" not in text, "review adapter must not create a seed state artifact"
    assert "loop-state.v1" not in text and "schema_version" not in text


def test_manual_adapter_drops_stage_and_state_inputs():
    inputs = _on(_doc(MANUAL))["workflow_dispatch"]["inputs"]
    for field in ("stage", "state_run_id", "state_artifact_name"):
        assert field not in inputs, f"manual adapter must not expose {field} input"
    # The remaining entry-point inputs are still present.
    for field in ("pr_number", "head_sha", "base_ref", "iteration", "correlation_id", "requested_by"):
        assert field in inputs, f"manual adapter must keep {field} input"
