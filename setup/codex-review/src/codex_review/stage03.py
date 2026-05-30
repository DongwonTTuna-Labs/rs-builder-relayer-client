"""Stage03 design artifact contract logic."""

from __future__ import annotations

from typing import Any

from .validators import require_keys, require_schema_version


REVIEW_SCHEMA = "codex.stage01.review.v1"
TECHLEAD_SCHEMA = "codex.stage02.techlead.v1"
MODEL_DESIGN_SCHEMA = "codex.stage03.model_design.v1"
DESIGN_SCHEMA = "codex.stage03.design.v1"


def _require_string(value: Any, field: str) -> str:
    text = str(value or "").strip()
    if not text:
        raise ValueError(f"{field} is required")
    return text


def _string_list(value: Any, field: str, *, allow_empty: bool = True) -> list[str]:
    if not isinstance(value, list):
        raise ValueError(f"{field} must be an array")
    items = [_require_string(item, f"{field} entry") for item in value]
    if not allow_empty and not items:
        raise ValueError(f"{field} must be a non-empty array")
    if len(set(items)) != len(items):
        raise ValueError(f"{field} must not contain duplicates")
    return items


def _validate_review(payload: dict[str, Any]) -> dict[str, str]:
    require_schema_version(payload, REVIEW_SCHEMA)
    require_keys(payload, ["repository", "pr_number", "base_sha", "head_sha"])
    return {
        "repository": _require_string(payload.get("repository"), "repository"),
        "pr_number": _require_string(payload.get("pr_number"), "pr_number"),
        "base_sha": _require_string(payload.get("base_sha"), "base_sha"),
        "head_sha": _require_string(payload.get("head_sha"), "head_sha"),
    }


def _validate_techlead(payload: dict[str, Any]) -> dict[str, Any]:
    require_schema_version(payload, TECHLEAD_SCHEMA)
    require_keys(payload, ["requires_design", "requires_fix", "blocking_finding_ids"])
    if payload.get("requires_design") is not True:
        raise ValueError("stage03 requires techlead.requires_design")
    blocking_ids = _string_list(payload.get("blocking_finding_ids"), "blocking_finding_ids", allow_empty=False)
    return {
        "requires_design": True,
        "requires_fix": bool(payload.get("requires_fix")),
        "blocking_finding_ids": blocking_ids,
    }


def _validate_step(raw_step: Any) -> dict[str, Any]:
    if not isinstance(raw_step, dict):
        raise ValueError("implementation_steps entries must be objects")
    require_keys(raw_step, ["step_id", "title", "description", "files"])
    return {
        "step_id": _require_string(raw_step.get("step_id"), "step_id"),
        "title": _require_string(raw_step.get("title"), "step title"),
        "description": _require_string(raw_step.get("description"), "step description"),
        "files": _string_list(raw_step.get("files"), "step files", allow_empty=False),
    }


def _validate_steps(raw_steps: Any) -> list[dict[str, Any]]:
    if not isinstance(raw_steps, list) or not raw_steps:
        raise ValueError("implementation_steps must be a non-empty array")
    steps = [_validate_step(step) for step in raw_steps]
    step_ids = [step["step_id"] for step in steps]
    if len(set(step_ids)) != len(step_ids):
        raise ValueError("duplicate step_id")
    return steps


def validate_model_design(payload: dict[str, Any], blocking_ids: list[str]) -> dict[str, Any]:
    require_schema_version(payload, MODEL_DESIGN_SCHEMA)
    require_keys(payload, ["summary", "target_finding_ids", "assumptions", "implementation_steps", "test_plan", "risk_notes"])
    target_ids = _string_list(payload.get("target_finding_ids"), "target_finding_ids", allow_empty=False)
    if set(target_ids) != set(blocking_ids):
        raise ValueError("target_finding_ids must match blocking_finding_ids")
    return {
        "summary": _require_string(payload.get("summary"), "summary"),
        "target_finding_ids": target_ids,
        "assumptions": _string_list(payload.get("assumptions"), "assumptions"),
        "implementation_steps": _validate_steps(payload.get("implementation_steps")),
        "test_plan": _string_list(payload.get("test_plan"), "test_plan", allow_empty=False),
        "risk_notes": _string_list(payload.get("risk_notes"), "risk_notes"),
    }


def build_design_result(
    review_payload: dict[str, Any],
    techlead_payload: dict[str, Any],
    model_design_payload: dict[str, Any],
) -> dict[str, Any]:
    review = _validate_review(review_payload)
    techlead = _validate_techlead(techlead_payload)
    model = validate_model_design(model_design_payload, techlead["blocking_finding_ids"])
    return {
        "schema_version": DESIGN_SCHEMA,
        "stage": "stage03-design",
        "status": "ready",
        "can_continue": True,
        "requires_design": True,
        "requires_fix": techlead["requires_fix"],
        "repository": review["repository"],
        "pr_number": review["pr_number"],
        "base_sha": review["base_sha"],
        "head_sha": review["head_sha"],
        "target_finding_ids": model["target_finding_ids"],
        "summary": model["summary"],
        "assumptions": model["assumptions"],
        "implementation_steps": model["implementation_steps"],
        "test_plan": model["test_plan"],
        "risk_notes": model["risk_notes"],
    }
