#!/usr/bin/env python3
"""PR 메타데이터 / 변경 파일 / 기존 인라인 코멘트를 수집해 컨텍스트 JSON 으로 저장.

``$RUNNER_TEMP/codex-review-context.json`` 에 결과를 쓰며, 이후 ``build_prompt.py``
와 ``post_review_comments.py`` 가 이 파일을 읽어 사용한다. 시크릿은 ``codex_redaction``
의 ``redact()`` 를 통해 마스킹된다.

필수 환경 변수
  - GITHUB_REPOSITORY: ``owner/repo``
  - PR_NUMBER:         PR 번호
  - HEAD_SHA:          PR head 커밋 SHA
  - RUNNER_TEMP:       임시 디렉토리
  - GH_TOKEN:          ``gh`` CLI 인증 토큰
선택
  - GITHUB_BASE_REF / GITHUB_HEAD_REF
"""

from __future__ import annotations

import json
import os
import re
from pathlib import Path

from codex_redaction import redact
from gh_api import gh_json, gh_paginated

MARKER = "<!-- codex-inline-review -->"
MAX_CHANGED_LINES = 2000
MAX_PATCH_EXCERPT_CHARS = 12000
MAX_COMMENT_BODY_CHARS = 4000
# Total byte budget across all changed-file excerpts. Prevents axis prompts
# from blowing past Codex context limits on PRs that touch hundreds of files.
MAX_TOTAL_PATCH_BYTES = 200_000


def changed_right_lines(patch: str | None) -> list[int]:
    """patch 의 RIGHT-side (추가) 라인 번호 목록."""
    result: list[int] = []
    right_line: int | None = None
    for raw in (patch or "").splitlines():
        if raw.startswith("@@"):
            match = re.search(r"\+(\d+)(?:,(\d+))?", raw)
            right_line = int(match.group(1)) if match else None
            continue
        if right_line is None:
            continue
        if raw.startswith("+") and not raw.startswith("+++"):
            result.append(right_line)
            right_line += 1
        elif raw.startswith("-") and not raw.startswith("---"):
            continue
        else:
            right_line += 1
    return result


def main() -> int:
    repo = os.environ["GITHUB_REPOSITORY"]
    pr_number = os.environ["PR_NUMBER"]
    head_sha = os.environ["HEAD_SHA"]
    base_sha = os.environ.get("BASE_SHA", "")
    runner_temp = Path(os.environ["RUNNER_TEMP"])

    pr = gh_json(f"repos/{repo}/pulls/{pr_number}")
    current_head_sha = ((pr or {}).get("head") or {}).get("sha") or ""
    current_base_sha = ((pr or {}).get("base") or {}).get("sha") or ""
    if current_head_sha != head_sha:
        raise SystemExit(
            "::error::PR head SHA changed while preparing review context "
            f"(expected {head_sha}, got {current_head_sha})"
        )
    if base_sha and current_base_sha != base_sha:
        raise SystemExit(
            "::error::PR base SHA changed while preparing review context "
            f"(expected {base_sha}, got {current_base_sha})"
        )

    files = gh_paginated(f"repos/{repo}/pulls/{pr_number}/files")
    comments = gh_paginated(f"repos/{repo}/pulls/{pr_number}/comments")

    context: dict = {
        "pull_request": {
            "number": int(pr_number),
            "base_ref": os.environ.get("GITHUB_BASE_REF", ""),
            "base_sha": base_sha,
            "head_ref": os.environ.get("GITHUB_HEAD_REF", ""),
            "head_sha": head_sha,
        },
        "changed_files": [],
        "existing_inline_comments": [],
    }

    # Process files sorted by total change volume so large/important diffs win
    # the byte budget when truncation kicks in. Original order is preserved
    # for the output by remembering the input index.
    sorted_files = sorted(
        enumerate(files),
        key=lambda pair: int(pair[1].get("changes") or 0),
        reverse=True,
    )
    total_patch_bytes = 0
    rendered_by_index: dict[int, dict] = {}
    for original_index, item in sorted_files:
        patch = item.get("patch") or ""
        per_file = patch[:MAX_PATCH_EXCERPT_CHARS]
        remaining = MAX_TOTAL_PATCH_BYTES - total_patch_bytes
        truncated = len(patch) > MAX_PATCH_EXCERPT_CHARS
        if remaining <= 0:
            per_file = ""
            truncated = True
        elif len(per_file) > remaining:
            per_file = per_file[:remaining]
            truncated = True
        redacted_excerpt = redact(per_file)
        total_patch_bytes += len(redacted_excerpt.encode("utf-8"))
        rendered_by_index[original_index] = {
            "filename": item.get("filename", ""),
            "status": item.get("status", ""),
            "additions": item.get("additions", 0),
            "deletions": item.get("deletions", 0),
            "changes": item.get("changes", 0),
            "changed_right_lines": changed_right_lines(patch)[:MAX_CHANGED_LINES],
            "patch_excerpt": redacted_excerpt,
            "truncated": truncated,
        }
    for original_index in range(len(files)):
        context["changed_files"].append(rendered_by_index[original_index])

    for comment in comments:
        body = comment.get("body") or ""
        user = comment.get("user") or {}
        managed = MARKER in body
        # Identify "our" managed comments by marker only (App installation
        # token causes the bot login to vary), so the LLM still sees them as
        # editable and won't treat them as third-party reviews.
        editable = managed
        entry = {
            "id": comment.get("id"),
            "path": comment.get("path") or "",
            "line": comment.get("line") or 0,
            "original_line": comment.get("original_line") or 0,
            "side": comment.get("side") or comment.get("original_side") or "RIGHT",
            "user_login": user.get("login", ""),
            "user_type": user.get("type", ""),
            "editable": editable,
            "codex_managed": managed,
            "body_excerpt": redact(body[:MAX_COMMENT_BODY_CHARS]) if editable else "",
        }
        if entry["path"]:
            context["existing_inline_comments"].append(entry)

    out_path = runner_temp / "codex-review-context.json"
    out_path.write_text(
        json.dumps(context, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
