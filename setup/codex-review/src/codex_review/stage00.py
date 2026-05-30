"""Stage00 resolve gate contract logic."""

from __future__ import annotations

from typing import Any

from .validators import require_keys, require_schema_version


THREAD_INVENTORY_SCHEMA = "codex.stage00.thread_inventory.v1"
RESOLVE_GATE_SCHEMA = "codex.stage00.resolve_gate.v1"


def _require_string(value: Any, field: str) -> str:
    text = str(value or "").strip()
    if not text:
        raise ValueError(f"{field} is required")
    return text


def _validate_comment(thread_id: str, comment: Any) -> None:
    if not isinstance(comment, dict):
        raise ValueError(f"{thread_id} comments must be objects")
    require_keys(comment, ["comment_node_id", "body_sha256"])
    _require_string(comment.get("comment_node_id"), f"{thread_id} comment_node_id")
    body_sha = _require_string(comment.get("body_sha256"), f"{thread_id} body_sha256")
    if len(body_sha) != 64:
        raise ValueError(f"{thread_id} body_sha256 must be a sha256 hex digest")


def _validate_thread(raw: Any) -> dict[str, Any]:
    if not isinstance(raw, dict):
        raise ValueError("thread entries must be objects")
    require_keys(raw, ["thread_id", "root_cause_key", "comments", "forced_state", "needs_human_hint"])
    thread_id = _require_string(raw.get("thread_id"), "thread_id")
    _require_string(raw.get("root_cause_key"), f"{thread_id} root_cause_key")
    forced_state = str(raw.get("forced_state") or "").strip()
    if forced_state not in {"", "needs_human"}:
        raise ValueError(f"{thread_id} has unknown forced_state: {forced_state}")
    hint = str(raw.get("needs_human_hint") or "").strip()
    if forced_state == "needs_human" and not hint:
        raise ValueError(f"{thread_id} forced needs_human requires needs_human_hint")
    comments = raw.get("comments")
    if not isinstance(comments, list) or not comments:
        raise ValueError(f"{thread_id} comments must be a non-empty array")
    for comment in comments:
        _validate_comment(thread_id, comment)
    return {
        **raw,
        "thread_id": thread_id,
        "forced_state": forced_state,
        "needs_human_hint": hint,
    }


def validate_thread_inventory(payload: dict[str, Any]) -> list[dict[str, Any]]:
    require_schema_version(payload, THREAD_INVENTORY_SCHEMA)
    require_keys(payload, ["repository", "pr_number", "base_ref", "base_sha", "head_sha", "threads"])
    threads = payload.get("threads")
    if not isinstance(threads, list):
        raise ValueError("threads must be an array")
    validated = [_validate_thread(thread) for thread in threads]
    seen: set[str] = set()
    for thread in validated:
        thread_id = str(thread["thread_id"])
        if thread_id in seen:
            raise ValueError(f"duplicate thread_id: {thread_id}")
        seen.add(thread_id)
    return validated


def build_resolve_gate_result(payload: dict[str, Any]) -> dict[str, Any]:
    threads = validate_thread_inventory(payload)
    thread_ids = [str(thread["thread_id"]) for thread in threads]
    needs_human = [
        str(thread["thread_id"])
        for thread in threads
        if str(thread.get("forced_state") or "") == "needs_human"
    ]
    if needs_human:
        status = "needs_human"
    elif thread_ids:
        status = "needs_lifecycle_review"
    else:
        status = "clear"
    stop_reasons = [
        f"{thread['thread_id']}: {thread.get('needs_human_hint')}"
        for thread in threads
        if str(thread.get("forced_state") or "") == "needs_human"
    ]
    return {
        "schema_version": RESOLVE_GATE_SCHEMA,
        "stage": "stage00-resolve-gate",
        "status": status,
        "can_continue": status == "clear",
        "thread_count": len(thread_ids),
        "thread_ids": thread_ids,
        "needs_human_thread_ids": needs_human,
        "stop_reasons": stop_reasons,
    }
