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
MAX_INLINE_COMMENTS = 50
RESOLVE_BATCH_SIZE = 3
TRUSTED_USER = "DongwonTTuna"


def require_env(name: str) -> str:
    value = os.environ.get(name, "").strip()
    if not value:
        raise SystemExit(f"{name} is required")
    return value


def trim_text(value: Any, limit: int) -> str:
    text = "" if value is None else str(value)
    return text if len(text) <= limit else text[:limit] + "\n...[truncated]"


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

    if event_name == "pull_request":
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
            "trigger": f"pull_request:{event.get('action', '')}",
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
    if actor != TRUSTED_USER or triggering_actor != TRUSTED_USER or event_name != "pull_request":
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


def redact_comment_body(body: str) -> str:
    patterns = [
        r"-----BEGIN [A-Z ]*PRIVATE KEY-----.*?-----END [A-Z ]*PRIVATE KEY-----",
        r"\bgh[opsru]_[A-Za-z0-9_]{20,}\b",
        r"\bsk-[A-Za-z0-9_-]{20,}\b",
        r"\bsk-clb-[A-Za-z0-9_-]{20,}\b",
    ]
    redacted = body
    for pattern in patterns:
        redacted = re.sub(pattern, "[redacted]", redacted, flags=re.DOTALL)
    return trim_text(redacted, 1200)


def extract_marker_key(body: str) -> str | None:
    match = re.search(r"<!--\s*codex-review-id:\s*([^>]+?)\s*-->", body)
    return match.group(1).strip() if match else None


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


def build_resolve_item(thread: dict[str, Any], comment: dict[str, Any], workspace: Path) -> dict[str, Any]:
    line = comment.get("line") or thread.get("line") or comment.get("originalLine") or thread.get("originalLine")
    try:
        line_int = int(line) if line is not None else None
    except (TypeError, ValueError):
        line_int = None
    file_path = comment.get("path") or thread.get("path")
    return {
        "thread_id": thread["id"],
        "comment_node_id": comment["id"],
        "comment_id": int(comment["fullDatabaseId"]),
        "file": file_path,
        "line": line_int,
        "outdated": bool(thread.get("isOutdated") or comment.get("outdated")),
        "marker_key": extract_marker_key(comment.get("body") or ""),
        "body_excerpt": redact_comment_body(comment.get("body") or ""),
        "code_snippet": code_snippet(workspace, file_path, line_int),
        "url": comment.get("url"),
    }


def command_collect_resolutions(args: argparse.Namespace) -> None:
    repo = require_env("GITHUB_REPOSITORY")
    pr_number = require_env("PR_NUMBER")
    head_sha = require_env("HEAD_SHA")
    workspace = Path(args.workspace)
    batch_dir = Path(args.batch_dir)
    batch_dir.mkdir(parents=True, exist_ok=True)

    items: list[dict[str, Any]] = []
    for thread in collect_review_threads(repo, pr_number):
        if thread.get("isResolved"):
            continue
        for comment in (thread.get("comments") or {}).get("nodes") or []:
            body = comment.get("body") or ""
            author = ((comment.get("author") or {}).get("login")) or ""
            commit_oid = ((comment.get("commit") or {}).get("oid")) or ""
            if INLINE_MARKER not in body:
                continue
            if not author.endswith("[bot]"):
                continue
            if commit_oid == head_sha:
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
    return parser


def main() -> int:
    args = build_parser().parse_args()
    args.func(args)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
