#!/usr/bin/env python3
"""Resolve and authorize a Forgejo PR review request."""

from __future__ import annotations

import json
import os
import sys
from pathlib import Path
from typing import Any

from forgejo_api import ForgejoClient, require_env, set_output


def login_of(value: Any) -> str:
    if isinstance(value, dict):
        return str(value.get("login") or value.get("username") or "")
    return ""


def event_payload() -> dict[str, Any]:
    path = Path(require_env("GITHUB_EVENT_PATH"))
    return json.loads(path.read_text(encoding="utf-8"))


def command_body(payload: dict[str, Any]) -> str:
    comment = payload.get("comment")
    if isinstance(comment, dict):
        return str(comment.get("body") or "")
    return ""


def requested_pr_number(payload: dict[str, Any]) -> tuple[str | None, str]:
    event_name = os.environ.get("GITHUB_EVENT_NAME", "")
    if event_name == "workflow_dispatch":
        inputs = payload.get("inputs") or {}
        return str(inputs.get("pr_number") or os.environ.get("PR_NUMBER") or ""), "workflow_dispatch"
    if event_name == "pull_request_target":
        pr = payload.get("pull_request") or {}
        return str(pr.get("number") or ""), f"pull_request_target:{payload.get('action') or 'unknown'}"
    if event_name in {"issue_comment", "issues"}:
        body = command_body(payload)
        issue = payload.get("issue") or {}
        if "/codex-review" not in body:
            return None, f"{event_name}:missing-command"
        if not issue.get("pull_request"):
            return None, f"{event_name}:not-pr"
        return str(issue.get("number") or ""), f"{event_name}:/codex-review"
    return None, f"{event_name}:unsupported"


def same_repo(pr: dict[str, Any], repo: str) -> bool:
    head_repo = ((pr.get("head") or {}).get("repo") or {}).get("full_name")
    if not head_repo:
        return False
    return str(head_repo) == repo


def authorize(payload: dict[str, Any], pr: dict[str, Any], repo: str) -> tuple[bool, str]:
    allowed_login = os.environ.get("CODEX_ALLOWED_LOGIN", "DongwonTTuna")
    actor = os.environ.get("GITHUB_ACTOR") or login_of(payload.get("sender"))
    if actor.endswith("[bot]"):
        return False, "bot actor is not allowed to trigger Codex review"
    if actor != allowed_login:
        return False, f"actor {actor!r} is not allowed"
    if pr.get("draft"):
        return False, "draft PR is not reviewed"
    base_ref = str((pr.get("base") or {}).get("ref") or "")
    if base_ref != "main":
        return False, f"base ref {base_ref!r} is not allowed"
    if not same_repo(pr, repo):
        return False, "fork PR is not reviewed"
    author = login_of(pr.get("user"))
    if author and author != allowed_login:
        return False, f"PR author {author!r} is not allowed"
    return True, "authorized"


def main() -> int:
    client = ForgejoClient.from_env()
    payload = event_payload()
    pr_number, trigger = requested_pr_number(payload)
    if not pr_number:
        set_output("allowed", "false")
        set_output("reason", trigger)
        return 0

    pr = client.request("GET", client.repo_path(f"pulls/{pr_number}"))
    if not isinstance(pr, dict):
        raise SystemExit("unexpected Forgejo PR response")
    allowed, reason = authorize(payload, pr, client.repo)
    head = pr.get("head") or {}
    base = pr.get("base") or {}
    set_output("allowed", "true" if allowed else "false")
    set_output("reason", reason)
    set_output("pr_number", str(pr_number))
    set_output("head_sha", str(head.get("sha") or ""))
    set_output("base_sha", str(base.get("sha") or ""))
    set_output("base_ref", str(base.get("ref") or "main"))
    set_output("scripts_ref", str(base.get("sha") or base.get("ref") or "main"))
    set_output("trigger", trigger)
    if not allowed:
        print(f"Codex review skipped: {reason}")
    else:
        print(f"Codex review authorized for PR #{pr_number}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
