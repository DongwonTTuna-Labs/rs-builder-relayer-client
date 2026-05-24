#!/usr/bin/env python3
"""Collect Forgejo PR metadata for Codex review prompts."""

from __future__ import annotations

import json
import os
import re
import sys
from pathlib import Path
from typing import Any

from forgejo_api import ForgejoApiError, ForgejoClient, require_env, warn

INLINE_MARKER = "<!-- forgejo-codex-inline"
STICKY_MARKER = "<!-- forgejo-codex-review-sticky -->"
PROMPT_DIFF_LIMIT = 120000
DIFF_FETCH_LIMIT = 2_000_000
MAX_REVIEW_COMMENT_REVIEW_SCAN = 80


def comment_login(comment: dict[str, Any]) -> str:
    user = comment.get("user") or comment.get("poster") or {}
    if isinstance(user, dict):
        return str(user.get("login") or user.get("username") or "")
    return ""


def bot_login() -> str:
    return os.environ.get("FORGEJO_BOT_LOGIN", "").strip()


def bot_logins() -> set[str]:
    login = bot_login()
    return {login} if login else set()


def is_bot_comment(comment: dict[str, Any]) -> bool:
    return comment_login(comment) in bot_logins()


def configure_bot_login(client: ForgejoClient) -> str:
    configured = bot_login()
    if configured:
        return configured
    login = client.authenticated_login()
    os.environ["FORGEJO_BOT_LOGIN"] = login
    return login


def parse_changed_right_lines(diff: str) -> dict[str, set[int]]:
    changed: dict[str, set[int]] = {}
    current_file: str | None = None
    new_line = 0
    for raw in diff.splitlines():
        if raw.startswith("+++ b/"):
            current_file = raw[6:]
            changed.setdefault(current_file, set())
            continue
        if raw.startswith("+++ /dev/null"):
            current_file = None
            continue
        match = re.match(r"@@ -\d+(?:,\d+)? \+(\d+)(?:,\d+)? @@", raw)
        if match:
            new_line = int(match.group(1))
            continue
        if current_file is None:
            continue
        if raw.startswith("\\ "):
            continue
        if raw.startswith("+") and not raw.startswith("+++ b/") and raw != "+++ /dev/null":
            changed[current_file].add(new_line)
            new_line += 1
        elif raw.startswith("-") and not raw.startswith("---"):
            continue
        else:
            new_line += 1
    return changed


def fetch_review_comments(client: ForgejoClient, pr_number: str) -> list[dict[str, Any]]:
    comments: list[dict[str, Any]] = []
    try:
        reviews = client.paginated(client.repo_path(f"pulls/{pr_number}/reviews"))
    except ForgejoApiError as exc:
        warn(f"could not list pull reviews; inline context will be empty: {exc}")
        return []
    if len(reviews) > MAX_REVIEW_COMMENT_REVIEW_SCAN:
        warn(
            "review comment scan capped at latest "
            f"{MAX_REVIEW_COMMENT_REVIEW_SCAN} reviews because Forgejo REST exposes comments per review"
        )
    for review in reviews[-MAX_REVIEW_COMMENT_REVIEW_SCAN:]:
        review_id = review.get("id")
        if not review_id:
            continue
        try:
            review_comments = client.paginated(
                client.repo_path(f"pulls/{pr_number}/reviews/{review_id}/comments"),
            )
        except ForgejoApiError as exc:
            warn(f"could not list pull review comments for review {review_id}: {exc}")
            continue
        comments.extend(review_comments)
    return comments


def normalize_file(row: dict[str, Any], changed_lines: dict[str, set[int]]) -> dict[str, Any]:
    path = str(row.get("filename") or row.get("name") or row.get("path") or "")
    return {
        "filename": path,
        "status": row.get("status") or "",
        "additions": row.get("additions") or 0,
        "deletions": row.get("deletions") or 0,
        "changes": row.get("changes") or 0,
        "changed_right_lines": sorted(changed_lines.get(path, set())),
    }


def main() -> int:
    client = ForgejoClient.from_env()
    pr_number = require_env("PR_NUMBER")
    runner_temp = Path(require_env("RUNNER_TEMP"))
    runner_temp.mkdir(parents=True, exist_ok=True)
    head_sha_expected = os.environ.get("HEAD_SHA", "").strip()
    base_sha_expected = os.environ.get("BASE_SHA", "").strip()

    pr = client.request("GET", client.repo_path(f"pulls/{pr_number}"))
    if not isinstance(pr, dict):
        raise SystemExit("unexpected Forgejo PR response")

    head = pr.get("head") or {}
    base = pr.get("base") or {}
    head_sha = str(head.get("sha") or "")
    base_sha = str(base.get("sha") or "")
    if head_sha_expected and head_sha and head_sha != head_sha_expected:
        raise SystemExit("PR head SHA changed while preparing review context")
    if base_sha_expected and base_sha and base_sha != base_sha_expected:
        raise SystemExit("PR base SHA changed while preparing review context")

    configure_bot_login(client)

    diff_text, diff_truncated_for_scan = client.request_text_limited(
        client.repo_path(f"pulls/{pr_number}.diff"),
        DIFF_FETCH_LIMIT,
    )
    if diff_truncated_for_scan:
        raise SystemExit("PR diff exceeds Forgejo review scan limit; refusing partial review context")
    prompt_diff = diff_text[:PROMPT_DIFF_LIMIT]
    changed_lines = parse_changed_right_lines(diff_text)

    files = client.paginated(client.repo_path(f"pulls/{pr_number}/files"))
    issue_comments = client.paginated(client.repo_path(f"issues/{pr_number}/comments"))
    review_comments = fetch_review_comments(client, pr_number)

    existing_inline = []
    for comment in review_comments:
        body = str(comment.get("body") or "")
        if INLINE_MARKER not in body or not is_bot_comment(comment):
            continue
        existing_inline.append(
            {
                "id": comment.get("id"),
                "path": comment.get("path") or "",
                "line": comment.get("position") or comment.get("line") or 0,
                "body": body,
                "html_url": comment.get("html_url") or "",
                "updated_at": comment.get("updated_at") or "",
            }
        )

    context = {
        "provider": "forgejo",
        "repository": client.repo,
        "pr_number": int(pr_number),
        "title": pr.get("title") or "",
        "body": pr.get("body") or "",
        "base_ref": base.get("ref") or os.environ.get("GITHUB_BASE_REF", ""),
        "base_sha": base_sha or base_sha_expected,
        "head_ref": head.get("ref") or os.environ.get("GITHUB_HEAD_REF", ""),
        "head_sha": head_sha or head_sha_expected,
        "diff": prompt_diff,
        "diff_truncated": len(diff_text) > PROMPT_DIFF_LIMIT,
        "changed_files": [normalize_file(row, changed_lines) for row in files],
        "existing_inline_comments": existing_inline,
        "existing_sticky_comments": [
            {
                "id": c.get("id"),
                "body": c.get("body") or "",
                "updated_at": c.get("updated_at") or "",
            }
            for c in issue_comments
            if STICKY_MARKER in str(c.get("body") or "") and is_bot_comment(c)
        ],
    }
    out = runner_temp / "pr-context.json"
    out.write_text(json.dumps(context, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"wrote Forgejo PR context: {out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
