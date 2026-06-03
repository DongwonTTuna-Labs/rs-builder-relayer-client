"""CLI handler: loop commands."""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
from typing import Any

from codex_review.core.artifacts import read_json, read_text, write_json, write_text
from codex_review.core.config import load_config
from codex_review.core.env import read_event_payload
from codex_review.core.errors import CodexReviewError, ValidationError, format_error
from codex_review.core.output import append_step_summary, mask_secret, write_output
from codex_review.cli._helpers import (
    _add_common, _artifact_paths, _default_inspection_evidence, _emit,
    _json_or_default, _maybe_json, _maybe_text, _model_or_fallback,
    _preferred_artifact_paths, _repo_parts_from_context, _safe_path_component,
)


def handle_loop(args: argparse.Namespace) -> tuple[Any, str | None]:
    from codex_review.loop.router import route_after_resolve_gate, route_after_techlead, route_after_design_chief, route_after_push, write_route_outputs
    cmd = args.command
    payload = _maybe_json(args.in_path, {})
    if cmd == "route-after-resolve_gate":
        route = route_after_resolve_gate(payload)
        write_route_outputs(route)
        return route, None
    if cmd == "route-after-techlead":
        route = route_after_techlead(payload)
        write_route_outputs(route)
        return route, None
    if cmd == "route-after-design_chief":
        route = route_after_design_chief(payload)
        write_route_outputs(route)
        return route, None
    if cmd == "route-after-push":
        route = route_after_push(payload)
        write_route_outputs(route)
        return route, None
    if cmd == "summary":
        from codex_review.loop.events import render_event_summary
        return render_event_summary(payload if isinstance(payload, list) else payload.get("events", [])), None
    if cmd == "read-state":
        from codex_review.loop.state import read_loop_state_from_comments
        empty = {"schema_version": "loop-state.v1", "recent_pushes": [], "round_count": 0}
        pr = _json_or_default(args.pr_context, {})
        owner, repo = _repo_parts_from_context(pr)
        pr_number = pr.get("pr_number")
        if not (owner and repo and pr_number and args.token):
            return empty, None
        # Tolerant: a comment-read hiccup must not break bootstrap; degrade to no history.
        try:
            from codex_review.github.comments import list_issue_comments
            comments = list_issue_comments(owner, repo, int(pr_number), args.token)
            state = read_loop_state_from_comments(comments)
        except Exception:
            state = None
        return ({**empty, **state} if state else empty), None
    raise ValueError(f"unknown loop command: {cmd}")


