"""Stage08 reentry artifact contract logic."""

from __future__ import annotations

from typing import Any

from ...schema import require_keys, require_schema_version


PUSH_SCHEMA = "codex.stage07.push.v1"
RUN_STATE_SCHEMA = "codex.stage08.run_state.v1"
REENTRY_SCHEMA = "codex.stage08.reentry.v1"


def _require_string(value: Any, field: str) -> str:
    text = str(value or "").strip()
    if not text:
        raise ValueError(f"{field} is required")
    return text


def _require_sha(value: Any, field: str) -> str:
    text = _require_string(value, field)
    if len(text) != 40 or any(char not in "0123456789abcdef" for char in text.lower()):
        raise ValueError(f"{field} must be a sha160 hex digest")
    return text


def _non_negative_int(value: Any, field: str) -> int:
    if not isinstance(value, int) or value < 0:
        raise ValueError(f"{field} must be a non-negative integer")
    return value


def validate_push(payload: dict[str, Any]) -> dict[str, Any]:
    require_schema_version(payload, PUSH_SCHEMA)
    require_keys(payload, ["status", "can_continue", "repository", "pr_number", "pushed_head_sha", "pr_merged"])
    if _require_string(payload.get("status"), "status") != "pushed" or payload.get("can_continue") is not True:
        raise ValueError("stage08 requires pushed stage07 artifact")
    if payload.get("pr_merged") is True:
        raise ValueError("stage08 must not follow a merged PR")
    return {
        "repository": _require_string(payload.get("repository"), "repository"),
        "pr_number": _require_string(payload.get("pr_number"), "pr_number"),
        "pushed_head_sha": _require_sha(payload.get("pushed_head_sha"), "pushed_head_sha"),
    }


def validate_run_state(payload: dict[str, Any]) -> dict[str, Any]:
    require_schema_version(payload, RUN_STATE_SCHEMA)
    require_keys(payload, ["run_id", "event_name", "loop_count", "max_loops"])
    loop_count = _non_negative_int(payload.get("loop_count"), "loop_count")
    max_loops = _non_negative_int(payload.get("max_loops"), "max_loops")
    if loop_count >= max_loops:
        raise ValueError("same-run reentry loop limit reached")
    return {
        "run_id": _require_string(payload.get("run_id"), "run_id"),
        "event_name": _require_string(payload.get("event_name"), "event_name"),
        "loop_count": loop_count,
        "max_loops": max_loops,
    }


def build_reentry_result(push_payload: dict[str, Any], run_state_payload: dict[str, Any]) -> dict[str, Any]:
    push = validate_push(push_payload)
    run_state = validate_run_state(run_state_payload)
    return {
        "schema_version": REENTRY_SCHEMA,
        "stage": "stage08-reentry",
        "status": "same_run_reentry_ready",
        "can_continue": True,
        "same_run_reentry": True,
        "next_stage": "stage00-resolve-gate",
        "repository": push["repository"],
        "pr_number": push["pr_number"],
        "expected_head_sha": push["pushed_head_sha"],
        "source_run_id": run_state["run_id"],
        "source_event_name": run_state["event_name"],
        "next_event_name": run_state["event_name"],
        "loop_count": run_state["loop_count"] + 1,
        "max_loops": run_state["max_loops"],
    }
