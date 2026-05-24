#!/usr/bin/env python3
"""Collect Forgejo PR metadata for Codex review prompts."""

from __future__ import annotations

import json
import os
import re
import sys
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path
from typing import Any

from forgejo_api import ForgejoApiError, ForgejoClient, require_env, warn
from codex_redaction import find_unredacted_secret_risks, redact  # noqa: E402

INLINE_MARKER = "<!-- forgejo-codex-inline"
STICKY_MARKER = "<!-- forgejo-codex-review-sticky -->"
PROMPT_DIFF_LIMIT = 1_000_000
PULL_FILES_PAGE_LIMIT = 10
ISSUE_COMMENTS_PAGE_LIMIT = 10
REVIEW_COMMENTS_PAGE_LIMIT = 5
REVIEW_FALLBACK_PAGE_LIMIT = 5
REVIEW_FALLBACK_COMMENT_PAGE_LIMIT = 3
MAX_CHANGED_LINES = 2000
INLINE_MARKER_RE = re.compile(
    r"<!--\s*forgejo-codex-inline\s+key=\"([0-9a-f]+)\"\s+status=\"([a-z-]+)\"\s*-->"
)


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


def configure_bot_login_from_token(client: ForgejoClient) -> None:
    configured = os.environ.get("FORGEJO_BOT_LOGIN", "").strip()
    try:
        login = client.authenticated_login()
    except (AttributeError, ForgejoApiError) as exc:
        if configured:
            return
        warn(f"could not resolve authenticated Forgejo bot login; bot comment trust is disabled: {exc}")
        return
    if login:
        if configured and configured != login:
            warn("configured Forgejo bot login did not match authenticated token user; using token login")
        os.environ["FORGEJO_BOT_LOGIN"] = login


def is_bot_comment(comment: dict[str, Any]) -> bool:
    return comment_login(comment) in bot_logins()


def extract_inline_marker(body: str) -> tuple[str | None, str | None]:
    match = INLINE_MARKER_RE.search(body)
    if not match:
        return None, None
    return match.group(1), match.group(2)


class ChangedRightLineCollector:
    def __init__(self, max_changed_lines: int | None = None) -> None:
        self.changed: dict[str, set[int]] = {}
        self.positions: dict[str, dict[int, int]] = {}
        self.current_file: str | None = None
        self.in_hunk = False
        self.new_line = 0
        self.diff_position = 0
        self.max_changed_lines = max_changed_lines
        self.changed_count = 0

    def feed(self, raw: str) -> None:
        if raw.startswith("diff --git "):
            self.current_file = None
            self.in_hunk = False
            self.diff_position = 0
            return
        if not self.in_hunk and raw.startswith("+++ b/"):
            self.current_file = raw[6:]
            self.changed.setdefault(self.current_file, set())
            self.positions.setdefault(self.current_file, {})
            return
        if not self.in_hunk and raw.startswith("+++ /dev/null"):
            self.current_file = None
            return
        match = re.match(r"@@ -\d+(?:,\d+)? \+(\d+)(?:,\d+)? @@", raw)
        if match:
            self.in_hunk = True
            self.new_line = int(match.group(1))
            self.diff_position = 0
            return
        if self.current_file is None:
            return
        if raw.startswith("\\ "):
            return
        if raw.startswith("+"):
            if self.max_changed_lines is None or self.changed_count < self.max_changed_lines:
                self.changed[self.current_file].add(self.new_line)
                self.positions[self.current_file][self.new_line] = self.diff_position + 1
                self.changed_count += 1
            self.diff_position += 1
            self.new_line += 1
        elif raw.startswith("-"):
            self.diff_position += 1
            return
        else:
            self.diff_position += 1
            self.new_line += 1


def parse_changed_right_lines(diff: str) -> dict[str, set[int]]:
    collector = ChangedRightLineCollector(max_changed_lines=MAX_CHANGED_LINES)
    for raw in diff.splitlines():
        collector.feed(raw)
    return collector.changed


def collapse_line_ranges(lines: set[int]) -> list[dict[str, int]]:
    ranges: list[dict[str, int]] = []
    start: int | None = None
    previous: int | None = None
    for line in sorted(lines):
        if start is None or previous is None:
            start = line
            previous = line
            continue
        if line == previous + 1:
            previous = line
            continue
        ranges.append({"start": start, "end": previous})
        start = line
        previous = line
    if start is not None and previous is not None:
        ranges.append({"start": start, "end": previous})
    return ranges


def fetch_diff_context(
    client: ForgejoClient,
    pr_number: str,
) -> tuple[str, bool, dict[str, set[int]]]:
    collector = ChangedRightLineCollector(max_changed_lines=None)
    prompt_diff, diff_truncated = client.request_text_prefix(
        client.repo_path(f"pulls/{pr_number}.diff"),
        PROMPT_DIFF_LIMIT,
        accept="text/plain",
        on_line=collector.feed,
        continue_after_prefix=False,
    )
    return prompt_diff, diff_truncated, collector.changed


def paginated_filter(
    client: ForgejoClient,
    path: str,
    predicate,
    limit: int = 100,
    max_pages: int | None = 50,
) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    seen_ids: set[Any] = set()
    page = 1
    while max_pages is None or page <= max_pages:
        payload = client.request("GET", path, query={"limit": str(limit), "page": str(page)})
        if not isinstance(payload, list) or not payload:
            break
        new_count = 0
        for item in payload:
            if not isinstance(item, dict):
                continue
            item_id = item.get("id")
            if item_id is not None:
                if item_id in seen_ids:
                    continue
                seen_ids.add(item_id)
            new_count += 1
            if predicate(item):
                rows.append(item)
        if new_count == 0:
            break
        page += 1
    return rows


def fetch_review_comments(
    client: ForgejoClient,
    pr_number: str,
    strict: bool = False,
    max_workers: int = 8,
    max_bot_reviews: int | None = None,
    use_aggregate: bool = True,
) -> list[dict[str, Any]]:
    comments: list[dict[str, Any]] = []
    if use_aggregate:
        try:
            aggregate_comments = paginated_filter(
                client,
                client.repo_path(f"pulls/{pr_number}/comments"),
                is_bot_comment,
                max_pages=REVIEW_COMMENTS_PAGE_LIMIT,
            )
        except ForgejoApiError as exc:
            warn(f"could not list aggregate pull review comments; falling back to per-review fetch: {exc}")
        else:
            return aggregate_comments

    try:
        reviews = client.paginated(
            client.repo_path(f"pulls/{pr_number}/reviews"),
            limit=100,
            max_pages=REVIEW_FALLBACK_PAGE_LIMIT,
        )
    except ForgejoApiError as exc:
        if strict:
            raise
        warn(f"could not list pull reviews; inline context will be empty: {exc}")
        return []
    bot_reviews = []
    for review in reviews:
        if not is_bot_comment(review):
            continue
        review_id = review.get("id")
        if not review_id:
            continue
        bot_reviews.append(review_id)
    fallback_limit = 10 if max_bot_reviews is None else max_bot_reviews
    if fallback_limit > 0 and len(bot_reviews) > fallback_limit:
        warn(
            "aggregate pull review comments unavailable; "
            f"limiting fallback review comment fetch to the latest {fallback_limit} bot reviews"
        )
        bot_reviews = bot_reviews[-fallback_limit:]

    def fetch_one(review_id: Any) -> list[dict[str, Any]]:
        return client.paginated(
            client.repo_path(f"pulls/{pr_number}/reviews/{review_id}/comments"),
            limit=100,
            max_pages=REVIEW_FALLBACK_COMMENT_PAGE_LIMIT,
        )

    review_iter = iter(bot_reviews)
    with ThreadPoolExecutor(max_workers=max(1, max_workers)) as executor:
        futures = {}

        def submit_next() -> None:
            try:
                review_id = next(review_iter)
            except StopIteration:
                return
            futures[executor.submit(fetch_one, review_id)] = review_id

        for _ in range(max(1, max_workers)):
            submit_next()

        while futures:
            for future in as_completed(list(futures)):
                review_id = futures.pop(future)
                break
            try:
                review_comments = future.result()
            except ForgejoApiError as exc:
                if strict:
                    raise
                warn(f"could not list pull review comments for review {review_id}: {exc}")
                submit_next()
                continue
            comments.extend(review_comments)
            submit_next()
    return comments


def normalize_file(
    row: dict[str, Any],
    changed_lines: dict[str, set[int]],
) -> dict[str, Any]:
    path = str(row.get("filename") or row.get("name") or row.get("path") or "")
    return {
        "filename": ensure_no_secret_risks("file name", path),
        "status": row.get("status") or "",
        "additions": row.get("additions") or 0,
        "deletions": row.get("deletions") or 0,
        "changes": row.get("changes") or 0,
        "changed_right_ranges": collapse_line_ranges(changed_lines.get(path, set())),
    }


def ensure_no_secret_risks(label: str, text: str) -> str:
    redacted = redact(text)
    if find_unredacted_secret_risks(redacted):
        raise SystemExit(
            f"PR {label} still contains credential-like literals after redaction; "
            "refusing to write review prompt artifact"
        )
    return redacted


def validate_pr_snapshot(
    client: ForgejoClient,
    pr_number: str,
    head_sha_expected: str,
    base_sha_expected: str,
) -> dict[str, Any]:
    pr = client.request("GET", client.repo_path(f"pulls/{pr_number}"))
    if not isinstance(pr, dict):
        raise SystemExit("unexpected Forgejo PR response")

    head = pr.get("head") or {}
    base = pr.get("base") or {}
    head_sha = str(head.get("sha") or "")
    base_sha = str(base.get("sha") or "")
    if head_sha_expected and not head_sha:
        raise SystemExit("PR head SHA missing while preparing review context")
    if base_sha_expected and not base_sha:
        raise SystemExit("PR base SHA missing while preparing review context")
    if head_sha_expected and head_sha and head_sha != head_sha_expected:
        raise SystemExit("PR head SHA changed while preparing review context")
    if base_sha_expected and base_sha and base_sha != base_sha_expected:
        raise SystemExit("PR base SHA changed while preparing review context")
    return pr


def fetch_context_api_data(
    client: ForgejoClient,
    pr_number: str,
) -> tuple[list[dict[str, Any]], list[dict[str, Any]], list[dict[str, Any]]]:
    def fetch_files() -> list[dict[str, Any]]:
        return client.paginated(
            client.repo_path(f"pulls/{pr_number}/files"),
            limit=100,
            max_pages=PULL_FILES_PAGE_LIMIT + 1,
        )

    def fetch_issue_comments() -> list[dict[str, Any]]:
        return paginated_filter(
            client,
            client.repo_path(f"issues/{pr_number}/comments"),
            lambda comment: STICKY_MARKER in str(comment.get("body") or "") and is_bot_comment(comment),
            max_pages=ISSUE_COMMENTS_PAGE_LIMIT,
        )

    def fetch_inline_comments() -> list[dict[str, Any]]:
        return fetch_review_comments(client, pr_number, strict=True)

    with ThreadPoolExecutor(max_workers=3) as executor:
        files_future = executor.submit(fetch_files)
        issue_comments_future = executor.submit(fetch_issue_comments)
        review_comments_future = executor.submit(fetch_inline_comments)
        return files_future.result(), issue_comments_future.result(), review_comments_future.result()


def main() -> int:
    client = ForgejoClient.from_env()
    configure_bot_login_from_token(client)
    pr_number = require_env("PR_NUMBER")
    runner_temp = Path(require_env("RUNNER_TEMP"))
    runner_temp.mkdir(parents=True, exist_ok=True)
    head_sha_expected = os.environ.get("HEAD_SHA", "").strip()
    base_sha_expected = os.environ.get("BASE_SHA", "").strip()

    pr = validate_pr_snapshot(client, pr_number, head_sha_expected, base_sha_expected)
    head = pr.get("head") or {}
    base = pr.get("base") or {}
    head_sha = str(head.get("sha") or "")
    base_sha = str(base.get("sha") or "")

    with ThreadPoolExecutor(max_workers=2) as executor:
        diff_future = executor.submit(fetch_diff_context, client, pr_number)
        api_future = executor.submit(fetch_context_api_data, client, pr_number)
        prompt_diff, diff_truncated, changed_lines = diff_future.result()
        files, issue_comments, review_comments = api_future.result()
    if diff_truncated:
        raise SystemExit(
            "PR diff exceeds Codex review size limit; refusing partial review because changed lines after the "
            "truncation boundary would not be visible to review agents"
        )

    if len(files) > PULL_FILES_PAGE_LIMIT * 100:
        raise SystemExit("PR changed file list exceeds Codex review size limit; refusing partial review")
    pr = validate_pr_snapshot(client, pr_number, head_sha_expected, base_sha_expected)
    head = pr.get("head") or {}
    base = pr.get("base") or {}
    head_sha = str(head.get("sha") or "")
    base_sha = str(base.get("sha") or "")

    existing_inline = []
    for comment in review_comments:
        body = str(comment.get("body") or "")
        if INLINE_MARKER not in body or not is_bot_comment(comment):
            continue
        _, status = extract_inline_marker(body)
        if status != "active":
            continue
        existing_inline.append(
            {
                "id": comment.get("id"),
                "path": comment.get("path") or "",
                "line": comment.get("position") or comment.get("line") or 0,
                "commit_id": comment.get("commit_id") or comment.get("commit_sha") or comment.get("commit") or "",
                "body": body,
                "html_url": comment.get("html_url") or "",
                "updated_at": comment.get("updated_at") or "",
                "user": {"login": comment_login(comment)},
            }
        )

    redacted_title = ensure_no_secret_risks("title", str(pr.get("title") or ""))
    redacted_body = ensure_no_secret_risks("body", str(pr.get("body") or ""))
    redacted_diff = ensure_no_secret_risks("diff", prompt_diff)
    for comment in existing_inline:
        comment["body"] = ensure_no_secret_risks("existing inline comment", str(comment.get("body") or ""))

    context = {
        "provider": "forgejo",
        "repository": client.repo,
        "pr_number": int(pr_number),
        "title": redacted_title,
        "body": redacted_body,
        "base_ref": base.get("ref") or os.environ.get("GITHUB_BASE_REF", ""),
        "base_sha": base_sha or base_sha_expected,
        "head_ref": head.get("ref") or os.environ.get("GITHUB_HEAD_REF", ""),
        "head_sha": head_sha or head_sha_expected,
        "diff": redacted_diff,
        "diff_truncated": diff_truncated,
        "changed_files": [normalize_file(row, changed_lines) for row in files],
        "existing_inline_comments": existing_inline,
        "existing_sticky_comments": [
            {
                "id": c.get("id"),
                "body": ensure_no_secret_risks("existing sticky comment", str(c.get("body") or "")),
                "updated_at": c.get("updated_at") or "",
            }
            for c in issue_comments
        ],
    }
    out = runner_temp / "pr-context.json"
    out.write_text(json.dumps(context, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"wrote Forgejo PR context: {out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
