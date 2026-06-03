"""Stage routing helpers."""
from __future__ import annotations

from typing import Any

from codex_review.core.output import set_route_outputs, write_json_output, write_output


def route_after_stage00(gate_result: dict[str, Any], loop_state: dict[str, Any] | None = None, pr_context: dict[str, Any] | None = None) -> dict[str, Any]:
    route=gate_result.get("route") or gate_result.get("next_route") or "stop_noop"
    return {"schema_version": "route.v1", "after": "stage00", "route": route, "run_review": route == "run_review", "run_design": route == "run_design_from_existing_threads", "needs_human": route == "stop_needs_human"}


def route_after_stage02(techlead_decision: dict[str, Any]) -> dict[str, Any]:
    status=techlead_decision.get("status")
    items=techlead_decision.get("decisions") or techlead_decision.get("items") or []
    if status == "needs_human" or any(i.get("action") == "needs_human" for i in items):
        route="stop_needs_human"
    elif techlead_decision.get("needs_design") or any(i.get("action") in {"needs_design","publish_and_fix_now","summary_only_fix_now"} for i in items):
        route="run_design"
    elif items:
        route="stop_after_publish"
    else:
        route="stop_lgtm"
    return {"schema_version": "route.v1", "after": "stage02", "route": route}


def route_after_stage04(chief_decision: dict[str, Any]) -> dict[str, Any]:
    status=chief_decision.get("status")
    route="run_stage05" if status == "approved_for_fix" else ("stop_needs_human" if status == "needs_human" else "stop_noop")
    return {"schema_version": "route.v1", "after": "stage04", "route": route}


def route_after_stage07(push_result: dict[str, Any]) -> dict[str, Any]:
    pushed=bool(push_result.get("pushed") or push_result.get("status") == "pushed")
    return {"schema_version": "route.v1", "after": "stage07", "route": "record_reentry" if pushed else "stop_push_blocked", "pushed": pushed}


def write_route_outputs(route: dict[str, Any]) -> None:
    write_json_output("route_object", route)
    write_output("route", route.get("route"))
    set_route_outputs(route)
