#!/usr/bin/env python3
"""Publish Forgejo Codex review inline and sticky comments."""

from __future__ import annotations

import hashlib
import json
import os
import re
import sys
import textwrap
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from forgejo_api import ForgejoApiError, ForgejoClient, require_env, warn
from prepare_pr_context import INLINE_MARKER, STICKY_MARKER, fetch_review_comments, is_bot_comment

DISPLAY_NAME = "Codex Reviewer for DongwonTTuna"
RESOLVED_MARKER = "<!-- forgejo-codex-inline-resolved"
MAX_INLINE_COMMENTS = 30


def require_json(path: Path) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError as exc:
        raise SystemExit(f"required review artifact is missing: {path}") from exc
    except json.JSONDecodeError as exc:
        raise SystemExit(f"required review artifact is invalid JSON: {path}: {exc}") from exc


def finding_key(finding: dict[str, Any]) -> str:
    material = "\n".join(
        [
            str(finding.get("agent") or ""),
            str(finding.get("type") or ""),
            str(finding.get("file") or ""),
            str(finding.get("line") or ""),
            "cross-cutting" if finding.get("cross_cutting") else "inline",
        ]
    )
    return hashlib.sha256(material.encode("utf-8")).hexdigest()[:16]


def marker_for(key: str, status: str = "active") -> str:
    return f'{INLINE_MARKER} key="{key}" status="{status}" -->'


def extract_marker(body: str) -> tuple[str | None, str | None]:
    match = re.search(
        r"<!--\s*forgejo-codex-inline\s+key=\"([0-9a-f]+)\"\s+status=\"([a-z-]+)\"\s*-->",
        body,
    )
    if not match:
        return None, None
    return match.group(1), match.group(2)


def render_inline_body(finding: dict[str, Any], key: str) -> str:
    title = str(finding.get("title") or "").strip()
    reason = str(finding.get("reason") or "").strip()
    finding_type = str(finding.get("type") or "SUGGEST").strip()
    finding_id = str(finding.get("id") or "").strip()
    agent = str(finding.get("agent") or "").strip()
    return "\n".join(
        [
            marker_for(key),
            f"**[{finding_type}] {title}**",
            "",
            reason,
            "",
            f"_{DISPLAY_NAME}: `{agent}` / `{finding_id}`_",
        ]
    ).strip()


def render_sticky(
    pr_number: str,
    allowed: list[dict[str, Any]],
    posted: int,
    skipped_existing: int,
    resolved: int,
    axes_status: dict[str, Any],
    judgment: dict[str, Any] | None,
) -> str:
    counts: dict[str, int] = {}
    for finding in allowed:
        counts[str(finding.get("type") or "UNKNOWN")] = counts.get(str(finding.get("type") or "UNKNOWN"), 0) + 1
    count_text = ", ".join(f"{name}: {count}" for name, count in sorted(counts.items())) or "none"
    missing_axes = ", ".join(axes_status.get("missing") or []) or "none"
    status = "NEEDS_WORK" if allowed else "LGTM"
    headline = "리뷰 코멘트가 있습니다." if allowed else "게시할 신규 리뷰 코멘트가 없습니다."
    if isinstance(judgment, dict) and judgment.get("status"):
        status = str(judgment.get("status"))
        headline = str(judgment.get("headline") or headline)
    now = datetime.now(timezone.utc).strftime("%Y-%m-%d %H:%M:%SZ")
    cross_cutting = [
        finding
        for finding in allowed
        if finding.get("cross_cutting") or not finding.get("file") or not finding.get("line")
    ]
    cross_cutting_lines = []
    for finding in cross_cutting[:10]:
        finding_type = str(finding.get("type") or "SUGGEST")
        title = str(finding.get("title") or "").strip()
        agent = str(finding.get("agent") or "").strip()
        cross_cutting_lines.append(f"- **[{finding_type}]** {title} (`{agent}`)")
    if len(cross_cutting) > 10:
        cross_cutting_lines.append(f"- ...and {len(cross_cutting) - 10} more cross-cutting findings")
    cross_cutting_text = "\n".join(cross_cutting_lines) or "- none"
    return textwrap.dedent(
        f"""\
        {STICKY_MARKER}
        ## {DISPLAY_NAME}

        Updated: {now}
        PR: #{pr_number}
        Status: **{status}**

        {headline}

        - Allowed findings: {len(allowed)} ({count_text})
        - Inline comments posted: {posted}
        - Existing inline comments kept: {skipped_existing}
        - Bot-managed resolved comments: {resolved}
        - Missing axes: {missing_axes}

        Cross-cutting findings:
        {cross_cutting_text}

        재리뷰는 `/codex-review` 코멘트로 요청하세요.
        """
    ).strip()


def existing_by_key(comments: list[dict[str, Any]]) -> dict[str, dict[str, Any]]:
    result: dict[str, dict[str, Any]] = {}
    for comment in comments:
        if not is_bot_comment(comment):
            continue
        key, status = extract_marker(str(comment.get("body") or ""))
        if key and status == "active":
            result.setdefault(key, comment)
    return result


def delete_review_comment(client: ForgejoClient, pr_number: str, comment: dict[str, Any]) -> bool:
    comment_id = comment.get("id")
    review_id = comment.get("pull_request_review_id")
    if not comment_id or not review_id:
        warn(f"could not replace review comment without review/comment id: {comment_id}")
        return False
    try:
        client.request(
            "DELETE",
            client.repo_path(f"pulls/{pr_number}/reviews/{review_id}/comments/{comment_id}"),
        )
        return True
    except ForgejoApiError as exc:
        warn(f"could not delete stale review comment {comment_id}: {exc}")
        return False


def post_inline_comments(
    client: ForgejoClient,
    pr_number: str,
    head_sha: str,
    allowed: list[dict[str, Any]],
    existing: dict[str, dict[str, Any]],
) -> tuple[int, int]:
    new_comments: list[dict[str, Any]] = []
    skipped_existing = 0
    for finding in allowed:
        path = finding.get("file")
        line = finding.get("line")
        if not path or not line or finding.get("cross_cutting"):
            continue
        key = finding_key(finding)
        body = render_inline_body(finding, key)
        if key in existing:
            if str(existing[key].get("body") or "") == body:
                skipped_existing += 1
                continue
            if not delete_review_comment(client, pr_number, existing[key]):
                skipped_existing += 1
                continue
        new_comments.append(
            {
                "path": str(path),
                "body": body,
                "new_position": int(line),
                "old_position": 0,
            }
        )
    if not new_comments:
        return 0, skipped_existing
    posted = 0
    for offset in range(0, len(new_comments), MAX_INLINE_COMMENTS):
        batch = new_comments[offset : offset + MAX_INLINE_COMMENTS]
        client.request(
            "POST",
            client.repo_path(f"pulls/{pr_number}/reviews"),
            {
                "event": "COMMENT",
                "commit_id": head_sha,
                "body": f"{DISPLAY_NAME} automated review",
                "comments": batch,
            },
        )
        posted += len(batch)
    return posted, skipped_existing


def reply_resolved(
    client: ForgejoClient,
    pr_number: str,
    comments: list[dict[str, Any]],
    current_keys: set[str],
) -> int:
    resolved = 0
    resolved_keys = {
        match.group(1)
        for comment in comments
        for match in re.finditer(
            r"<!--\s*forgejo-codex-inline-resolved\s+key=\"([0-9a-f]+)\"\s*-->",
            str(comment.get("body") or ""),
        )
        if is_bot_comment(comment)
    }
    for comment in comments:
        if not is_bot_comment(comment):
            continue
        body = str(comment.get("body") or "")
        key, status = extract_marker(body)
        comment_id = comment.get("id")
        if not key or status != "active" or key in current_keys or not comment_id:
            continue
        if key in resolved_keys:
            continue
        reply = (
            f'{RESOLVED_MARKER} key="{key}" -->\n'
            f"{DISPLAY_NAME}: 현재 PR head 기준으로 더 이상 게시 대상 finding에 포함되지 않아 "
            "bot-managed resolved 상태로 표시합니다."
        )
        try:
            client.request(
                "POST",
                client.repo_path(f"pulls/{pr_number}/comments/{comment_id}/replies"),
                {"body": reply},
            )
            resolved += 1
        except ForgejoApiError as exc:
            warn(f"could not reply with resolved marker for review comment {comment_id}: {exc}")
    return resolved


def upsert_sticky(client: ForgejoClient, pr_number: str, body: str) -> None:
    comments = client.paginated(client.repo_path(f"issues/{pr_number}/comments"))
    existing_id = None
    for comment in comments:
        if STICKY_MARKER in str(comment.get("body") or "") and is_bot_comment(comment):
            existing_id = comment.get("id")
            break
    if existing_id:
        client.request(
            "PATCH",
            client.repo_path(f"issues/comments/{existing_id}"),
            {"body": body},
        )
    else:
        client.request(
            "POST",
            client.repo_path(f"issues/{pr_number}/comments"),
            {"body": body},
        )


def main() -> int:
    client = ForgejoClient.from_env()
    pr_number = require_env("PR_NUMBER")
    head_sha = require_env("HEAD_SHA")
    art_dir = Path(os.environ.get("ART_DIR", "artifacts"))
    allowed = require_json(art_dir / "allowed.json")
    axes_status = require_json(art_dir / "axes_status.json")
    decisions = require_json(art_dir / "decisions.json")
    judgment = decisions.get("judgment") if isinstance(decisions, dict) else None
    if not isinstance(allowed, list):
        allowed = []

    review_comments = fetch_review_comments(client, pr_number)
    existing = existing_by_key(review_comments)
    current_keys = {finding_key(f) for f in allowed}
    resolved = reply_resolved(client, pr_number, review_comments, current_keys)
    posted, skipped_existing = post_inline_comments(client, pr_number, head_sha, allowed, existing)
    sticky = render_sticky(
        pr_number,
        allowed,
        posted,
        skipped_existing,
        resolved,
        axes_status if isinstance(axes_status, dict) else {},
        judgment if isinstance(judgment, dict) else None,
    )
    upsert_sticky(client, pr_number, sticky)
    print(
        f"Forgejo review posted: inline={posted}, kept={skipped_existing}, resolved={resolved}, allowed={len(allowed)}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
