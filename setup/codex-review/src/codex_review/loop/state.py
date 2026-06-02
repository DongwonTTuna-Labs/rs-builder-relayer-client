"""Loop state stored in sticky comments or artifacts."""
from __future__ import annotations

import hashlib
import json
import re
from datetime import datetime, timezone
from typing import Any

from codex_review.context.diff import parse_unified_diff
from codex_review.errors import ValidationError
from codex_review.github.comments import upsert_sticky_comment
from codex_review.github.markers import parse_marker, render_marker

_WS = re.compile(r"\s+")


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


# --- Oscillation / round-cap detection (case A) ---------------------------------
#
# The loop has no memory across runs (each push re-triggers the whole workflow), so a
# bounded history of recent pushes is persisted in the sticky loop-state comment.
# Detection is similarity/identity based, NOT byte-equality: an AI re-fixing the same
# issue produces a byte-different patch every round, so an exact patch hash alone is
# useless for catching A->B->A oscillation.


def _normalize_line(text: str) -> str:
    return _WS.sub(" ", (text or "").strip())


def _line_fingerprint(text: str) -> str:
    return hashlib.sha256(_normalize_line(text).encode("utf-8")).hexdigest()[:16]


def fingerprint_patch(patch_text: str) -> dict[str, Any]:
    """Whitespace-normalized fingerprints of a patch's changed lines + touched paths.

    Robust to cosmetic drift (whitespace/comment wording/ordering) so two rounds that
    add/remove the *same* lines compare equal even when the raw bytes differ.
    """
    files = parse_unified_diff(patch_text or "")
    added: set[str] = set()
    removed: set[str] = set()
    touched: set[str] = set()
    for f in files:
        path = f.get("new_path") or f.get("old_path")
        if path and path != "/dev/null":
            touched.add(str(path))
        for hunk in f.get("hunks", []):
            for line in hunk.get("lines", []):
                norm = _normalize_line(line.get("text"))
                if not norm:
                    continue
                if line.get("kind") == "add":
                    added.add(_line_fingerprint(norm))
                elif line.get("kind") == "del":
                    removed.add(_line_fingerprint(norm))
    return {"added_line_fp": sorted(added), "removed_line_fp": sorted(removed), "touched_paths": sorted(touched)}


def normalized_finding_keys_from_plan(design_plan: dict[str, Any] | None) -> list[str]:
    """Stable-ish identity of WHAT a fix targets, from the design plan edit sequence."""
    keys: set[str] = set()
    for step in (design_plan or {}).get("edit_sequence", []) or []:
        for fid in step.get("finding_ids", []) or []:
            if fid:
                keys.add(str(fid).strip().lower())
        if step.get("task_id"):
            keys.add(str(step["task_id"]).strip().lower())
    return sorted(keys)


def build_push_entry(round_no: int, push_result: dict[str, Any], patch_text: str, design_plan: dict[str, Any] | None) -> dict[str, Any]:
    # Empty patch_sha256 stays falsy so a missing/empty patch can never be mistaken for
    # an "identical patch repeated" match (sha256("") is otherwise a constant).
    patch_sha256 = hashlib.sha256(patch_text.encode("utf-8")).hexdigest() if patch_text else ""
    return {
        "round": int(round_no),
        "commit_sha": push_result.get("commit_sha"),
        "head_sha": push_result.get("head_sha") or push_result.get("old_head"),
        "patch_sha256": patch_sha256,
        "normalized_finding_keys": normalized_finding_keys_from_plan(design_plan),
        **fingerprint_patch(patch_text),
    }


def append_push_to_loop_state(prior: dict[str, Any] | None, entry: dict[str, Any], window: int) -> dict[str, Any]:
    prior = prior or {}
    recent = list(prior.get("recent_pushes", []) or [])
    recent.append(entry)
    if window and len(recent) > int(window):
        recent = recent[-int(window):]
    return {
        **prior,
        "schema_version": "loop-state.v1",
        "stage": "stage08_reentry",
        "recent_pushes": recent,
        "round_count": int(prior.get("round_count", 0)) + 1,
        "head_sha": entry.get("head_sha") or prior.get("head_sha"),
        "updated_at": datetime.now(timezone.utc).isoformat(),
    }


def _jaccard(a: list[str] | None, b: list[str] | None) -> float:
    sa, sb = set(a or []), set(b or [])
    if not sa or not sb:
        return 0.0
    return len(sa & sb) / len(sa | sb)


def detect_oscillation(prior: dict[str, Any] | None, candidate: dict[str, Any], config: dict[str, Any] | None) -> dict[str, Any]:
    """Decide whether pushing ``candidate`` would continue a non-converging loop.

    Returns {"ok": bool, "status": str, "reason": str}. ok=False blocks the push so the
    loop escalates to an issue + human instead of ping-ponging forever.
    """
    auto = (config or {}).get("autofix", {}) or {}
    max_rounds = int(auto.get("max_rounds", 5))
    pingpong_threshold = int(auto.get("pingpong_threshold", 2))
    revert_threshold = float(auto.get("revert_threshold", 0.8))
    prior = prior or {}
    recent = prior.get("recent_pushes", []) or []
    round_count = int(prior.get("round_count", 0))

    if round_count >= max_rounds:
        return {"ok": False, "status": "max_rounds_reached", "reason": f"autofix rounds {round_count} reached max_rounds {max_rounds}"}
    if candidate.get("patch_sha256") and any(p.get("patch_sha256") == candidate["patch_sha256"] for p in recent):
        return {"ok": False, "status": "oscillation_detected", "reason": "identical merged patch was already pushed in a prior round"}
    for key in candidate.get("normalized_finding_keys", []) or []:
        hits = sum(1 for p in recent if key in (p.get("normalized_finding_keys") or []))
        if hits >= pingpong_threshold:
            return {"ok": False, "status": "oscillation_detected", "reason": f"finding '{key}' was already addressed in {hits} prior round(s) (ping-pong)"}
    cand_added = candidate.get("added_line_fp", []) or []
    cand_paths = set(candidate.get("touched_paths", []) or [])
    for p in recent:
        if cand_paths & set(p.get("touched_paths", []) or []) and _jaccard(cand_added, p.get("removed_line_fp")) >= revert_threshold:
            return {"ok": False, "status": "oscillation_detected", "reason": "patch re-adds lines a prior round removed in the same files (revert loop)"}
    return {"ok": True, "status": "ok", "reason": ""}
