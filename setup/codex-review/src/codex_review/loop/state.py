"""Loop state stored in sticky comments or artifacts."""
from __future__ import annotations

import hashlib
import json
from datetime import datetime, timezone
from typing import Any

from codex_review.errors import ValidationError
from codex_review.github.comments import upsert_sticky_comment
from codex_review.github.markers import parse_marker, render_marker


def read_loop_state_from_comments(comments: list[dict[str, Any]]) -> dict[str, Any] | None:
    states=[]
    for c in comments or []:
        payload=parse_marker(c.get("body", ""), "codex-review:loop-state")
        if payload is not None and not payload.get("_invalid"):
            states.append((c.get("updated_at") or c.get("created_at") or "", payload))
    return sorted(states, key=lambda x:x[0])[-1][1] if states else None


def build_loop_state(stage: str, decision: dict[str, Any], head_sha: str, artifacts: dict[str, Any]) -> dict[str, Any]:
    material=json.dumps(artifacts or {}, sort_keys=True, default=str).encode("utf-8")
    return {"schema_version": "loop-state.v1", "stage": stage, "decision": decision, "head_sha": head_sha, "artifact_hash": hashlib.sha256(material).hexdigest(), "updated_at": datetime.now(timezone.utc).isoformat()}


def validate_loop_state(state: dict[str, Any], current_pr: dict[str, Any]) -> None:
    current=((current_pr.get("head") or {}).get("sha") or current_pr.get("head_sha"))
    if state.get("head_sha") and current and state["head_sha"] != current:
        raise ValidationError("loop state head_sha does not match current PR head")


def render_loop_state_marker(state: dict[str, Any]) -> str:
    return render_marker("codex-review:loop-state", state)


def write_loop_state_comment(owner: str, repo: str, pr_number: int, state: dict[str, Any], token: str | None) -> dict[str, Any]:
    body=f"Codex review loop state updated.\n\n{render_loop_state_marker(state)}"
    return upsert_sticky_comment(owner, repo, pr_number, "codex-review:loop-state", body, token)
