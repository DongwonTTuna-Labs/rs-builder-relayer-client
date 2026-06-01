"""Record next-run reentry state."""
from __future__ import annotations
from pathlib import Path
from typing import Any

from codex_review.artifacts import write_json
from codex_review.errors import ValidationError
from codex_review.github.app_token import assert_installation_token_for_repo, permissions_for_write_mode
from codex_review.loop.state import build_loop_state, write_loop_state_comment


def build_reentry_record(push_result: dict[str, Any], loop_state: dict[str, Any], artifacts: dict[str, Any]) -> dict[str, Any]:
    pushed = bool(push_result.get("pushed"))
    state = loop_state or build_loop_state(
        "stage08_reentry",
        {"next_entry": "stage00_on_synchronize" if pushed else "none", "pushed": pushed},
        push_result.get("commit_sha") or "",
        artifacts or {"push_result": push_result},
    )
    return {
        "schema_version": "stage08-loop-reentry.v1",
        "pushed": pushed,
        "commit_sha": push_result.get("commit_sha"),
        "next_entry": "stage00_on_synchronize" if pushed else "none",
        "loop_state": state,
        "artifacts": artifacts,
        "persisted": False,
    }


def write_reentry_artifact(record: dict[str, Any], out_path: str | Path) -> Path:
    return write_json(out_path, record, "stage08-loop-reentry.v1")


def persist_reentry_loop_state(record: dict[str, Any], pr_context: dict[str, Any], token: str | None) -> dict[str, Any]:
    if not record.get("pushed"):
        return {**record, "persisted": False, "persist_reason": "no push occurred"}
    missing = [key for key in ["owner", "repo", "pr_number"] if not pr_context.get(key)]
    if missing:
        raise ValidationError(f"stage08 loop-state persistence requires pr_context fields: {', '.join(missing)}")
    assert_installation_token_for_repo(token, pr_context["owner"], pr_context["repo"], permissions_for_write_mode("stage08"))
    state = record.get("loop_state") or build_loop_state("stage08_reentry", {"next_entry": record.get("next_entry")}, record.get("commit_sha") or "", record.get("artifacts", {}))
    result = write_loop_state_comment(pr_context["owner"], pr_context["repo"], int(pr_context["pr_number"]), state, token)
    out = dict(record)
    out["loop_state"] = state
    out["persisted"] = True
    out["persist_result"] = result
    return out


# Backward-compatible helper retained for older imports.
def update_loop_state_after_push(record: dict[str, Any], token: str | None = None) -> dict[str, Any]:
    return build_loop_state("stage08_reentry", {"next_entry": record.get("next_entry")}, record.get("commit_sha") or "", record.get("artifacts", {}))
