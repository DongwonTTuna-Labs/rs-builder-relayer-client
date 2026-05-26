#!/usr/bin/env python3
"""Publish Forgejo Codex review inline and sticky comments."""

from __future__ import annotations

import hashlib
import html
import json
import os
import re
import sys
import textwrap
from concurrent.futures import ThreadPoolExecutor, as_completed
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from forgejo_api import ForgejoApiError, ForgejoClient, require_env, warn
from prepare_pr_context import (
    INLINE_MARKER,
    STICKY_MARKER,
    configure_bot_login_from_token,
    is_bot_comment,
)
from codex_redaction import redact  # noqa: E402
from codex_redaction import find_unredacted_secret_risks  # noqa: E402

DISPLAY_NAME = "Codex Reviewer for DongwonTTuna"
RESOLVED_MARKER = "<!-- forgejo-codex-inline-resolved"
RESOLVED_INLINE_MARKER_RE = re.compile(
    r"<!--\s*(?:forgejo-codex-inline-resolved\s+key=\"([0-9a-f]+)\"|forgejo-codex-inline\s+key=\"([0-9a-f]+)\"\s+status=\"resolved\")"
)
SUPERSEDED_STICKY_MARKER = "<!-- forgejo-codex-review-sticky-superseded -->"
MAX_INLINE_COMMENTS = 30
INLINE_REQUERY_PAGE_LIMIT = 10
MAX_RESOLVE_COMMENTS = 50
PUBLIC_SECRET_PLACEHOLDER = "<redacted-secret>"
MARKDOWN_ESCAPE_RE = re.compile(r"([\\`*_{}\[\]()#+\-.!|>])")


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


def public_text(value: Any) -> str:
    text = redact(str(value or "").strip())
    if find_unredacted_secret_risks(text):
        return PUBLIC_SECRET_PLACEHOLDER
    return text


def public_markdown_text(value: Any) -> str:
    text = public_text(value)
    text = text.replace(r"\`", "")
    text = text.replace("`", "")
    secret_placeholder = "\0SECRETPLACEHOLDER\0"
    text = html.escape(text.replace(PUBLIC_SECRET_PLACEHOLDER, secret_placeholder), quote=False)
    escaped = MARKDOWN_ESCAPE_RE.sub(r"\\\1", text)
    return escaped.replace(secret_placeholder, PUBLIC_SECRET_PLACEHOLDER)


def public_code_text(value: Any) -> str:
    return public_markdown_text(value).replace("\r", " ").replace("\n", " ")


def render_inline_body(finding: dict[str, Any], key: str) -> str:
    title = public_markdown_text(finding.get("title"))
    reason = public_markdown_text(finding.get("reason"))
    finding_type = public_markdown_text(finding.get("type") or "SUGGEST")
    finding_id = public_code_text(finding.get("id"))
    agent = public_code_text(finding.get("agent"))
    return "\n".join(
        [
            marker_for(key),
            f"**[{finding_type}] {title}**",
            "",
            reason,
            "",
            f"_{DISPLAY_NAME}: {agent} / {finding_id}_",
        ]
    ).strip()


def render_resolved_body(key: str) -> str:
    return "\n".join(
        [
            marker_for(key, "resolved"),
            f"_{DISPLAY_NAME}: 현재 PR head 기준으로 더 이상 게시 대상 finding에 포함되지 않아 "
            "bot-managed resolved 상태로 표시합니다._",
        ]
    ).strip()


def render_sticky(
    pr_number: str,
    allowed: list[dict[str, Any]],
    posted: int,
    skipped_existing: int,
    resolved: int,
    skipped_inline: int,
    axes_status: dict[str, Any],
    judgment: dict[str, Any] | None,
    now: datetime | None = None,
) -> str:
    counts: dict[str, int] = {}
    for finding in allowed:
        counts[str(finding.get("type") or "UNKNOWN")] = counts.get(str(finding.get("type") or "UNKNOWN"), 0) + 1
    count_text = ", ".join(f"{name}: {count}" for name, count in sorted(counts.items())) or "none"
    missing_axes = ", ".join(axes_status.get("missing") or []) or "none"
    has_must = any(str(finding.get("type") or "").upper() == "MUST" for finding in allowed)
    status = "NEEDS_WORK" if has_must else ("LGTM_WITH_COMMENTS" if allowed else "LGTM")
    headline = "필수 수정 리뷰 코멘트가 있습니다." if has_must else (
        "선택 검토 코멘트가 있습니다." if allowed else "게시할 신규 리뷰 코멘트가 없습니다."
    )
    if isinstance(judgment, dict) and judgment.get("status"):
        candidate_status = str(judgment.get("status"))
        status = "NEEDS_WORK" if has_must else ("LGTM_WITH_COMMENTS" if allowed else candidate_status)
        if not allowed:
            headline = public_markdown_text(judgment.get("headline") or headline)
    now_text = (now or datetime.now(timezone.utc)).strftime("%Y-%m-%d %H:%M:%SZ")
    cross_cutting = [
        finding
        for finding in allowed
        if finding.get("cross_cutting") or not finding.get("file") or not finding.get("line")
    ]
    cross_cutting_lines = []
    for finding in cross_cutting[:10]:
        finding_type = public_markdown_text(finding.get("type") or "SUGGEST")
        title = public_markdown_text(finding.get("title"))
        agent = public_code_text(finding.get("agent"))
        reason = " ".join(public_markdown_text(finding.get("reason")).split())
        if len(reason) > 260:
            reason = reason[:257].rstrip() + "..."
        suffix = f": {reason}" if reason else ""
        cross_cutting_lines.append(f"- **[{finding_type}]** {title} ({agent}){suffix}")
    if len(cross_cutting) > 10:
        cross_cutting_lines.append(f"- ...and {len(cross_cutting) - 10} more cross-cutting findings")
    cross_cutting_text = "\n".join(cross_cutting_lines) or "- none"
    skipped_inline_lines = []
    for finding in allowed:
        reason = finding.get("_inline_skip_reason")
        if not reason:
            continue
        finding_type = public_markdown_text(finding.get("type") or "SUGGEST")
        title = public_markdown_text(finding.get("title"))
        agent = public_code_text(finding.get("agent"))
        location = f"{public_code_text(finding.get('file'))}:{public_code_text(finding.get('line'))}"
        skipped_inline_lines.append(f"- **[{finding_type}]** {title} ({agent}, {location}, {reason})")
    skipped_inline_text = "\n".join(skipped_inline_lines[:10]) or "- none"
    if len(skipped_inline_lines) > 10:
        skipped_inline_text += f"\n- ...and {len(skipped_inline_lines) - 10} more skipped inline findings"
    return textwrap.dedent(
        f"""\
        {STICKY_MARKER}
        ## {DISPLAY_NAME}

        Updated: {now_text}
        PR: #{pr_number}
        Status: **{status}**

        {headline}

        - Allowed findings: {len(allowed)} ({count_text})
        - Inline comments posted: {posted}
        - Existing inline comments kept: {skipped_existing}
        - Inline comments skipped: {skipped_inline}
        - Bot-managed resolved comments: {resolved}
        - Missing axes: {missing_axes}

        Cross-cutting findings:
        {cross_cutting_text}

        Inline skipped findings:
        {skipped_inline_text}

        재리뷰는 `/codex-review` 코멘트로 요청하세요.
        """
    ).strip()


def render_superseded_sticky_body() -> str:
    return "\n".join(
        [
            SUPERSEDED_STICKY_MARKER,
            f"_{DISPLAY_NAME}: newer review summary superseded this comment._",
        ]
    )


def sticky_sort_key(comment: dict[str, Any]) -> tuple[str, str, int]:
    comment_id = comment.get("id")
    try:
        numeric_id = int(comment_id)
    except (TypeError, ValueError):
        numeric_id = 0
    return (
        str(comment.get("updated_at") or ""),
        str(comment.get("created_at") or ""),
        numeric_id,
    )


def select_sticky_comment(comments: list[dict[str, Any]]) -> dict[str, Any] | None:
    candidates = [
        comment
        for comment in comments
        if STICKY_MARKER in str(comment.get("body") or "") and is_bot_comment(comment)
    ]
    if not candidates:
        return None
    return max(candidates, key=sticky_sort_key)


def comment_commit_id(comment: dict[str, Any]) -> str:
    return str(
        comment.get("commit_id")
        or comment.get("commit_sha")
        or comment.get("commit")
        or ""
    )


def existing_by_key(comments: list[dict[str, Any]], head_sha: str | None = None) -> dict[str, dict[str, Any]]:
    result: dict[str, dict[str, Any]] = {}
    for comment in comments:
        if not is_bot_comment(comment):
            continue
        if head_sha and comment_commit_id(comment) != head_sha:
            continue
        key, status = extract_marker(str(comment.get("body") or ""))
        if key and status == "active":
            result.setdefault(key, comment)
    return result


def load_changed_ranges(context_path: Path) -> dict[str, list[tuple[int, int]]]:
    context = require_json(context_path)
    if not isinstance(context, dict):
        raise SystemExit(f"required review context is invalid JSON object: {context_path}")
    changed_ranges: dict[str, list[tuple[int, int]]] = {}
    for row in context.get("changed_files") or []:
        if not isinstance(row, dict):
            continue
        path = str(row.get("filename") or "")
        if not path:
            continue
        ranges: list[tuple[int, int]] = []
        for item in row.get("changed_right_ranges") or []:
            if not isinstance(item, dict):
                continue
            try:
                start = int(item.get("start"))
                end = int(item.get("end"))
            except (TypeError, ValueError):
                continue
            if start <= 0 or end < start:
                continue
            ranges.append((start, end))
        changed_ranges[path] = ranges
    return changed_ranges


def load_existing_inline_comments(context_path: Path) -> list[dict[str, Any]]:
    context = require_json(context_path)
    if not isinstance(context, dict):
        raise SystemExit(f"required review context is invalid JSON object: {context_path}")
    comments = context.get("existing_inline_comments")
    if not isinstance(comments, list):
        return []
    return [comment for comment in comments if isinstance(comment, dict)]


def line_in_ranges(line: int, ranges: list[tuple[int, int]]) -> bool:
    return any(start <= line <= end for start, end in ranges)


def update_review_comment_body(client: ForgejoClient, comment: dict[str, Any], body: str) -> bool:
    comment_id = comment.get("id")
    if not comment_id:
        warn("could not update review comment without comment id")
        return False
    try:
        client.request(
            "PATCH",
            client.repo_path(f"issues/comments/{comment_id}"),
            {"body": body},
        )
        return True
    except ForgejoApiError as exc:
        warn(f"could not update review comment {comment_id}: {exc}")
        return False


def submit_inline_batch(
    client: ForgejoClient,
    pr_number: str,
    head_sha: str,
    comments: list[dict[str, Any]],
) -> int:
    if not comments:
        return 0
    payload = {
        "event": "COMMENT",
        "commit_id": head_sha,
        "body": f"{DISPLAY_NAME} automated review",
        "comments": comments,
    }
    try:
        client.request("POST", client.repo_path(f"pulls/{pr_number}/reviews"), payload)
        return len(comments)
    except ForgejoApiError as exc:
        warn(f"could not post inline review batch; retrying comments individually: {exc}")

    existing_after_failure = remote_active_inline_keys(client, pr_number, head_sha)
    posted = 0
    failed: list[str] = []
    for comment in comments:
        key, _status = extract_marker(str(comment.get("body") or ""))
        if key and key in existing_after_failure:
            continue
        try:
            client.request(
                "POST",
                client.repo_path(f"pulls/{pr_number}/reviews"),
                {
                    "event": "COMMENT",
                    "commit_id": head_sha,
                    "body": f"{DISPLAY_NAME} automated review",
                    "comments": [comment],
                },
            )
            posted += 1
        except ForgejoApiError as exc:
            failed.append(f"{comment.get('path')}:{comment.get('new_position')}")
            warn(
                "could not post inline review comment "
                f"{comment.get('path')}:{comment.get('new_position')}: {exc}"
            )
    if failed:
        raise SystemExit("could not post inline review comments: " + ", ".join(failed))
    return posted


def remote_active_inline_keys(client: ForgejoClient, pr_number: str, head_sha: str) -> set[str]:
    try:
        comments = client.paginated(
            client.repo_path(f"pulls/{pr_number}/comments"), limit=100, max_pages=INLINE_REQUERY_PAGE_LIMIT
        )
    except ForgejoApiError as exc:
        raise SystemExit(
            "could not re-query inline comments after batch failure; refusing duplicate-prone retry"
        ) from exc
    keys: set[str] = set()
    for comment in comments:
        if not is_bot_comment(comment):
            continue
        if comment_commit_id(comment) != head_sha:
            continue
        key, status = extract_marker(str(comment.get("body") or ""))
        if key and status == "active":
            keys.add(key)
    return keys


def post_inline_comments(
    client: ForgejoClient,
    pr_number: str,
    head_sha: str,
    allowed: list[dict[str, Any]],
    existing: dict[str, dict[str, Any]],
    changed_ranges: dict[str, list[tuple[int, int]]] | None = None,
) -> tuple[int, int, int]:
    new_comments: list[dict[str, Any]] = []
    queued_keys: set[str] = set()
    skipped_existing = 0
    skipped_inline = 0
    for finding in allowed:
        path = finding.get("file")
        line = finding.get("line")
        if not path or not line or finding.get("cross_cutting"):
            continue
        if changed_ranges is not None and not line_in_ranges(int(line), changed_ranges.get(str(path), [])):
            finding["_inline_skip_reason"] = "outside_changed_lines"
            skipped_inline += 1
            continue
        key = finding_key(finding)
        body = render_inline_body(finding, key)
        if key in queued_keys:
            skipped_existing += 1
            continue
        if key in existing:
            if str(existing[key].get("body") or "") == body:
                skipped_existing += 1
                continue
            if not update_review_comment_body(client, existing[key], body):
                raise SystemExit(f"could not update existing review comment {existing[key].get('id')}")
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
        queued_keys.add(key)
    if not new_comments:
        return 0, skipped_existing, skipped_inline
    posted = 0
    for offset in range(0, len(new_comments), MAX_INLINE_COMMENTS):
        batch = new_comments[offset : offset + MAX_INLINE_COMMENTS]
        posted += submit_inline_batch(client, pr_number, head_sha, batch)
    return posted, skipped_existing, skipped_inline


def postable_inline_keys(
    findings: list[dict[str, Any]],
    changed_ranges: dict[str, list[tuple[int, int]]] | None,
) -> set[str]:
    keys: set[str] = set()
    for finding in findings:
        path = finding.get("file")
        line = finding.get("line")
        if not path or not line or finding.get("cross_cutting"):
            continue
        if changed_ranges is not None and not line_in_ranges(int(line), changed_ranges.get(str(path), [])):
            continue
        keys.add(finding_key(finding))
    return keys


def reply_resolved(
    client: ForgejoClient,
    pr_number: str,
    comments: list[dict[str, Any]],
    current_keys: set[str],
    head_sha: str,
) -> int:
    resolved = 0
    resolved_keys = {
        key
        for comment in comments
        if is_bot_comment(comment)
        for match in RESOLVED_INLINE_MARKER_RE.finditer(str(comment.get("body") or ""))
        for key in match.groups()
        if key
    }
    stale_comments: list[tuple[Any, str]] = []
    for comment in comments:
        if not is_bot_comment(comment):
            continue
        body = str(comment.get("body") or "")
        key, status = extract_marker(body)
        comment_id = comment.get("id")
        same_head = comment_commit_id(comment) == head_sha
        if not key or status != "active" or (same_head and key in current_keys) or not comment_id:
            continue
        if key in resolved_keys:
            continue
        stale_comments.append((comment_id, key))
    if not stale_comments:
        return 0
    if len(stale_comments) > MAX_RESOLVE_COMMENTS:
        warn(
            "limiting bot-managed resolved inline comment updates to "
            f"{MAX_RESOLVE_COMMENTS} of {len(stale_comments)} stale comments"
        )
        stale_comments = stale_comments[-MAX_RESOLVE_COMMENTS:]

    def mark_resolved(comment_id: Any, key: str) -> bool:
        try:
            client.request(
                "PATCH",
                client.repo_path(f"issues/comments/{comment_id}"),
                {"body": render_resolved_body(key)},
            )
            return True
        except ForgejoApiError as exc:
            warn(f"could not mark review comment {comment_id} as bot-managed resolved: {exc}")
            return False

    with ThreadPoolExecutor(max_workers=min(8, max(1, len(stale_comments)))) as executor:
        futures = [executor.submit(mark_resolved, comment_id, key) for comment_id, key in stale_comments]
        for future in as_completed(futures):
            if future.result():
                resolved += 1
    return resolved


def sticky_id_from_context(context_path: Path) -> Any:
    try:
        context = require_json(context_path)
    except SystemExit:
        return None
    if not isinstance(context, dict):
        return None
    comments = context.get("existing_sticky_comments")
    if not isinstance(comments, list) or not comments:
        return None
    candidates = [comment for comment in comments if isinstance(comment, dict)]
    selected = select_sticky_comment(candidates)
    if selected:
        return selected.get("id")
    return max(candidates, key=sticky_sort_key).get("id")


def retire_duplicate_stickies(
    client: ForgejoClient,
    keep_id: Any,
    comments: list[dict[str, Any]],
) -> None:
    keep_id_text = str(keep_id)
    retired_body = render_superseded_sticky_body()
    for comment in comments:
        comment_id = comment.get("id")
        if str(comment_id) == keep_id_text:
            continue
        if STICKY_MARKER not in str(comment.get("body") or "") or not is_bot_comment(comment):
            continue
        try:
            client.request(
                "PATCH",
                client.repo_path(f"issues/comments/{comment_id}"),
                {"body": retired_body},
            )
        except ForgejoApiError as exc:
            warn(f"could not mark duplicate sticky comment {comment_id} as superseded: {exc}")


def upsert_sticky(client: ForgejoClient, pr_number: str, body: str, existing_id: Any = None) -> None:
    if existing_id:
        try:
            client.request(
                "PATCH",
                client.repo_path(f"issues/comments/{existing_id}"),
                {"body": body},
            )
        except ForgejoApiError as exc:
            warn(f"could not patch sticky comment {existing_id}: {exc}")
        else:
            comments = client.paginated(client.repo_path(f"issues/{pr_number}/comments"), limit=100, max_pages=10)
            retire_duplicate_stickies(client, existing_id, comments)
            return

    comments = client.paginated(client.repo_path(f"issues/{pr_number}/comments"), limit=100, max_pages=10)
    existing = select_sticky_comment(comments)
    if existing:
        selected_id = existing.get("id")
        try:
            client.request(
                "PATCH",
                client.repo_path(f"issues/comments/{selected_id}"),
                {"body": body},
            )
            retire_duplicate_stickies(client, selected_id, comments)
            return
        except ForgejoApiError as exc:
            warn(f"could not patch sticky comment {selected_id}: {exc}")
    client.request(
        "POST",
        client.repo_path(f"issues/{pr_number}/comments"),
        {"body": body},
    )


def ensure_pr_snapshot_unchanged(
    client: ForgejoClient,
    pr_number: str,
    expected_head_sha: str,
    expected_base_sha: str,
) -> None:
    pr = client.request("GET", client.repo_path(f"pulls/{pr_number}"))
    if not isinstance(pr, dict):
        raise SystemExit("unexpected Forgejo PR response")
    current_head_sha = str(((pr.get("head") or {}).get("sha")) or "")
    current_base_sha = str(((pr.get("base") or {}).get("sha")) or "")
    if not current_head_sha:
        raise SystemExit("PR head SHA is missing in Forgejo response; refusing to publish review comments")
    if not current_base_sha:
        raise SystemExit("PR base SHA is missing in Forgejo response; refusing to publish review comments")
    if current_head_sha and current_head_sha != expected_head_sha:
        raise SystemExit(
            "PR head SHA changed before publishing review comments; "
            f"expected {expected_head_sha}, got {current_head_sha}"
        )
    if current_base_sha and current_base_sha != expected_base_sha:
        raise SystemExit(
            "PR base SHA changed before publishing review comments; "
            f"expected {expected_base_sha}, got {current_base_sha}"
        )


def main() -> int:
    client = ForgejoClient.from_env()
    configure_bot_login_from_token(client)
    pr_number = require_env("PR_NUMBER")
    head_sha = require_env("HEAD_SHA")
    base_sha = require_env("BASE_SHA")
    art_dir = Path(os.environ.get("ART_DIR", "artifacts"))
    allowed = require_json(art_dir / "allowed.json")
    axes_status = require_json(art_dir / "axes_status.json")
    decisions = require_json(art_dir / "decisions.json")
    judgment = decisions.get("judgment") if isinstance(decisions, dict) else None
    if not isinstance(allowed, list):
        allowed = []

    ensure_pr_snapshot_unchanged(client, pr_number, head_sha, base_sha)
    context_path = art_dir / "pr-context.json"
    changed_ranges = load_changed_ranges(context_path)
    review_comments = load_existing_inline_comments(context_path)
    existing = existing_by_key(review_comments, head_sha)
    current_keys = postable_inline_keys(allowed, changed_ranges)
    posted, skipped_existing, skipped_inline = post_inline_comments(
        client,
        pr_number,
        head_sha,
        allowed,
        existing,
        changed_ranges,
    )
    resolved = reply_resolved(client, pr_number, review_comments, current_keys, head_sha)
    sticky = render_sticky(
        pr_number,
        allowed,
        posted,
        skipped_existing,
        resolved,
        skipped_inline,
        axes_status if isinstance(axes_status, dict) else {},
        judgment if isinstance(judgment, dict) else None,
    )
    upsert_sticky(client, pr_number, sticky, sticky_id_from_context(context_path))
    print(
        "Forgejo review posted: "
        f"inline={posted}, kept={skipped_existing}, skipped_inline={skipped_inline}, "
        f"resolved={resolved}, allowed={len(allowed)}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
