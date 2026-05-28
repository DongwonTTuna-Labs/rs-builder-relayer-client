#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import re
import sys
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any


AXES = ("correctness", "security", "performance", "test-coverage", "domain")
INLINE_MARKER = "<!-- codex-review-inline -->"
REVIEW_MARKER = "<!-- codex-review -->"
RESOLVE_MARKER = "<!-- codex-resolve-check -->"
DESIGN_MARKER = "<!-- codex-design-plan -->"
MAX_INLINE_COMMENTS = 50
RESOLVE_BATCH_SIZE = 3
RESOLVE_SEARCH_CONTEXT_RADIUS = 6
RESOLVE_SEARCH_MAX_TERMS = 8
RESOLVE_SEARCH_MAX_MATCHES = 3
REVIEW_CONTEXT_MAX_CHARS = 60000
REVIEW_CONTEXT_SECTION_LIMIT = 12000
DESIGN_PLAN_SUMMARY_LIMIT = 800
DESIGN_PLAN_ROOT_CAUSE_LIMIT = 1200
DESIGN_PLAN_ITEM_LIMIT = 600
TRUSTED_USER = "DongwonTTuna"
TRUSTED_CODEX_REVIEW_AUTHORS = ("codex-reviewer-for-dongwonttuna",)


def require_env(name: str) -> str:
    value = os.environ.get(name, "").strip()
    if not value:
        raise SystemExit(f"{name} is required")
    return value


def trim_text(value: Any, limit: int) -> str:
    text = "" if value is None else str(value)
    return text if len(text) <= limit else text[:limit] + "\n...[truncated]"


def is_trusted_codex_review_author(author: str) -> bool:
    return author in TRUSTED_CODEX_REVIEW_AUTHORS or author.endswith("[bot]")


def github_api(path: str, *, method: str = "GET", payload: dict[str, Any] | None = None) -> Any:
    data = None
    headers = {
        "Accept": "application/vnd.github+json",
        "Authorization": f"Bearer {require_env('GH_TOKEN')}",
        "X-GitHub-Api-Version": "2022-11-28",
        "User-Agent": "codex-review",
    }
    if payload is not None:
        data = json.dumps(payload).encode("utf-8")
        headers["Content-Type"] = "application/json"
    request = urllib.request.Request(f"https://api.github.com{path}", data=data, headers=headers, method=method)
    try:
        with urllib.request.urlopen(request, timeout=60) as response:
            body = response.read().decode("utf-8", "replace")
            return json.loads(body) if body else None
    except urllib.error.HTTPError as exc:
        body = exc.read().decode("utf-8", "replace")
        raise SystemExit(f"{method} {path} failed with {exc.code}: {body[:800]}") from exc


def github_paginated(path: str) -> list[Any]:
    items: list[Any] = []
    next_path: str | None = path
    while next_path:
        request = urllib.request.Request(
            f"https://api.github.com{next_path}",
            headers={
                "Accept": "application/vnd.github+json",
                "Authorization": f"Bearer {require_env('GH_TOKEN')}",
                "X-GitHub-Api-Version": "2022-11-28",
                "User-Agent": "codex-review",
            },
        )
        try:
            with urllib.request.urlopen(request, timeout=60) as response:
                items.extend(json.loads(response.read().decode("utf-8", "replace")))
                link = response.headers.get("Link", "")
        except urllib.error.HTTPError as exc:
            body = exc.read().decode("utf-8", "replace")
            raise SystemExit(f"GET {next_path} failed with {exc.code}: {body[:800]}") from exc
        next_path = parse_next_path(link)
    return items


def github_graphql(query: str, variables: dict[str, Any] | None = None) -> dict[str, Any]:
    payload = {"query": query, "variables": variables or {}}
    data = json.dumps(payload).encode("utf-8")
    request = urllib.request.Request(
        "https://api.github.com/graphql",
        data=data,
        headers={
            "Accept": "application/vnd.github+json",
            "Authorization": f"Bearer {require_env('GH_TOKEN')}",
            "X-GitHub-Api-Version": "2022-11-28",
            "User-Agent": "codex-review",
            "Content-Type": "application/json",
        },
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=60) as response:
            result = json.loads(response.read().decode("utf-8", "replace"))
    except urllib.error.HTTPError as exc:
        body = exc.read().decode("utf-8", "replace")
        raise SystemExit(f"GraphQL failed with {exc.code}: {body[:800]}") from exc
    if result.get("errors"):
        raise SystemExit("GraphQL errors: " + json.dumps(result["errors"], ensure_ascii=False)[:1200])
    return result["data"]


def parse_next_path(link: str) -> str | None:
    for part in link.split(","):
        if 'rel="next"' not in part:
            continue
        match = re.search(r"<https://api.github.com([^>]+)>", part)
        if match:
            return match.group(1)
    return None


def changed_right_lines(patch: str | None) -> list[int]:
    result: list[int] = []
    right_line: int | None = None
    for raw_line in (patch or "").splitlines():
        if raw_line.startswith("@@"):
            match = re.search(r"\+(\d+)(?:,(\d+))?", raw_line)
            right_line = int(match.group(1)) if match else None
            continue
        if right_line is None:
            continue
        if raw_line.startswith("+") and not raw_line.startswith("+++"):
            result.append(right_line)
            right_line += 1
        elif raw_line.startswith("-") and not raw_line.startswith("---"):
            continue
        else:
            right_line += 1
    return result


def build_changed_line_map(repo: str, pr_number: str) -> dict[str, set[int]]:
    files = github_paginated(f"/repos/{repo}/pulls/{pr_number}/files?per_page=100")
    return {
        str(item.get("filename")): set(changed_right_lines(item.get("patch")))
        for item in files
        if item.get("filename")
    }


def write_github_output(values: dict[str, str]) -> None:
    output_path = os.environ.get("GITHUB_OUTPUT")
    if not output_path:
        for key, value in values.items():
            print(f"{key}={value}")
        return
    with open(output_path, "a", encoding="utf-8") as output:
        for key, value in values.items():
            output.write(f"{key}={value}\n")


def skipped_current_review() -> dict[str, str]:
    return {
        "should_run": "false",
        "pr_number": "",
        "head_sha": "",
        "base_ref": "",
        "base_sha": "",
        "trigger": "",
    }


def skipped_resolve_checker() -> dict[str, str]:
    return {"should_collect": "false", "pr_number": "", "head_sha": "", "base_ref": "", "base_sha": ""}


def resolve_current_review_event(
    *,
    event_name: str,
    event: dict[str, Any],
    repo: str,
    actor: str,
    triggering_actor: str,
    fetch_pr: Any = github_api,
) -> dict[str, str]:
    if actor != TRUSTED_USER or triggering_actor != TRUSTED_USER:
        return skipped_current_review()

    if event_name in {"pull_request", "pull_request_target"}:
        pr = event.get("pull_request") or {}
        sender = (event.get("sender") or {}).get("login")
        base_ref = ((pr.get("base") or {}).get("ref")) or ""
        head_repo = ((pr.get("head") or {}).get("repo") or {}).get("full_name")
        author = (pr.get("user") or {}).get("login")
        if (
            not pr
            or pr.get("draft")
            or base_ref != "main"
            or head_repo != repo
            or author != TRUSTED_USER
            or sender != TRUSTED_USER
        ):
            return skipped_current_review()
        return {
            "should_run": "true",
            "pr_number": str(pr["number"]),
            "head_sha": str(pr["head"]["sha"]),
            "base_ref": "main",
            "base_sha": str(pr["base"]["sha"]),
            "trigger": f"{event_name}:{event.get('action', '')}",
        }

    if event_name == "issue_comment":
        issue = event.get("issue") or {}
        comment = event.get("comment") or {}
        body = str(comment.get("body") or "")
        comment_user = ((comment.get("user") or {}).get("login")) or ""
        if "pull_request" not in issue or "/codex-review" not in body or comment_user != TRUSTED_USER or actor.endswith("[bot]"):
            return skipped_current_review()
        pr_number = str(issue["number"])
        pr = fetch_pr(f"/repos/{repo}/pulls/{pr_number}")
        base_ref = ((pr.get("base") or {}).get("ref")) or ""
        head_repo = ((pr.get("head") or {}).get("repo") or {}).get("full_name")
        author = (pr.get("user") or {}).get("login")
        if base_ref != "main" or head_repo != repo or author != TRUSTED_USER:
            return skipped_current_review()
        return {
            "should_run": "true",
            "pr_number": pr_number,
            "head_sha": str(pr["head"]["sha"]),
            "base_ref": "main",
            "base_sha": str(pr["base"]["sha"]),
            "trigger": "issue_comment:/codex-review",
        }

    return skipped_current_review()


def resolve_previous_review_event(
    *,
    event_name: str,
    event: dict[str, Any],
    repo: str,
    actor: str,
    triggering_actor: str,
) -> dict[str, str]:
    if actor != TRUSTED_USER or triggering_actor != TRUSTED_USER or event_name not in {"pull_request", "pull_request_target"}:
        return skipped_resolve_checker()
    pr = event.get("pull_request") or {}
    base_ref = ((pr.get("base") or {}).get("ref")) or ""
    head_repo = ((pr.get("head") or {}).get("repo") or {}).get("full_name")
    author = (pr.get("user") or {}).get("login")
    if not pr or pr.get("draft") or base_ref != "main" or head_repo != repo or author != TRUSTED_USER:
        return skipped_resolve_checker()
    return {
        "should_collect": "true",
        "pr_number": str(pr["number"]),
        "head_sha": str(pr["head"]["sha"]),
        "base_ref": "main",
        "base_sha": str(pr["base"]["sha"]),
    }


def load_event_payload() -> dict[str, Any]:
    return json.loads(Path(require_env("GITHUB_EVENT_PATH")).read_text(encoding="utf-8"))


def command_resolve_current(args: argparse.Namespace) -> None:
    del args
    write_github_output(
        resolve_current_review_event(
            event_name=os.environ.get("GITHUB_EVENT_NAME", ""),
            event=load_event_payload(),
            repo=require_env("GITHUB_REPOSITORY"),
            actor=os.environ.get("GITHUB_ACTOR", ""),
            triggering_actor=os.environ.get("GITHUB_TRIGGERING_ACTOR", ""),
        )
    )


def command_resolve_previous(args: argparse.Namespace) -> None:
    del args
    write_github_output(
        resolve_previous_review_event(
            event_name=os.environ.get("GITHUB_EVENT_NAME", ""),
            event=load_event_payload(),
            repo=require_env("GITHUB_REPOSITORY"),
            actor=os.environ.get("GITHUB_ACTOR", ""),
            triggering_actor=os.environ.get("GITHUB_TRIGGERING_ACTOR", ""),
        )
    )


def load_current_findings(artifacts: Path) -> list[dict[str, Any]]:
    findings: list[dict[str, Any]] = []
    missing: list[str] = []
    for axis in AXES:
        path = artifacts / f"findings-{axis}.json"
        if not path.exists():
            missing.append(axis)
            continue
        payload = json.loads(path.read_text(encoding="utf-8"))
        if payload.get("agent") != axis:
            raise SystemExit(f"{path} has wrong agent: {payload.get('agent')}")
        axis_findings = payload.get("findings")
        if not isinstance(axis_findings, list):
            raise SystemExit(f"{path} has invalid findings")
        for item in axis_findings:
            findings.append(normalize_finding(axis, item))
    if missing:
        raise SystemExit("missing reviewer artifacts: " + ", ".join(missing))
    return findings


def normalize_finding(axis: str, item: Any) -> dict[str, Any]:
    if not isinstance(item, dict):
        raise SystemExit(f"{axis} finding must be an object")
    finding_id = str(item.get("id") or "").strip()
    if not re.match(r"^(correctness|security|performance|test-coverage|domain)-[0-9]+$", finding_id):
        raise SystemExit(f"{axis} finding has invalid id: {finding_id}")
    finding_type = str(item.get("type") or "SUGGEST").upper()
    if finding_type not in {"MUST", "SUGGEST", "IMO", "NITS", "ASK"}:
        finding_type = "SUGGEST"
    line = item.get("line")
    if line is not None:
        try:
            line = int(line)
        except (TypeError, ValueError) as exc:
            raise SystemExit(f"{finding_id} has invalid line") from exc
        if line < 1:
            line = None
    file_path = item.get("file")
    rule_ref = item.get("rule_ref")
    return {
        "id": finding_id,
        "agent": axis,
        "type": finding_type,
        "file": str(file_path) if file_path else None,
        "line": line,
        "title": trim_text(item.get("title"), 200).strip() or "Review finding",
        "reason": trim_text(item.get("reason"), 1000).strip() or "No detail provided.",
        "rule_ref": str(rule_ref) if rule_ref else None,
        "cross_cutting": bool(item.get("cross_cutting")),
    }


def load_decisions(path: Path) -> dict[str, Any]:
    payload = json.loads(path.read_text(encoding="utf-8"))
    decisions = payload.get("decisions")
    if not isinstance(decisions, list):
        raise SystemExit("tech-lead decisions must be an array")
    by_id: dict[str, dict[str, Any]] = {}
    for decision in decisions:
        if not isinstance(decision, dict):
            continue
        decision_id = str(decision.get("id") or "")
        if decision_id:
            by_id[decision_id] = {
                "allow": bool(decision.get("allow")),
                "reason": trim_text(decision.get("reason"), 300).strip() or "No decision reason provided.",
            }
    judgment = payload.get("judgment") if isinstance(payload.get("judgment"), dict) else None
    merge_notes = payload.get("merge_notes") if isinstance(payload.get("merge_notes"), list) else []
    return {"by_id": by_id, "judgment": judgment, "merge_notes": merge_notes}


def hard_allow(finding: dict[str, Any]) -> bool:
    rule_ref = (finding.get("rule_ref") or "").lower()
    return finding["type"] == "MUST" or finding["agent"] == "security" or "critical" in rule_ref


def hard_block(finding: dict[str, Any]) -> bool:
    return hard_allow(finding)


def render_current_inline(finding: dict[str, Any], decision: dict[str, Any] | None) -> str:
    lines = [
        INLINE_MARKER,
        f"<!-- codex-review-id: {finding['id']} -->",
        f"**[{finding['type']}][{finding['agent']}] {finding['title']}**",
        "",
        finding["reason"],
    ]
    if decision:
        lines.extend(["", f"테크리드: {decision['reason']}"])
    return "\n".join(lines)


def render_current_body(
    *,
    event: str,
    allowed: list[tuple[dict[str, Any], dict[str, Any] | None]],
    denied_count: int,
    unplaced: list[tuple[dict[str, Any], dict[str, Any] | None]],
    decisions: dict[str, Any],
) -> str:
    judgment = decisions.get("judgment") or {}
    lines = [
        "<!-- codex-review -->",
        "Codex 리뷰가 완료되었습니다.",
        "",
        f"- 이벤트: {event}",
        f"- 게시한 지적: {len(allowed)}",
        f"- 테크리드가 필터링한 지적: {denied_count}",
    ]
    if judgment:
        lines.extend(
            [
                f"- 테크리드 상태: {judgment.get('status', 'UNKNOWN')}",
                f"- 테크리드 요약: {judgment.get('headline', '')}",
            ]
        )
    if unplaced:
        lines.extend(["", "위치에 직접 달지 못한 지적:"])
        for finding, decision in unplaced[:25]:
            location = finding.get("file") or "일반"
            if finding.get("line"):
                location = f"{location}:{finding['line']}"
            suffix = f" 테크리드: {decision['reason']}" if decision else ""
            lines.append(
                f"- [{finding['type']}][{finding['agent']}] {finding['id']} {location} - "
                f"{finding['title']}: {finding['reason']}{suffix}"
            )
    merge_notes = decisions.get("merge_notes") or []
    if merge_notes:
        lines.extend(["", "병합 메모:"])
        for note in merge_notes[:10]:
            lines.append(
                f"- {note.get('primary_id')}: 병합됨 {', '.join(note.get('merged_ids') or [])} - "
                f"{note.get('reason', '')}"
            )
    return "\n".join(lines)


def command_post_current(args: argparse.Namespace) -> None:
    repo = require_env("GITHUB_REPOSITORY")
    pr_number = require_env("PR_NUMBER")
    head_sha = require_env("HEAD_SHA")
    findings = load_current_findings(Path(args.artifacts))
    decisions = load_decisions(Path(args.decisions))
    changed_by_file = build_changed_line_map(repo, pr_number)

    allowed: list[tuple[dict[str, Any], dict[str, Any] | None]] = []
    denied_count = 0
    for finding in findings:
        decision = decisions["by_id"].get(finding["id"])
        allow = hard_allow(finding) or (decision["allow"] if decision else False)
        if allow:
            allowed.append((finding, decision))
        else:
            denied_count += 1

    comments: list[dict[str, Any]] = []
    unplaced: list[tuple[dict[str, Any], dict[str, Any] | None]] = []
    for finding, decision in allowed:
        file_path = finding["file"]
        line = finding["line"]
        if (
            isinstance(file_path, str)
            and isinstance(line, int)
            and line in changed_by_file.get(file_path, set())
            and len(comments) < MAX_INLINE_COMMENTS
        ):
            comments.append(
                {
                    "path": file_path,
                    "line": line,
                    "side": "RIGHT",
                    "body": render_current_inline(finding, decision),
                }
            )
        else:
            unplaced.append((finding, decision))

    judgment = decisions.get("judgment") or {}
    blocking = any(hard_block(finding) for finding, _ in allowed)
    event = "REQUEST_CHANGES" if blocking or judgment.get("status") == "NEEDS_WORK" else "COMMENT"
    payload = {
        "commit_id": head_sha,
        "event": event,
        "body": render_current_body(
            event=event,
            allowed=allowed,
            denied_count=denied_count,
            unplaced=unplaced,
            decisions=decisions,
        ),
        "comments": comments,
    }
    github_api(f"/repos/{repo}/pulls/{pr_number}/reviews", method="POST", payload=payload)
    print(f"posted {event} review with {len(comments)} inline comments and {len(unplaced)} unplaced findings")


def collect_review_threads(repo: str, pr_number: str) -> list[dict[str, Any]]:
    owner, name = repo.split("/", 1)
    query = """
    query($owner: String!, $name: String!, $number: Int!, $cursor: String) {
      repository(owner: $owner, name: $name) {
        pullRequest(number: $number) {
          reviewThreads(first: 100, after: $cursor) {
            pageInfo { hasNextPage endCursor }
            nodes {
              id
              isResolved
              isOutdated
              path
              line
              originalLine
              comments(first: 50) {
                nodes {
                  id
                  fullDatabaseId
                  body
                  author { login }
                  url
                  createdAt
                  path
                  line
                  originalLine
                  outdated
                  commit { oid }
                  originalCommit { oid }
                }
              }
            }
          }
        }
      }
    }
    """
    threads: list[dict[str, Any]] = []
    cursor: str | None = None
    while True:
        data = github_graphql(
            query,
            {"owner": owner, "name": name, "number": int(pr_number), "cursor": cursor},
        )
        conn = data["repository"]["pullRequest"]["reviewThreads"]
        threads.extend(conn["nodes"])
        if not conn["pageInfo"]["hasNextPage"]:
            return threads
        cursor = conn["pageInfo"]["endCursor"]


def redact_secrets(body: str) -> str:
    patterns = [
        r"-----BEGIN [A-Z ]*PRIVATE KEY-----.*?-----END [A-Z ]*PRIVATE KEY-----",
        r"\bgh[opsru]_[A-Za-z0-9_]{20,}\b",
        r"\bsk-[A-Za-z0-9_-]{20,}\b",
        r"\bsk-clb-[A-Za-z0-9_-]{20,}\b",
    ]
    redacted = body
    for pattern in patterns:
        redacted = re.sub(pattern, "[redacted]", redacted, flags=re.DOTALL)
    return redacted


def redact_comment_body(body: str) -> str:
    return trim_text(redact_secrets(body), 1200)


def extract_marker_key(body: str) -> str | None:
    match = re.search(r"<!--\s*codex-review-id:\s*([^>]+?)\s*-->", body)
    return match.group(1).strip() if match else None


def comment_commit_oid(comment: dict[str, Any]) -> str:
    return str(((comment.get("commit") or {}).get("oid")) or "")


def comment_original_commit_oid(comment: dict[str, Any]) -> str:
    return str(((comment.get("originalCommit") or {}).get("oid")) or "")


def is_current_head_inline_comment(comment: dict[str, Any], head_sha: str) -> bool:
    original_oid = comment_original_commit_oid(comment)
    if original_oid:
        return original_oid == head_sha
    return comment_commit_oid(comment) == head_sha


def code_snippet(workspace: Path, file_path: str | None, line: int | None) -> str | None:
    if not file_path or line is None:
        return None
    path = (workspace / file_path).resolve()
    try:
        path.relative_to(workspace.resolve())
    except ValueError:
        return None
    if not path.exists() or not path.is_file():
        return None
    lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
    if line < 1 or line > len(lines):
        return None
    start = max(1, line - 15)
    end = min(len(lines), line + 15)
    width = len(str(end))
    return "\n".join(f"{idx:>{width}}: {lines[idx - 1]}" for idx in range(start, end + 1))


def format_line_snippet(lines: list[str], line: int, radius: int) -> str:
    start = max(1, line - radius)
    end = min(len(lines), line + radius)
    width = len(str(end))
    return "\n".join(f"{idx:>{width}}: {lines[idx - 1]}" for idx in range(start, end + 1))


def add_search_term(terms: list[str], seen: set[str], value: str) -> None:
    term = value.strip()
    if not (3 <= len(term) <= 120) or "\n" in term:
        return
    if not re.search(r"[A-Za-z0-9_]", term):
        return
    key = term.lower()
    if key in seen:
        return
    terms.append(term)
    seen.add(key)


def resolve_search_terms(body: str) -> list[str]:
    terms: list[str] = []
    seen: set[str] = set()
    redacted = redact_comment_body(body)
    for span in re.findall(r"`([^`\n]{3,120})`", redacted):
        add_search_term(terms, seen, span)
        for token in re.findall(r"[A-Za-z_][A-Za-z0-9_]{2,}", span):
            add_search_term(terms, seen, token)
            if len(terms) >= RESOLVE_SEARCH_MAX_TERMS:
                return terms
        if len(terms) >= RESOLVE_SEARCH_MAX_TERMS:
            return terms
    return terms


def search_current_context(workspace: Path, file_path: str | None, body: str) -> list[dict[str, Any]]:
    if not file_path:
        return []
    path = (workspace / file_path).resolve()
    try:
        path.relative_to(workspace.resolve())
    except ValueError:
        return []
    if not path.exists() or not path.is_file():
        return []
    lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
    contexts: list[dict[str, Any]] = []
    seen_ranges: set[tuple[int, int]] = set()
    for term in resolve_search_terms(body):
        needle = term.lower()
        for index, line in enumerate(lines, start=1):
            if needle not in line.lower():
                continue
            start = max(1, index - RESOLVE_SEARCH_CONTEXT_RADIUS)
            end = min(len(lines), index + RESOLVE_SEARCH_CONTEXT_RADIUS)
            range_key = (start, end)
            if range_key in seen_ranges:
                continue
            contexts.append(
                {
                    "term": term,
                    "line": index,
                    "snippet": format_line_snippet(lines, index, RESOLVE_SEARCH_CONTEXT_RADIUS),
                }
            )
            seen_ranges.add(range_key)
            if len(contexts) >= RESOLVE_SEARCH_MAX_MATCHES:
                return contexts
            break
    return contexts


def build_resolve_item(thread: dict[str, Any], comment: dict[str, Any], workspace: Path) -> dict[str, Any]:
    line = comment.get("line") or thread.get("line") or comment.get("originalLine") or thread.get("originalLine")
    try:
        line_int = int(line) if line is not None else None
    except (TypeError, ValueError):
        line_int = None
    file_path = comment.get("path") or thread.get("path")
    body = comment.get("body") or ""
    outdated = bool(thread.get("isOutdated") or comment.get("outdated"))
    return {
        "thread_id": thread["id"],
        "comment_node_id": comment["id"],
        "comment_id": int(comment["fullDatabaseId"]),
        "file": file_path,
        "line": line_int,
        "outdated": outdated,
        "marker_key": extract_marker_key(body),
        "current_commit_oid": comment_commit_oid(comment) or None,
        "original_commit_oid": comment_original_commit_oid(comment) or None,
        "body_excerpt": redact_comment_body(body),
        "code_snippet": code_snippet(workspace, file_path, line_int),
        "search_context": search_current_context(workspace, file_path, body) if outdated else [],
        "url": comment.get("url"),
    }


def command_collect_resolutions(args: argparse.Namespace) -> None:
    repo = require_env("GITHUB_REPOSITORY")
    pr_number = require_env("PR_NUMBER")
    head_sha = require_env("HEAD_SHA")
    workspace = Path(args.workspace)
    batch_dir = Path(args.batch_dir)
    batch_dir.mkdir(parents=True, exist_ok=True)
    for stale_batch in batch_dir.glob("resolve-batch-*.json"):
        stale_batch.unlink()

    items: list[dict[str, Any]] = []
    for thread in collect_review_threads(repo, pr_number):
        if thread.get("isResolved"):
            continue
        for comment in (thread.get("comments") or {}).get("nodes") or []:
            body = comment.get("body") or ""
            author = ((comment.get("author") or {}).get("login")) or ""
            if INLINE_MARKER not in body:
                continue
            if not is_trusted_codex_review_author(author):
                continue
            if is_current_head_inline_comment(comment, head_sha):
                continue
            items.append(build_resolve_item(thread, comment, workspace))

    if not items:
        write_github_output({"has_comments": "false", "batch_indexes": "[]"})
        print("no previous Codex inline comments to resolve")
        return

    batch_indexes: list[int] = []
    for index, start in enumerate(range(0, len(items), RESOLVE_BATCH_SIZE)):
        batch = {"comments": items[start : start + RESOLVE_BATCH_SIZE]}
        (batch_dir / f"resolve-batch-{index}.json").write_text(
            json.dumps(batch, ensure_ascii=False, indent=2) + "\n",
            encoding="utf-8",
        )
        batch_indexes.append(index)
    write_github_output({"has_comments": "true", "batch_indexes": json.dumps(batch_indexes)})
    print(f"collected {len(items)} previous Codex inline comments in {len(batch_indexes)} batches")


def load_resolution_inputs(batches: Path) -> dict[int, dict[str, Any]]:
    comments: dict[int, dict[str, Any]] = {}
    for path in sorted(batches.glob("resolve-batch-*.json")):
        payload = json.loads(path.read_text(encoding="utf-8"))
        for comment in payload.get("comments") or []:
            comment_id = int(comment["comment_id"])
            comments[comment_id] = comment
    if not comments:
        raise SystemExit("no resolve-check comments found")
    return comments


def load_resolution_outputs(results: Path, expected_ids: set[int]) -> dict[int, dict[str, Any]]:
    resolutions: dict[int, dict[str, Any]] = {}
    for path in sorted(results.glob("resolutions-*.json")):
        payload = json.loads(path.read_text(encoding="utf-8"))
        for item in payload.get("resolutions") or []:
            comment_id = int(item["comment_id"])
            resolutions[comment_id] = {
                "resolved": bool(item.get("resolved")),
                "reason": trim_text(item.get("reason"), 300).strip() or "No reason provided.",
            }
    actual_ids = set(resolutions)
    if actual_ids != expected_ids:
        raise SystemExit(f"resolution ids mismatch: expected {sorted(expected_ids)}, got {sorted(actual_ids)}")
    return resolutions


def resolve_thread(thread_id: str) -> None:
    mutation = """
    mutation($threadId: ID!) {
      resolveReviewThread(input: {threadId: $threadId}) {
        thread { id isResolved }
      }
    }
    """
    github_graphql(mutation, {"threadId": thread_id})


def render_resolution_body(
    *,
    event: str,
    resolved: list[tuple[dict[str, Any], dict[str, Any]]],
    unresolved: list[tuple[dict[str, Any], dict[str, Any]]],
) -> str:
    lines = [
        "<!-- codex-resolve-check -->",
        "Codex 해결 여부 확인이 완료되었습니다.",
        "",
        f"- 이벤트: {event}",
        f"- 해결된 스레드: {len(resolved)}",
        f"- 아직 미해결: {len(unresolved)}",
    ]
    if unresolved:
        lines.extend(["", "아직 미해결:"])
        for comment, resolution in unresolved[:25]:
            location = comment.get("file") or "일반"
            if comment.get("line"):
                location = f"{location}:{comment['line']}"
            url = comment.get("url") or ""
            lines.append(f"- {location} - {resolution['reason']} {url}".rstrip())
    if resolved:
        lines.extend(["", "이번에 해결됨:"])
        for comment, resolution in resolved[:25]:
            location = comment.get("file") or "일반"
            if comment.get("line"):
                location = f"{location}:{comment['line']}"
            lines.append(f"- {location} - {resolution['reason']}")
    return "\n".join(lines)


def command_apply_resolutions(args: argparse.Namespace) -> None:
    repo = require_env("GITHUB_REPOSITORY")
    pr_number = require_env("PR_NUMBER")
    head_sha = require_env("HEAD_SHA")
    comments = load_resolution_inputs(Path(args.batches))
    resolutions = load_resolution_outputs(Path(args.results), set(comments))

    resolved: list[tuple[dict[str, Any], dict[str, Any]]] = []
    unresolved: list[tuple[dict[str, Any], dict[str, Any]]] = []
    resolved_threads: set[str] = set()
    for comment_id, comment in comments.items():
        resolution = resolutions[comment_id]
        if resolution["resolved"]:
            thread_id = str(comment["thread_id"])
            if thread_id not in resolved_threads:
                resolve_thread(thread_id)
                resolved_threads.add(thread_id)
            resolved.append((comment, resolution))
        else:
            unresolved.append((comment, resolution))

    event = "REQUEST_CHANGES" if unresolved else "COMMENT"
    payload = {
        "commit_id": head_sha,
        "event": event,
        "body": render_resolution_body(event=event, resolved=resolved, unresolved=unresolved),
    }
    github_api(f"/repos/{repo}/pulls/{pr_number}/reviews", method="POST", payload=payload)
    print(f"posted {event} resolve-check review; resolved={len(resolved)} unresolved={len(unresolved)}")


def latest_marker_comment(comments: list[dict[str, Any]], marker: str) -> dict[str, Any] | None:
    matches = [comment for comment in comments if marker in str(comment.get("body") or "")]
    if not matches:
        return None
    return sorted(matches, key=lambda item: str(item.get("updated_at") or item.get("created_at") or ""))[-1]


def review_marker_kind(body: str) -> str | None:
    if REVIEW_MARKER in body:
        return "review"
    if RESOLVE_MARKER in body:
        return "resolve"
    return None


def thread_category(thread: dict[str, Any], comment: dict[str, Any] | None = None) -> str:
    body = str((comment or {}).get("body") or "")
    marker_key = extract_marker_key(body)
    if marker_key and "-" in marker_key:
        return marker_key.rsplit("-", 1)[0]
    path = str((comment or {}).get("path") or thread.get("path") or "")
    if path.startswith("src/deposit_wallet/") or "deposit_wallet" in path:
        return "deposit-wallet"
    if path.startswith(".github/"):
        return "workflow"
    if path.startswith("docs/"):
        return "docs"
    if path:
        return path.split("/", 1)[0]
    return "general"


def unresolved_thread_summaries(threads: list[dict[str, Any]]) -> dict[str, list[str]]:
    grouped: dict[str, list[str]] = {}
    for thread in threads:
        if thread.get("isResolved"):
            continue
        nodes = (thread.get("comments") or {}).get("nodes") or []
        if not nodes:
            continue
        codex_comments = [
            item
            for item in nodes
            if INLINE_MARKER in str(item.get("body") or "")
            and is_trusted_codex_review_author(((item.get("author") or {}).get("login")) or "")
        ]
        comment = codex_comments[-1] if codex_comments else nodes[-1]
        category = thread_category(thread, comment)
        location = str(comment.get("path") or thread.get("path") or "general")
        line = comment.get("line") or thread.get("line") or comment.get("originalLine") or thread.get("originalLine")
        if line:
            location = f"{location}:{line}"
        marker_key = extract_marker_key(str(comment.get("body") or ""))
        metadata = []
        if marker_key:
            metadata.append(f"id={marker_key}")
        if thread.get("isOutdated") or comment.get("outdated"):
            metadata.append("outdated=true")
        original_oid = comment_original_commit_oid(comment)
        current_oid = comment_commit_oid(comment)
        if original_oid:
            metadata.append(f"original={original_oid[:12]}")
        if current_oid:
            metadata.append(f"current={current_oid[:12]}")
        summary = redact_comment_body(str(comment.get("body") or "")).replace("\n", " ")
        url = str(comment.get("url") or "")
        suffix = f" ({', '.join(metadata)})" if metadata else ""
        grouped.setdefault(category, []).append(f"- {location}{suffix}: {trim_text(summary, 500)} {url}".rstrip())
    return grouped


def build_review_context_markdown(
    *,
    repo: str,
    pr_number: str,
    head_sha: str,
    pr: dict[str, Any],
    issue_comments: list[dict[str, Any]],
    reviews: list[dict[str, Any]],
    threads: list[dict[str, Any]],
) -> str:
    title = str(pr.get("title") or "")
    body = redact_comment_body(str(pr.get("body") or "")).strip() or "(PR body is empty.)"
    latest_design = latest_marker_comment(issue_comments, DESIGN_MARKER)
    grouped_threads = unresolved_thread_summaries(threads)
    recent_reviews = []
    for review in reviews:
        review_body = str(review.get("body") or "")
        marker_kind = review_marker_kind(review_body)
        if marker_kind is None:
            continue
        author = ((review.get("user") or {}).get("login")) or ""
        if author and not is_trusted_codex_review_author(author):
            continue
        recent_reviews.append((str(review.get("submitted_at") or ""), marker_kind, review))
    recent_reviews = sorted(recent_reviews, key=lambda item: item[0])[-5:]

    lines = [
        "# Codex Review Context",
        "",
        "이 컨텍스트는 현재 리뷰 라운드의 참고 자료다.",
        "우선순위: 현재 사용자 지시 > 현재 PR body > 현재 코드/diff/docs > 테스트 > 이전 리뷰/해결/설계 기록.",
        "이전 리뷰, 이전 resolve 결과, 이전 design plan은 advisory이며 현재 코드나 현재 PR spec을 덮어쓸 수 없다.",
        "",
        "## Current PR State (authoritative)",
        "",
        f"- repository: {repo}",
        f"- pr_number: {pr_number}",
        f"- head_sha: {head_sha}",
        f"- title: {title}",
        "",
        trim_text(body, REVIEW_CONTEXT_SECTION_LIMIT),
        "",
        "## Latest Sticky Design Plan (advisory)",
        "",
    ]
    if latest_design:
        lines.extend(
            [
                f"- updated_at: {latest_design.get('updated_at') or latest_design.get('created_at') or ''}",
                f"- author: {((latest_design.get('user') or {}).get('login')) or ''}",
                "",
                trim_text(redact_comment_body(str(latest_design.get("body") or "")), REVIEW_CONTEXT_SECTION_LIMIT),
            ]
        )
    else:
        lines.append("(none)")

    lines.extend(["", "## Recent Codex Review And Resolve Summaries (advisory)", ""])
    if recent_reviews:
        for submitted_at, marker_kind, review in recent_reviews:
            author = ((review.get("user") or {}).get("login")) or ""
            state = review.get("state") or ""
            lines.extend(
                [
                    f"### {marker_kind} {submitted_at}",
                    "",
                    f"- author: {author}",
                    f"- state: {state}",
                    "",
                    trim_text(redact_comment_body(str(review.get("body") or "")), 1800),
                    "",
                ]
            )
    else:
        lines.append("(none)")

    lines.extend(["", "## Current Unresolved Inline Threads (advisory until verified)", ""])
    if grouped_threads:
        for category in sorted(grouped_threads):
            lines.extend([f"### {category}", ""])
            lines.extend(grouped_threads[category][:30])
            if len(grouped_threads[category]) > 30:
                lines.append(f"- ... {len(grouped_threads[category]) - 30} more")
            lines.append("")
    else:
        lines.append("(none)")

    return trim_text("\n".join(lines).rstrip() + "\n", REVIEW_CONTEXT_MAX_CHARS)


def command_build_review_context(args: argparse.Namespace) -> None:
    repo = require_env("GITHUB_REPOSITORY")
    pr_number = require_env("PR_NUMBER")
    head_sha = require_env("HEAD_SHA")
    pr = github_api(f"/repos/{repo}/pulls/{pr_number}")
    issue_comments = github_paginated(f"/repos/{repo}/issues/{pr_number}/comments?per_page=100")
    reviews = github_paginated(f"/repos/{repo}/pulls/{pr_number}/reviews?per_page=100")
    threads = collect_review_threads(repo, pr_number)
    output = build_review_context_markdown(
        repo=repo,
        pr_number=pr_number,
        head_sha=head_sha,
        pr=pr,
        issue_comments=issue_comments,
        reviews=reviews,
        threads=threads,
    )
    Path(args.output).write_text(output, encoding="utf-8")
    print(f"wrote review context to {args.output}")


def finding_effectively_allowed(finding: dict[str, Any], decisions: dict[str, Any]) -> bool:
    decision = decisions["by_id"].get(finding["id"])
    return hard_allow(finding) or bool(decision and decision.get("allow"))


def design_blockers(findings: list[dict[str, Any]], decisions: dict[str, Any]) -> list[dict[str, Any]]:
    blockers: list[dict[str, Any]] = []
    for finding in findings:
        if not finding_effectively_allowed(finding, decisions):
            continue
        if hard_block(finding) or finding.get("type") == "MUST":
            blockers.append(finding)
    return blockers


def should_run_design(findings: list[dict[str, Any]], decisions: dict[str, Any]) -> tuple[bool, int]:
    judgment = decisions.get("judgment") or {}
    blockers = design_blockers(findings, decisions)
    return judgment.get("status") == "NEEDS_WORK" or bool(blockers), len(blockers)


def command_classify_design_need(args: argparse.Namespace) -> None:
    findings = load_current_findings(Path(args.artifacts))
    decisions = load_decisions(Path(args.decisions))
    needs_design, blocking_count = should_run_design(findings, decisions)
    write_github_output({"needs_design": "true" if needs_design else "false", "blocking_count": str(blocking_count)})
    print(f"needs_design={needs_design} blocking_count={blocking_count}")


DESIGN_PLAN_LIST_KEYS = (
    "invariants",
    "retired_approaches",
    "intended_architecture",
    "edit_sequence",
    "tests",
    "acceptance_criteria",
    "open_questions",
)


def design_plan_list(plan: dict[str, Any], key: str) -> list[str]:
    value = plan.get(key)
    if not isinstance(value, list):
        return []
    return [trim_text(redact_secrets(str(item)), DESIGN_PLAN_ITEM_LIMIT).strip() for item in value if str(item).strip()]


def compact_design_plan(plan: dict[str, Any]) -> dict[str, Any]:
    compact: dict[str, Any] = {
        "version": plan.get("version", 1),
        "summary": trim_text(redact_secrets(str(plan.get("summary") or "")), DESIGN_PLAN_SUMMARY_LIMIT).strip(),
        "root_cause": trim_text(redact_secrets(str(plan.get("root_cause") or "")), DESIGN_PLAN_ROOT_CAUSE_LIMIT).strip(),
    }
    for key in DESIGN_PLAN_LIST_KEYS:
        compact[key] = design_plan_list(plan, key)
    return compact


def render_design_plan_body(plan: dict[str, Any]) -> str:
    plan = compact_design_plan(plan)
    summary = str(plan.get("summary") or "").strip() or "설계 요약이 제공되지 않았습니다."
    root_cause = (
        str(plan.get("root_cause") or "").strip()
        or "root cause가 명시되지 않았습니다."
    )
    lines = [
        DESIGN_MARKER,
        "# Codex Design Plan",
        "",
        "이 설계안은 자동 리뷰 이후의 advisory plan입니다. 현재 PR body, 현재 코드, 현재 사용자 지시가 이 기록보다 우선합니다.",
        "",
        "## Summary",
        "",
        summary,
        "",
        "## Root Cause",
        "",
        root_cause,
    ]
    sections = [
        ("Invariants", "invariants"),
        ("Retired / Failed Approaches", "retired_approaches"),
        ("Intended Architecture", "intended_architecture"),
        ("Edit Sequence", "edit_sequence"),
        ("Tests", "tests"),
        ("Acceptance Criteria", "acceptance_criteria"),
        ("Open Questions", "open_questions"),
    ]
    for title, key in sections:
        items = design_plan_list(plan, key)
        lines.extend(["", f"## {title}", ""])
        if items:
            lines.extend(f"- {item}" for item in items)
        else:
            lines.append("- 없음")

    lines.extend(
        [
            "",
            "## Machine Readable JSON",
            "",
            "```json",
            json.dumps(plan, ensure_ascii=False, indent=2),
            "```",
        ]
    )
    return "\n".join(lines).rstrip() + "\n"


def command_render_design_plan(args: argparse.Namespace) -> None:
    plan_path = Path(args.plan)
    plan = compact_design_plan(json.loads(plan_path.read_text(encoding="utf-8")))
    plan_path.write_text(json.dumps(plan, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    Path(args.output).write_text(render_design_plan_body(plan), encoding="utf-8")
    print(f"wrote design plan markdown to {args.output}")


def upsert_design_comment(
    *,
    repo: str,
    pr_number: str,
    body: str,
    list_comments: Any = github_paginated,
    api: Any = github_api,
) -> str:
    comments = list_comments(f"/repos/{repo}/issues/{pr_number}/comments?per_page=100")
    existing = latest_marker_comment(comments, DESIGN_MARKER)
    if existing:
        api(f"/repos/{repo}/issues/comments/{existing['id']}", method="PATCH", payload={"body": body})
        return "updated"
    api(f"/repos/{repo}/issues/{pr_number}/comments", method="POST", payload={"body": body})
    return "created"


def command_post_design_plan(args: argparse.Namespace) -> None:
    repo = require_env("GITHUB_REPOSITORY")
    pr_number = require_env("PR_NUMBER")
    body = Path(args.body).read_text(encoding="utf-8")
    if DESIGN_MARKER not in body:
        raise SystemExit("design plan body is missing sticky marker")
    action = upsert_design_comment(repo=repo, pr_number=pr_number, body=body)
    print(f"{action} sticky design plan comment")


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="command", required=True)

    post_current = subparsers.add_parser("post-current")
    post_current.add_argument("--artifacts", required=True)
    post_current.add_argument("--decisions", required=True)
    post_current.set_defaults(func=command_post_current)

    resolve_current = subparsers.add_parser("resolve-current")
    resolve_current.set_defaults(func=command_resolve_current)

    resolve_previous = subparsers.add_parser("resolve-previous")
    resolve_previous.set_defaults(func=command_resolve_previous)

    collect = subparsers.add_parser("collect-resolutions")
    collect.add_argument("--workspace", required=True)
    collect.add_argument("--batch-dir", required=True)
    collect.set_defaults(func=command_collect_resolutions)

    apply = subparsers.add_parser("apply-resolutions")
    apply.add_argument("--batches", required=True)
    apply.add_argument("--results", required=True)
    apply.set_defaults(func=command_apply_resolutions)

    review_context = subparsers.add_parser("build-review-context")
    review_context.add_argument("--output", required=True)
    review_context.set_defaults(func=command_build_review_context)

    classify_design = subparsers.add_parser("classify-design-need")
    classify_design.add_argument("--artifacts", required=True)
    classify_design.add_argument("--decisions", required=True)
    classify_design.set_defaults(func=command_classify_design_need)

    render_design = subparsers.add_parser("render-design-plan")
    render_design.add_argument("--plan", required=True)
    render_design.add_argument("--output", required=True)
    render_design.set_defaults(func=command_render_design_plan)

    post_design = subparsers.add_parser("post-design-plan")
    post_design.add_argument("--body", required=True)
    post_design.set_defaults(func=command_post_design_plan)
    return parser


def main() -> int:
    args = build_parser().parse_args()
    args.func(args)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
