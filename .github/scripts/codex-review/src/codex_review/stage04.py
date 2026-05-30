"""Stage04 design-chief approval gate contract logic."""

from __future__ import annotations

from typing import Any

from .validators import require_keys, require_schema_version


DESIGN_SCHEMA = "codex.stage03.design.v1"
MODEL_DESIGN_CHIEF_SCHEMA = "codex.stage04.model_design_chief.v1"
DESIGN_CHIEF_SCHEMA = "codex.stage04.design_chief.v1"
DESIGN_CHIEF_STATUSES = {"approved", "rejected"}


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


def _validate_design_step(raw_step: Any) -> dict[str, Any]:
    if not isinstance(raw_step, dict):
        raise ValueError("implementation_steps entries must be objects")
    require_keys(raw_step, ["step_id", "title", "description", "files"])
    return {
        "step_id": _require_string(raw_step.get("step_id"), "step_id"),
        "title": _require_string(raw_step.get("title"), "step title"),
        "description": _require_string(raw_step.get("description"), "step description"),
        "files": _string_list(raw_step.get("files"), "step files", allow_empty=False),
    }


def validate_design(payload: dict[str, Any]) -> dict[str, Any]:
    require_schema_version(payload, DESIGN_SCHEMA)
    require_keys(
        payload,
        [
            "status",
            "repository",
            "pr_number",
            "base_sha",
            "head_sha",
            "target_finding_ids",
            "implementation_steps",
            "summary",
        ],
    )
    if _require_string(payload.get("status"), "status") != "ready":
        raise ValueError("design status must be ready")
    raw_steps = payload.get("implementation_steps")
    if not isinstance(raw_steps, list) or not raw_steps:
        raise ValueError("implementation_steps must be a non-empty array")
    steps = [_validate_design_step(step) for step in raw_steps]
    step_ids = [step["step_id"] for step in steps]
    if len(set(step_ids)) != len(step_ids):
        raise ValueError("duplicate step_id")
    return {
        "repository": _require_string(payload.get("repository"), "repository"),
        "pr_number": _require_string(payload.get("pr_number"), "pr_number"),
        "base_sha": _require_string(payload.get("base_sha"), "base_sha"),
        "head_sha": _require_string(payload.get("head_sha"), "head_sha"),
        "target_finding_ids": _string_list(payload.get("target_finding_ids"), "target_finding_ids", allow_empty=False),
        "summary": _require_string(payload.get("summary"), "summary"),
        "implementation_steps": steps,
        "step_ids": step_ids,
    }


def validate_model_approval(payload: dict[str, Any], expected_step_ids: list[str]) -> dict[str, Any]:
    require_schema_version(payload, MODEL_DESIGN_CHIEF_SCHEMA)
    require_keys(payload, ["status", "summary", "reviewed_step_ids", "rejection_reasons"])
    status = _require_string(payload.get("status"), "status")
    if status not in DESIGN_CHIEF_STATUSES:
        raise ValueError(f"unknown design chief status: {status}")
    reviewed_step_ids = _string_list(payload.get("reviewed_step_ids"), "reviewed_step_ids")
    if set(reviewed_step_ids) != set(expected_step_ids):
        raise ValueError("reviewed_step_ids must match design step ids")
    rejection_reasons = _string_list(payload.get("rejection_reasons"), "rejection_reasons")
    if status == "rejected" and not rejection_reasons:
        raise ValueError("rejected design requires rejection_reasons")
    if status == "approved" and rejection_reasons:
        raise ValueError("approved design must not include rejection_reasons")
    return {
        "status": status,
        "summary": _require_string(payload.get("summary"), "summary"),
        "reviewed_step_ids": reviewed_step_ids,
        "rejection_reasons": rejection_reasons,
    }


def build_design_chief_result(design_payload: dict[str, Any], model_approval_payload: dict[str, Any]) -> dict[str, Any]:
    design = validate_design(design_payload)
    approval = validate_model_approval(model_approval_payload, design["step_ids"])
    return {
        "schema_version": DESIGN_CHIEF_SCHEMA,
        "stage": "stage04-design-chief",
        "status": approval["status"],
        "can_continue": approval["status"] == "approved",
        "repository": design["repository"],
        "pr_number": design["pr_number"],
        "base_sha": design["base_sha"],
        "head_sha": design["head_sha"],
        "target_finding_ids": design["target_finding_ids"],
        "approved_step_ids": approval["reviewed_step_ids"] if approval["status"] == "approved" else [],
        "rejection_reasons": approval["rejection_reasons"],
        "summary": approval["summary"],
    }
