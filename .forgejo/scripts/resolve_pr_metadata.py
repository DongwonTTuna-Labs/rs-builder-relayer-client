#!/usr/bin/env python3
"""Resolve and authorize a Forgejo PR review request."""

from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path
from typing import Any

from forgejo_api import ForgejoClient, require_env, set_output

WIP_PREFIXES = ("WIP:", "[WIP]")
TRUSTED_ACTION_SURFACE_PATHS = (".forgejo/workflows", ".forgejo/scripts")


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
    if os.environ.get("GITHUB_EVENT_NAME") == "issues":
        issue = payload.get("issue")
        if isinstance(issue, dict):
            return str(issue.get("body") or "")
    return ""


def has_review_command(body: str) -> bool:
    return any(line.strip() == "/codex-review" for line in body.splitlines())


def has_wip_prefix(title: str) -> bool:
    normalized = title.strip().casefold()
    return any(normalized.startswith(prefix.casefold()) for prefix in WIP_PREFIXES)


def actor_name(payload: dict[str, Any]) -> str:
    return os.environ.get("GITHUB_ACTOR") or login_of(payload.get("sender"))


def actor_allowed(payload: dict[str, Any]) -> tuple[bool, str]:
    allowed_login = os.environ.get("CODEX_ALLOWED_LOGIN", "DongwonTTuna")
    actor = actor_name(payload)
    if actor.endswith("[bot]"):
        return False, "bot actor is not allowed to trigger Codex review"
    if actor != allowed_login:
        return False, f"actor {actor!r} is not allowed"
    return True, "authorized"


def is_ready_for_review_edit(payload: dict[str, Any], pr: dict[str, Any]) -> bool:
    if payload.get("action") != "edited" or pr.get("draft"):
        return False
    changes = payload.get("changes") or {}
    title_change = changes.get("title") if isinstance(changes, dict) else None
    old_title = title_change.get("from") if isinstance(title_change, dict) else None
    current_title = str(pr.get("title") or "")
    return isinstance(old_title, str) and has_wip_prefix(old_title) and not has_wip_prefix(current_title)


def requested_pr_number(payload: dict[str, Any]) -> tuple[str | None, str]:
    event_name = os.environ.get("GITHUB_EVENT_NAME", "")
    if event_name == "workflow_dispatch":
        inputs = payload.get("inputs") or {}
        pr_number = str(inputs.get("pr_number") or os.environ.get("PR_NUMBER") or "").strip()
        if not pr_number:
            return None, "workflow_dispatch:missing-pr-number"
        return pr_number, "workflow_dispatch"
    if event_name == "pull_request_target":
        pr = payload.get("pull_request") or {}
        return str(pr.get("number") or ""), f"pull_request_target:{payload.get('action') or 'unknown'}"
    if event_name in {"issue_comment", "issues"}:
        body = command_body(payload)
        issue = payload.get("issue") or {}
        if not has_review_command(body):
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


def run_git(args: list[str], *, with_auth: bool = False) -> subprocess.CompletedProcess[str]:
    command = ["git"]
    if with_auth:
        command.extend(
            [
                "-c",
                'credential.helper=!f() { echo username=x-access-token; echo "password=$GIT_AUTH_TOKEN"; }; f',
                "-c",
                "credential.useHttpPath=true",
            ]
        )
    command.extend(args)
    return subprocess.run(command, check=False, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)


def current_default_branch(client: ForgejoClient) -> str:
    branch = os.environ.get("CODEX_DEFAULT_BRANCH", "").strip()
    if branch:
        return branch
    repo = client.request("GET", f"repos/{client.repo}")
    if isinstance(repo, dict):
        branch = str(repo.get("default_branch") or "").strip()
    if not branch:
        raise SystemExit("could not resolve repository default branch")
    return branch


def current_default_sha() -> str:
    sha = os.environ.get("CODEX_DEFAULT_SHA", "").strip()
    if sha:
        return sha
    result = run_git(["rev-parse", "HEAD"])
    if result.returncode != 0:
        raise SystemExit("could not resolve trusted default branch SHA")
    return result.stdout.strip()


def trusted_action_surface_matches_default(head_sha: str) -> tuple[bool, str]:
    if not head_sha:
        return False, "PR head SHA is missing"
    if not os.environ.get("GIT_AUTH_TOKEN", "").strip():
        return False, "GIT_AUTH_TOKEN is required for trusted action surface check"
    fetch = run_git(["fetch", "--depth=1", "origin", head_sha], with_auth=True)
    if fetch.returncode != 0:
        return False, "could not fetch PR head for trusted action surface check"
    diff = run_git(["diff", "--quiet", "HEAD", head_sha, "--", *TRUSTED_ACTION_SURFACE_PATHS])
    if diff.returncode == 0:
        return True, "trusted action surface matches default branch"
    if diff.returncode == 1:
        return False, "trusted action surface differs from default branch"
    return False, "could not verify trusted action surface"


def authorize(
    payload: dict[str, Any],
    pr: dict[str, Any],
    repo: str,
    *,
    verify_trusted_surface: bool = False,
) -> tuple[bool, str]:
    actor_ok, reason = actor_allowed(payload)
    if not actor_ok:
        return False, reason
    allowed_login = os.environ.get("CODEX_ALLOWED_LOGIN", "DongwonTTuna")
    if pr.get("draft"):
        return False, "draft PR is not reviewed"
    if os.environ.get("GITHUB_EVENT_NAME") == "pull_request_target" and payload.get("action") == "edited":
        if not is_ready_for_review_edit(payload, pr):
            return False, "edited PR event is not a ready-for-review transition"
    if has_wip_prefix(str(pr.get("title") or "")):
        return False, "WIP title PR is not reviewed"
    if not same_repo(pr, repo):
        return False, "fork PR is not reviewed"
    author = login_of(pr.get("user"))
    if not author:
        return False, "PR author is missing"
    if author != allowed_login:
        return False, f"PR author {author!r} is not allowed"
    if verify_trusted_surface:
        trusted, trusted_reason = trusted_action_surface_matches_default(str((pr.get("head") or {}).get("sha") or ""))
        if not trusted:
            return False, trusted_reason
    return True, "authorized"


def write_denied_outputs(reason: str, pr_number: str = "") -> None:
    set_output("allowed", "false")
    set_output("reason", reason)
    set_output("pr_number", pr_number)
    set_output("head_sha", "")
    set_output("base_sha", "")
    set_output("base_ref", "")
    set_output("scripts_ref", "")
    set_output("trigger", "")


def main() -> int:
    client = ForgejoClient.from_env()
    payload = event_payload()
    pr_number, trigger = requested_pr_number(payload)
    if not pr_number:
        write_denied_outputs(trigger)
        return 0

    actor_ok, actor_reason = actor_allowed(payload)
    if not actor_ok:
        write_denied_outputs(actor_reason, pr_number)
        print(f"Codex review skipped: {actor_reason}")
        return 0

    pr = client.request("GET", client.repo_path(f"pulls/{pr_number}"))
    if not isinstance(pr, dict):
        raise SystemExit("unexpected Forgejo PR response")
    default_branch = current_default_branch(client)
    default_sha = current_default_sha()
    allowed, reason = authorize(payload, pr, client.repo, verify_trusted_surface=True)
    head = pr.get("head") or {}
    base = pr.get("base") or {}
    set_output("allowed", "true" if allowed else "false")
    set_output("reason", reason)
    set_output("pr_number", str(pr_number))
    set_output("head_sha", str(head.get("sha") or ""))
    set_output("base_sha", str(base.get("sha") or ""))
    set_output("base_ref", str(base.get("ref") or default_branch))
    set_output("scripts_ref", default_sha)
    set_output("trigger", trigger)
    if not allowed:
        print(f"Codex review skipped: {reason}")
    else:
        print(f"Codex review authorized for PR #{pr_number}")
        print(f"Trusted action surface matches default branch {default_branch}@{default_sha}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
