"""Stage00 resolve gate contract logic."""

from __future__ import annotations

import hashlib
from typing import Any

from ...schema import require_keys, require_schema_version


THREAD_INVENTORY_SCHEMA = "codex.stage00.thread_inventory.v1"
RESOLVE_GATE_SCHEMA = "codex.stage00.resolve_gate.v1"
LIFECYCLE_SCHEMA = "codex.stage00.lifecycle.v1"


def _require_string(value: Any, field: str) -> str:
    text = str(value or "").strip()
    if not text:
        raise ValueError(f"{field} is required")
    return text


def _review_threads_connection(payload: dict[str, Any]) -> dict[str, Any]:
    try:
        connection = payload["data"]["repository"]["pullRequest"]["reviewThreads"]
    except (KeyError, TypeError) as exc:
        raise ValueError("reviewThreads connection is required") from exc
    if not isinstance(connection, dict):
        raise ValueError("reviewThreads connection must be an object")
    return connection


def _build_thread(raw: dict[str, Any]) -> dict[str, Any]:
    thread_id = _require_string(raw.get("id"), "review thread id")
    comments_connection = raw.get("comments") or {}
    if not isinstance(comments_connection, dict):
        raise ValueError(f"{thread_id} comments connection must be an object")
    comments = comments_connection.get("nodes") or []
    if not isinstance(comments, list):
        raise ValueError(f"{thread_id} comments nodes must be an array")
    forced_state = ""
    needs_human_hint = ""
    if comments_connection.get("pageInfo", {}).get("hasNextPage"):
        forced_state = "needs_human"
        needs_human_hint = "review thread has more than 50 comments"
    if not comments:
        forced_state = "needs_human"
        needs_human_hint = "review thread has no visible comments"
        comments = [{"id": f"{thread_id}:missing-comments", "databaseId": None, "body": ""}]
    return {
        "thread_id": thread_id,
        "file": raw.get("path"),
        "line": raw.get("line") or raw.get("originalLine"),
        "root_cause_key": f"github-review-thread:{thread_id}",
        "forced_state": forced_state,
        "needs_human_hint": needs_human_hint,
        "comments": [
            {
                "comment_node_id": _require_string(comment.get("id"), f"{thread_id} comment_node_id"),
                "comment_id": comment.get("databaseId"),
                "body_sha256": hashlib.sha256(str(comment.get("body") or "").encode("utf-8")).hexdigest(),
            }
            for comment in comments
            if isinstance(comment, dict)
        ],
    }


def build_context_artifacts(
    pr_payload: dict[str, Any],
    review_threads_payload: dict[str, Any],
    *,
    repository: str,
    pr_number: str,
    base_sha: str,
    run_id: str,
    event_name: str,
) -> dict[str, Any]:
    require_keys(pr_payload, ["baseRefName", "headRefName", "headRefOid", "files"])
    repo = pr_payload.get("headRepository") or {}
    head_repo = repo.get("nameWithOwner") if isinstance(repo, dict) else None
    files = [
        {"path": _require_string(item.get("path"), "changed file path"), "status": item.get("changeType") or "modified"}
        for item in (pr_payload.get("files") or [])
        if isinstance(item, dict)
    ]
    connection = _review_threads_connection(review_threads_payload)
    if connection.get("pageInfo", {}).get("hasNextPage"):
        raise ValueError("reviewThreads pagination exceeded")
    threads = [
        _build_thread(raw)
        for raw in (connection.get("nodes") or [])
        if isinstance(raw, dict) and not raw.get("isResolved")
    ]
    values = {
        "pr_number": _require_string(pr_number, "pr_number"),
        "base_ref": _require_string(pr_payload.get("baseRefName"), "baseRefName"),
        "base_sha": _require_string(base_sha, "base_sha"),
        "head_ref": _require_string(pr_payload.get("headRefName"), "headRefName"),
        "head_sha": _require_string(pr_payload.get("headRefOid"), "headRefOid"),
        "head_repo": head_repo or _require_string(repository, "repository"),
    }
    return {
        "outputs": values,
        "thread_inventory": {
            "schema_version": THREAD_INVENTORY_SCHEMA,
            "repository": _require_string(repository, "repository"),
            "pr_number": values["pr_number"],
            "base_ref": values["base_ref"],
            "base_sha": values["base_sha"],
            "head_sha": values["head_sha"],
            "threads": threads,
        },
        "review_request": {
            "schema_version": "codex.stage01.review_request.v1",
            "repository": _require_string(repository, "repository"),
            "pr_number": values["pr_number"],
            "base_sha": values["base_sha"],
            "head_sha": values["head_sha"],
            "changed_files": files,
            "axes": ["correctness", "tests", "performance", "domain"],
        },
        "run_state": {
            "schema_version": "codex.stage08.run_state.v1",
            "run_id": _require_string(run_id, "run_id"),
            "event_name": _require_string(event_name, "event_name"),
            "loop_count": 0,
            "max_loops": 1,
        },
    }


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


def build_lifecycle_result(gate_payload: dict[str, Any], inventory_payload: dict[str, Any]) -> dict[str, Any]:
    require_schema_version(gate_payload, RESOLVE_GATE_SCHEMA)
    require_keys(gate_payload, ["status", "thread_ids", "needs_human_thread_ids", "stop_reasons"])
    status = _require_string(gate_payload.get("status"), "status")
    if status not in {"clear", "needs_lifecycle_review", "needs_human"}:
        raise ValueError(f"unknown resolve gate status: {status}")
    threads = validate_thread_inventory(inventory_payload)
    thread_ids = [str(thread["thread_id"]) for thread in threads]
    gate_thread_ids = [str(thread_id) for thread_id in gate_payload.get("thread_ids") or []]
    if sorted(thread_ids) != sorted(gate_thread_ids):
        raise ValueError("lifecycle thread_ids must match resolve gate")
    needs_human = [str(thread_id) for thread_id in gate_payload.get("needs_human_thread_ids") or []]
    deferred = thread_ids if status == "needs_lifecycle_review" else []
    can_continue = status in {"clear", "needs_lifecycle_review"}
    return {
        "schema_version": LIFECYCLE_SCHEMA,
        "stage": "stage00-lifecycle",
        "status": "classified" if status == "needs_lifecycle_review" else status,
        "can_continue": can_continue,
        "thread_count": len(thread_ids),
        "deferred_thread_ids": deferred,
        "needs_human_thread_ids": needs_human,
        "stop_reasons": [str(reason) for reason in gate_payload.get("stop_reasons") or []],
    }
