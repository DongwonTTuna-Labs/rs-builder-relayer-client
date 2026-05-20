#!/usr/bin/env python3
"""Stage 3 — hard rule 적용 + inline / sticky 코멘트 게시 (LLM 미사용).

입력 아티팩트
  - $ART_DIR/allowed.json           : tech-lead + hard rule 을 거쳐 살아남은 findings
  - $ART_DIR/decisions.json         : judgment.status / headline
  - $ART_DIR/findings-*.json        : positive[] / impact_summary
  - $ART_DIR/resolutions.json       : Stage 0 (resolve-check) 의 LLM 판정 결과

처리 흐름
  - allowed.json 의 findings → file:line 에 inline 코멘트 PATCH/POST
  - resolutions.resolved=true 인 codex inline 코멘트:
      * ASK   : Codex 판정 사유를 포함한 reply + ``minimizeComment`` (Hide, RESOLVED) + ``resolveReviewThread``
      * 그 외 : "수정작업완료" reply + ``resolveReviewThread``
    isResolved / isMinimized 인 thread 는 ``thread_map`` 에 없어 자동으로 건너뛴다.
  - 종합 sticky 코멘트는 ``review-summary-template.md`` 를 envsubst 식으로 채워 PATCH/POST.

필수 환경 변수
  - GH_TOKEN: GitHub App installation token
  - GITHUB_REPOSITORY, PR_NUMBER, HEAD_SHA
선택
  - GITHUB_STEP_SUMMARY (Actions 가 자동 주입)
  - ART_DIR  (기본: ./artifacts)
  - TEMPLATE (기본: .github/scripts/review-summary-template.md)
  - TRIGGER  (기본: unknown) — sticky 본문에 노출
"""

from __future__ import annotations

import hashlib
import json
import os
import re
from pathlib import Path
from string import Template

from codex_redaction import redact
from gh_api import gh_graphql, gh_json, gh_paginated

INLINE_MARKER = "<!-- codex-inline-review -->"
STICKY_MARKER = "<!-- codex-review-sticky -->"
BOT_LOGIN = "github-actions[bot]"


def is_codex_inline(comment: dict) -> bool:
    """codex 가 작성한 inline 코멘트인지 식별."""
    user = comment.get("user") or {}
    if (user.get("login") or "") != BOT_LOGIN:
        return False
    return INLINE_MARKER in (comment.get("body") or "")

AXIS_LABEL_MAP = {
    "correctness": "정확성",
    "security": "보안",
    "performance": "성능",
    "test-coverage": "테스트",
    "domain": "도메인",
}
TYPE_BADGE = {
    "MUST": ("must", "red"),
    "SUGGEST": ("suggest", "blue"),
    "IMO": ("imo", "orange"),
    "NITS": ("nits", "green"),
    "ASK": ("ask", "yellowgreen"),
}
TYPE_BADGE_RE = re.compile(r"!\[(must|suggest|imo|nits|ask)-badge\]")

REPLY_BODY_FIX = (
    "✅ 수정작업완료\n\n"
    "(Codex resolve-check 가 PR head 의 현재 코드를 검토한 결과 이 지적이 해결되었다고 판정했습니다. "
    "잘못 닫혔다면 thread 의 *Unresolve conversation* 을 눌러 다시 열어주세요.)"
)


RESOLVE_MUTATION = """
mutation($id: ID!) {
  resolveReviewThread(input: {threadId: $id}) {
    thread { id isResolved }
  }
}
"""

MINIMIZE_MUTATION = """
mutation($id: ID!, $classifier: ReportedContentClassifiers!) {
  minimizeComment(input: {subjectId: $id, classifier: $classifier}) {
    minimizedComment { isMinimized minimizedReason }
  }
}
"""

THREAD_MAP_QUERY = """
query($owner: String!, $name: String!, $pr: Int!) {
  repository(owner: $owner, name: $name) {
    pullRequest(number: $pr) {
      reviewThreads(first: 100) {
        nodes {
          id
          isResolved
          comments(first: 1) {
            nodes { databaseId isMinimized }
          }
        }
      }
    }
  }
}
"""


def fetch_thread_map(repo: str, pr_number: str) -> dict[int, str]:
    """unresolved AND first comment is not minimized 인 thread 의 ``{ first comment id: thread node id }``."""
    owner, name = repo.split("/", 1)
    payload = gh_graphql(
        THREAD_MAP_QUERY, {"owner": owner, "name": name, "pr": int(pr_number)}
    )
    threads = (
        payload.get("data", {})
        .get("repository", {})
        .get("pullRequest", {})
        .get("reviewThreads", {})
        .get("nodes", [])
    )
    out: dict[int, str] = {}
    for thread in threads:
        if thread.get("isResolved"):
            continue
        thread_id = thread.get("id")
        if not thread_id:
            continue
        for comment in (thread.get("comments") or {}).get("nodes") or []:
            if comment.get("isMinimized"):
                continue
            db_id = comment.get("databaseId")
            if db_id is not None:
                out[int(db_id)] = thread_id
    return out


def changed_right_lines(patch: str | None) -> set[int]:
    result: set[int] = set()
    right_line: int | None = None
    for raw in (patch or "").splitlines():
        if raw.startswith("@@"):
            match = re.search(r"\+(\d+)(?:,(\d+))?", raw)
            right_line = int(match.group(1)) if match else None
            continue
        if right_line is None:
            continue
        if raw.startswith("+") and not raw.startswith("+++"):
            result.add(right_line)
            right_line += 1
        elif raw.startswith("-") and not raw.startswith("---"):
            continue
        else:
            right_line += 1
    return result


def write_step_summary(text: str) -> None:
    path = os.environ.get("GITHUB_STEP_SUMMARY")
    if path:
        with open(path, "a", encoding="utf-8") as fp:
            fp.write(text + ("\n" if not text.endswith("\n") else ""))


# ----------------------------------------------------------------------------
# inline 본문 렌더링
# ----------------------------------------------------------------------------


def finding_unique_key(finding: dict) -> str:
    """다음 실행에서 동일 finding 을 PATCH 하기 위한 안정적인 해시 키.

    ``file:line:agent`` 만으로 결정하고 ``title`` / ``reason`` 은 키에 포함하지
    않는다. LLM 출력이 매 실행마다 미묘하게 다른 표현을 쓰더라도 같은 axis 의
    같은 라인에 대한 지적은 한 코멘트로 모이도록 한다 (PATCH 로 본문 갱신).
    """
    raw = "|".join(
        [
            str(finding.get("file") or ""),
            str(finding.get("line") or ""),
            str(finding.get("agent") or ""),
        ]
    )
    return hashlib.sha1(raw.encode("utf-8")).hexdigest()


def render_inline_body(finding: dict) -> tuple[str, str]:
    """(rendered body, unique key) 반환."""
    finding_type = (finding.get("type") or "").upper()
    badge_label, badge_color = TYPE_BADGE.get(finding_type, ("unknown", "lightgrey"))
    axis = finding.get("agent") or ""
    axis_label = AXIS_LABEL_MAP.get(axis, axis)
    title = redact(str(finding.get("title") or "")).strip()
    reason = redact(str(finding.get("reason") or "")).strip()
    key = finding_unique_key(finding)
    badge = (
        f"![{badge_label}-badge]"
        f"(https://img.shields.io/badge/codex--review-{badge_label}-{badge_color}.svg)"
    )
    parts = [
        INLINE_MARKER,
        badge,
        "",
        f"**{axis_label} — {title}**",
        "",
        reason,
        "",
        f"<!-- codex:key:{key} -->",
    ]
    return "\n".join(parts) + "\n", key


def extract_key(body: str) -> str | None:
    match = re.search(r"<!--\s*codex:key:([0-9a-f]+)\s*-->", body or "")
    return match.group(1) if match else None


def extract_type_from_body(body: str) -> str:
    m = TYPE_BADGE_RE.search(body or "")
    return m.group(1).upper() if m else ""


def build_ask_reply_body(codex_reason: str) -> str:
    reason = (codex_reason or "").strip() or "(별도 보충 사유 없음)"
    return (
        "✅ 확인 완료 (close)\n\n"
        f"Codex resolve-check 의 판정: {reason}\n\n"
        "(추가 조치가 불필요하다고 판단해 thread 를 닫고 본 ASK 코멘트를 Hide 처리했습니다. "
        "다시 논의가 필요하면 *Unresolve conversation* 을 눌러 다시 열어주세요.)"
    )


# ----------------------------------------------------------------------------
# inline 처리
# ----------------------------------------------------------------------------


def post_inline(
    allowed: list[dict],
    repo: str,
    pr_number: str,
    head_sha: str,
    resolutions_by_id: dict[int, dict],
    resolved_ids: set[int],
) -> dict:
    current_pr = gh_json(f"repos/{repo}/pulls/{pr_number}")
    current_head = ((current_pr or {}).get("head") or {}).get("sha")
    if current_head != head_sha:
        print(
            f"Skipping Codex inline comments because PR head moved from "
            f"{head_sha} to {current_head}."
        )
        return {
            "created": 0,
            "updated": 0,
            "resolved": 0,
            "skipped": 0,
            "aborted": True,
        }

    files = gh_paginated(f"repos/{repo}/pulls/{pr_number}/files")
    comments = gh_paginated(f"repos/{repo}/pulls/{pr_number}/comments")
    changed_lines_by_path: dict[str, set[int]] = {
        item.get("filename", ""): changed_right_lines(item.get("patch") or "")
        for item in files
    }
    codex_managed = [c for c in comments if is_codex_inline(c)]
    by_key: dict[str, dict] = {}
    for c in codex_managed:
        key = extract_key(c.get("body") or "")
        if key:
            by_key.setdefault(key, c)

    created = updated = resolved = skipped = 0
    kept_ids: set[int] = set()

    for finding in allowed:
        if finding.get("cross_cutting"):
            continue
        path = str(finding.get("file") or "").strip()
        line = int(finding.get("line") or 0)
        if not path or line <= 0:
            skipped += 1
            continue
        allowed_lines = changed_lines_by_path.get(path, set())
        if line not in allowed_lines:
            print(f"Skipping {path}:{line}: not a changed RIGHT-side line")
            skipped += 1
            continue

        body, key = render_inline_body(finding)
        existing = by_key.get(key)
        if existing:
            existing_path = existing.get("path")
            existing_line = int(existing.get("line") or 0)
            existing_side = (
                existing.get("side") or existing.get("original_side") or "RIGHT"
            )
            if (
                existing_path == path
                and existing_line == line
                and existing_side == "RIGHT"
            ):
                gh_json(
                    f"repos/{repo}/pulls/comments/{int(existing['id'])}",
                    method="PATCH",
                    payload={"body": body},
                )
                kept_ids.add(int(existing["id"]))
                updated += 1
                continue
        created_comment = gh_json(
            f"repos/{repo}/pulls/{pr_number}/comments",
            method="POST",
            payload={
                "body": body,
                "commit_id": head_sha,
                "path": path,
                "line": line,
                "side": "RIGHT",
            },
        )
        if created_comment and created_comment.get("id") is not None:
            kept_ids.add(int(created_comment["id"]))
        created += 1

    # Stage 0 의 LLM 판정 결과를 신뢰. resolved=true 로 마킹된 codex inline 만 처리하며,
    # ASK 는 reply (판정 사유 포함) + minimizeComment (Hide) + resolveReviewThread,
    # 그 외는 "수정작업완료" reply + resolveReviewThread. thread 가 이미 resolved /
    # minimized 면 thread_map 에서 빠지므로 자동으로 건너뛴다.
    thread_map: dict[int, str] | None = None
    for c in codex_managed:
        cid = int(c.get("id") or 0)
        if cid <= 0 or cid in kept_ids:
            continue
        if cid not in resolved_ids:
            continue
        if thread_map is None:
            thread_map = fetch_thread_map(repo, pr_number)
        thread_id = thread_map.get(cid)
        if not thread_id:
            continue
        finding_type = extract_type_from_body(c.get("body") or "")
        if finding_type == "ASK":
            codex_reason = (resolutions_by_id.get(cid) or {}).get("reason", "")
            reply_body = build_ask_reply_body(str(codex_reason))
        else:
            reply_body = REPLY_BODY_FIX
        try:
            gh_json(
                f"repos/{repo}/pulls/{pr_number}/comments/{cid}/replies",
                method="POST",
                payload={"body": reply_body},
            )
        except SystemExit as exc:
            print(f"::warning::failed to reply on comment {cid}: {exc}")
            continue
        if finding_type == "ASK":
            node_id = str(c.get("node_id") or "")
            if node_id:
                try:
                    gh_graphql(
                        MINIMIZE_MUTATION,
                        {"id": node_id, "classifier": "RESOLVED"},
                    )
                except SystemExit as exc:
                    print(f"::warning::failed to minimize ASK comment {cid}: {exc}")
        try:
            gh_graphql(RESOLVE_MUTATION, {"id": thread_id})
        except SystemExit as exc:
            print(f"::warning::failed to resolve thread for comment {cid}: {exc}")
            continue
        resolved += 1

    return {
        "created": created,
        "updated": updated,
        "resolved": resolved,
        "skipped": skipped,
        "aborted": False,
    }


# ----------------------------------------------------------------------------
# sticky 종합 코멘트
# ----------------------------------------------------------------------------


def badge_md(finding_type: str) -> str:
    badge_label, badge_color = TYPE_BADGE.get(
        finding_type, ("unknown", "lightgrey")
    )
    return (
        f"![{badge_label}-badge]"
        f"(https://img.shields.io/badge/codex--review-{badge_label}-{badge_color}.svg)"
    )


def render_block(allowed: list[dict], types: set[str], checkbox: bool) -> str:
    rows = []
    for f in allowed:
        if f.get("type") not in types:
            continue
        path = str(f.get("file") or "")
        line = f.get("line")
        loc = f"`{path}:{line}`" if path and line else (f"`{path}`" if path else "")
        title = redact(str(f.get("title") or "")).strip()
        reason = redact(str(f.get("reason") or "")).strip()
        prefix = "- [ ] " if checkbox else "- "
        head = f"{prefix}{badge_md(str(f.get('type') or ''))}"
        if loc:
            head += f" {loc}"
        head += f" — {title}"
        if reason:
            rows.append(f"{head}\n  이유: {reason}")
        else:
            rows.append(head)
    if not rows:
        return "- 해당 없음"
    return "\n".join(rows)


def render_positive(art_dir: Path) -> str:
    seen: list[str] = []
    for path in sorted(art_dir.glob("findings-*.json")):
        try:
            doc = json.loads(path.read_text(encoding="utf-8"))
        except json.JSONDecodeError:
            continue
        for item in doc.get("positive") or []:
            text = redact(str(item)).strip()
            if text and text not in seen:
                seen.append(text)
    if not seen:
        return "- (해당 없음)"
    return "\n".join(f"- {line}" for line in seen[:3])


def render_impact(art_dir: Path) -> str:
    domain_path = art_dir / "findings-domain.json"
    if not domain_path.exists():
        return "- (domain axis 출력 없음)"
    try:
        doc = json.loads(domain_path.read_text(encoding="utf-8"))
    except json.JSONDecodeError:
        return "- (domain axis 출력 파싱 실패)"
    summary = doc.get("impact_summary") or {}
    if not summary:
        return "- (해당 없음)"
    fields = [
        ("scope", "영향 범위"),
        ("backward_compat", "하위 호환성"),
        ("external_integration", "외부 연계"),
        ("env_settings", "환경 설정"),
        ("other_notes", "비고"),
    ]
    parts = []
    for key, label in fields:
        if summary.get(key):
            parts.append(f"- **{label}**: {redact(str(summary[key]))}")
    return "\n".join(parts) if parts else "- (해당 없음)"


def derive_status(counts: dict[str, int], decisions_doc: dict) -> tuple[str, str]:
    judgment = decisions_doc.get("judgment") or {}
    tl_status = str(judgment.get("status") or "")
    tl_headline = str(judgment.get("headline") or "").strip()
    must = counts.get("MUST", 0)
    ask = counts.get("ASK", 0)
    if must > 0:
        detail = (
            f"MUST {must} 건의 대응이 필수 (머지 차단) — 테크리드 코멘트: {tl_headline}"
            if tl_headline
            else f"MUST {must} 건의 대응이 필수 (머지 차단)"
        )
        return "대응 필요", detail
    if tl_status == "NEEDS_WORK":
        return (
            "재검토 필요",
            tl_headline or "설계상 우려가 있어 SUGGEST / IMO 를 확인하세요.",
        )
    if tl_status == "NEEDS_CLARIFICATION":
        return "확인 대기", tl_headline or f"ASK {ask} 건의 의도를 확인한 후 LGTM."
    if ask > 0:
        return "확인 대기", tl_headline or f"ASK {ask} 건의 의도를 확인한 후 LGTM."
    if tl_headline:
        return "LGTM", tl_headline
    optional_total = sum(counts.get(k, 0) for k in ("SUGGEST", "IMO", "NITS"))
    if optional_total > 0:
        return "LGTM", "SUGGEST / IMO / NITS 는 임의 대응."
    return "LGTM", "지적 사항 없음."


def post_sticky(
    allowed: list[dict],
    decisions_doc: dict,
    art_dir: Path,
    template_path: Path,
    repo: str,
    pr_number: str,
    trigger: str,
) -> None:
    counts = {key: 0 for key in TYPE_BADGE}
    for f in allowed:
        t = (f.get("type") or "").upper()
        if t in counts:
            counts[t] += 1

    status, detail = derive_status(counts, decisions_doc)

    template_text = template_path.read_text(encoding="utf-8")
    substitutions = {
        "PR_NUMBER": pr_number,
        "TRIGGER": trigger,
        "LGTM_STATUS": status,
        "LGTM_DETAIL": detail,
        "MUST_COUNT": str(counts.get("MUST", 0)),
        "ASK_COUNT": str(counts.get("ASK", 0)),
        "SUGGEST_COUNT": str(counts.get("SUGGEST", 0)),
        "IMO_COUNT": str(counts.get("IMO", 0)),
        "NITS_COUNT": str(counts.get("NITS", 0)),
        "MUST_BLOCK": render_block(allowed, {"MUST"}, checkbox=True),
        "ASK_BLOCK": render_block(allowed, {"ASK"}, checkbox=True),
        "SUGGEST_IMO_BLOCK": render_block(
            allowed, {"SUGGEST", "IMO"}, checkbox=False
        ),
        "NITS_BLOCK": render_block(allowed, {"NITS"}, checkbox=False),
        "POSITIVE_BLOCK": render_positive(art_dir),
        "IMPACT_BLOCK": render_impact(art_dir),
    }
    body = Template(template_text).safe_substitute(substitutions)

    sentinels = [
        STICKY_MARKER,
        "## Codex AI 리뷰",
        "### 결과 요약",
        "### MUST",
        "### ASK",
        "### SUGGEST / IMO",
        "### NITS",
        "### 영향 범위",
    ]
    for sentinel in sentinels:
        if sentinel not in body:
            raise SystemExit(f"sticky body missing sentinel: {sentinel}")

    existing = gh_paginated(f"repos/{repo}/issues/{pr_number}/comments")
    existing_sticky_id: int | None = None
    for comment in existing:
        if (comment.get("user") or {}).get("login") != BOT_LOGIN:
            continue
        body_text = comment.get("body") or ""
        if body_text.startswith(STICKY_MARKER):
            existing_sticky_id = int(comment.get("id") or 0)
            break

    if existing_sticky_id:
        print(f"PATCH existing sticky comment id={existing_sticky_id}")
        gh_json(
            f"repos/{repo}/issues/comments/{existing_sticky_id}",
            method="PATCH",
            payload={"body": body},
        )
    else:
        print("POST new sticky summary comment")
        gh_json(
            f"repos/{repo}/issues/{pr_number}/comments",
            method="POST",
            payload={"body": body},
        )


# ----------------------------------------------------------------------------
# 진입점
# ----------------------------------------------------------------------------


def main() -> int:
    repo = os.environ["GITHUB_REPOSITORY"]
    pr_number = os.environ["PR_NUMBER"]
    head_sha = os.environ["HEAD_SHA"]
    trigger = os.environ.get("TRIGGER", "unknown")
    art_dir = Path(os.environ.get("ART_DIR", "./artifacts"))
    template_path = Path(
        os.environ.get("TEMPLATE", ".github/scripts/review-summary-template.md")
    )

    allowed_path = art_dir / "allowed.json"
    decisions_path = art_dir / "decisions.json"
    if not allowed_path.exists():
        print(f"::warning::{allowed_path} missing; nothing to post")
        return 0

    allowed = json.loads(allowed_path.read_text(encoding="utf-8"))
    decisions_doc = (
        json.loads(decisions_path.read_text(encoding="utf-8"))
        if decisions_path.exists()
        else {"decisions": [], "merge_notes": []}
    )

    resolutions_path = art_dir / "resolutions.json"
    resolved_ids: set[int] = set()
    resolutions_by_id: dict[int, dict] = {}
    if resolutions_path.is_file():
        try:
            doc = json.loads(resolutions_path.read_text(encoding="utf-8"))
            for row in doc.get("resolutions") or []:
                try:
                    cid = int(row.get("comment_id") or 0)
                except (TypeError, ValueError):
                    continue
                if cid <= 0:
                    continue
                resolutions_by_id[cid] = row
                if row.get("resolved"):
                    resolved_ids.add(cid)
        except json.JSONDecodeError:
            print("::warning::failed to parse resolutions.json")

    inline_result = post_inline(
        allowed, repo, pr_number, head_sha, resolutions_by_id, resolved_ids
    )
    if inline_result.get("aborted"):
        print("::warning::Inline posting aborted because PR head advanced.")
        return 0

    summary_line = (
        f"인라인 — created {inline_result['created']}, "
        f"updated {inline_result['updated']}, "
        f"resolved {inline_result['resolved']}, "
        f"skipped {inline_result['skipped']}."
    )
    print(summary_line)
    write_step_summary(summary_line)

    post_sticky(
        allowed, decisions_doc, art_dir, template_path, repo, pr_number, trigger
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
