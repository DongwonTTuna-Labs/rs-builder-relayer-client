#!/usr/bin/env python3
from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any


AXES = ("correctness", "security", "performance", "test-coverage", "domain")
FINDING_ID_PATTERN = r"^(correctness|security|performance|test-coverage|domain)-[0-9]+$"
INLINE_MARKER = "<!-- codex-review-inline -->"
REVIEW_MARKER = "<!-- codex-review -->"
REVIEW_SUMMARY_MARKER = "<!-- codex-review-summary -->"
RESOLVE_MARKER = "<!-- codex-resolve-check -->"
DESIGN_MARKER = "<!-- codex-design-plan -->"
THREAD_LIFECYCLE_MARKER = "codex-thread-lifecycle:v3"
ISSUE_KEY_MARKER = "codex-issue-key:"
MAX_INLINE_COMMENTS = 12
MAX_INLINE_COMMENTS_PER_FILE = 3
THREAD_LIFECYCLE_BATCH_SIZE = 12
THREAD_LIFECYCLE_HARD_MAX = 16
THREAD_LIFECYCLE_BATCH_CHAR_BUDGET = 24000
RESOLVE_BATCH_SIZE = THREAD_LIFECYCLE_BATCH_SIZE
THREAD_INVENTORY_SCHEMA = "codex.thread_inventory.v3"
RESOLVE_MANIFEST_SCHEMA = "codex-resolve-manifest.v3"
THREAD_LIFECYCLE_BATCH_SCHEMA = "codex.thread_lifecycle_batch.v3"
THREAD_LIFECYCLE_RESULT_SCHEMA = "codex.thread_lifecycle_result.v3"
REVIEW_CONTEXT_MAX_CHARS = 60000
REVIEW_CONTEXT_SECTION_LIMIT = 12000
TRUSTED_USER = "DongwonTTuna"
TRUSTED_CODEX_REVIEW_AUTHORS = ("codex-reviewer-for-dongwonttuna", "codex-reviewer-for-dongwonttuna[bot]")
CODEX_AUTOFIX_COMMIT_SUBJECT = "fix(codex-review): apply bounded autofix"
LIFECYCLE_STATES = {
    "resolved_by_code",
    "fix_now",
    "defer_to_issue",
    "duplicate_of_issue",
    "false_positive",
    "stale_obsolete",
    "needs_human",
}
TERMINAL_LIFECYCLE_STATES = {
    "resolved_by_code",
    "defer_to_issue",
    "duplicate_of_issue",
    "false_positive",
    "stale_obsolete",
}
INLINE_ACTIONS = {"publish_and_fix_now"}
SUMMARY_ACTIONS = {"summary_only_fix_now", "defer_to_issue", "deny_false_positive", "needs_human"}
TECH_LEAD_ACTIONS = INLINE_ACTIONS | SUMMARY_ACTIONS
AUTOFIX_MANIFEST_SCHEMA = "codex.autofix_manifest.v1"
AUTOFIX_MAX_FILES = 8
AUTOFIX_MAX_PATCH_BYTES = 120000
AUTOFIX_MAX_COMMITS = 2
AUTOFIX_ALLOWED_PREFIXES = (".github/scripts/", "docs/", "src/", "tests/")
AUTOFIX_FORBIDDEN_PREFIXES = (".codex/", ".github/actions/", ".github/workflows/")
AUTOFIX_FORBIDDEN_FILES = {"Cargo.lock", "Cargo.toml"}
AUTOFIX_DANGEROUS_KEYWORDS = (
    "api key",
    "auth",
    "calldata",
    "eip-712",
    "exported",
    "live-capable",
    "nonce",
    "private key",
    "public api",
    "secret",
    "serde",
    "signature",
    "signing",
    "wallet",
    "wallet-create",
    "wire",
)
SECRET_PATTERNS = (
    r"-----BEGIN [A-Z ]*PRIVATE KEY-----.*?-----END [A-Z ]*PRIVATE KEY-----",
    r"\bgh[opsru]_[A-Za-z0-9_]{20,}\b",
    r"\bgithub_pat_[A-Za-z0-9_]{20,}\b",
    r"\bsk-(?:proj-)?[A-Za-z0-9_-]{20,}\b",
    r"\bsk-clb-[A-Za-z0-9_-]{20,}\b",
    r"\bAKIA[0-9A-Z]{16}\b",
    r"\beyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\b",
)


def require_env(name: str) -> str:
    value = os.environ.get(name, "").strip()
    if not value:
        raise SystemExit(f"{name} is required")
    return value


def trim_text(value: Any, limit: int) -> str:
    text = "" if value is None else str(value)
    return text if len(text) <= limit else text[:limit] + "\n...[truncated]"


def marker_value(value: Any, *, limit: int = 120) -> str:
    text = trim_text(redact_secrets("" if value is None else str(value)), limit).replace("\r", " ").replace("\n", " ").strip()
    text = text.replace("-->", "").replace("--", "-")
    return text


def slug_key(value: Any, *, fallback: str) -> str:
    slug = re.sub(r"[^a-z0-9-]+", "-", marker_value(value).lower()).strip("-")
    return slug or fallback


def is_finding_id_key(value: Any) -> bool:
    return bool(re.fullmatch(FINDING_ID_PATTERN, str(value or "").strip()))


def normalize_root_cause_key(value: Any, *, context: str) -> str:
    normalized = slug_key(value, fallback="")
    if not normalized:
        raise SystemExit(f"{context} root_cause_key is required")
    if is_finding_id_key(normalized):
        raise SystemExit(f"{context} root_cause_key must not be a finding id: {normalized}")
    return normalized


def is_trusted_codex_review_author(author: str) -> bool:
    return author in TRUSTED_CODEX_REVIEW_AUTHORS


def is_trusted_workflow_actor(actor: str) -> bool:
    return actor == TRUSTED_USER or is_trusted_codex_review_author(actor)


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


def split_repo(repo: str) -> tuple[str, str]:
    parts = repo.split("/", 1)
    if len(parts) != 2 or not parts[0] or not parts[1]:
        raise SystemExit(f"invalid repository name: {repo}")
    return parts[0], parts[1]


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
        "head_ref": "",
        "base_ref": "",
        "base_sha": "",
        "trigger": "",
        "trigger_class": "",
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
    fetch_commit: Any = github_api,
    fetch_compare: Any = github_api,
) -> dict[str, str]:
    if event_name in {"pull_request", "pull_request_target"}:
        pr = event.get("pull_request") or {}
        sender = (event.get("sender") or {}).get("login")
        base_ref = ((pr.get("base") or {}).get("ref")) or ""
        head_repo = ((pr.get("head") or {}).get("repo") or {}).get("full_name")
        author = (pr.get("user") or {}).get("login")
        action = str(event.get("action") or "")
        if (
            not pr
            or pr.get("draft")
            or base_ref != "main"
            or head_repo != repo
            or author != TRUSTED_USER
        ):
            return skipped_current_review()
        trigger_class = ""
        if actor == TRUSTED_USER and triggering_actor == TRUSTED_USER and sender == TRUSTED_USER:
            trigger_class = "human_trusted_synchronize"
        elif (
            event_name == "pull_request_target"
            and action == "synchronize"
            and is_trusted_codex_review_author(actor)
            and is_trusted_codex_review_author(triggering_actor)
            and is_trusted_codex_review_author(sender or "")
            and is_codex_autofix_commit(
                repo,
                str(pr["head"]["sha"]),
                base_sha=str(pr["base"]["sha"]),
                fetch_commit=fetch_commit,
                fetch_compare=fetch_compare,
            )
        ):
            trigger_class = "codex_app_autofix_synchronize"
        else:
            return skipped_current_review()
        return {
            "should_run": "true",
            "pr_number": str(pr["number"]),
            "head_sha": str(pr["head"]["sha"]),
            "head_ref": str((pr.get("head") or {}).get("ref") or ""),
            "base_ref": "main",
            "base_sha": str(pr["base"]["sha"]),
            "trigger": f"{event_name}:{event.get('action', '')}",
            "trigger_class": trigger_class,
        }

    if event_name == "issue_comment":
        if actor != TRUSTED_USER or triggering_actor != TRUSTED_USER:
            return skipped_current_review()
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
            "head_ref": str((pr.get("head") or {}).get("ref") or ""),
            "base_ref": "main",
            "base_sha": str(pr["base"]["sha"]),
            "trigger": "issue_comment:/codex-review",
            "trigger_class": "human_command",
        }

    return skipped_current_review()


def is_codex_autofix_commit(
    repo: str,
    head_sha: str,
    *,
    base_sha: str = "",
    fetch_commit: Any = github_api,
    fetch_compare: Any = github_api,
) -> bool:
    try:
        commit = fetch_commit(f"/repos/{repo}/commits/{head_sha}")
    except SystemExit:
        return False
    message = str(((commit.get("commit") or {}).get("message")) or "")
    subject = message.splitlines()[0] if message else ""
    author_login = str(((commit.get("author") or {}).get("login")) or "")
    committer_login = str(((commit.get("committer") or {}).get("login")) or "")
    if subject != CODEX_AUTOFIX_COMMIT_SUBJECT:
        return False
    if not author_login or not committer_login:
        return False
    if author_login and not is_trusted_codex_review_author(author_login):
        return False
    if committer_login and not is_trusted_codex_review_author(committer_login):
        return False
    if not base_sha:
        return True
    try:
        compare = fetch_compare(f"/repos/{repo}/compare/{base_sha}...{head_sha}")
    except SystemExit:
        return False
    commits = compare.get("commits") or []
    if not any(str(item.get("sha") or "") == head_sha for item in commits):
        return False
    autofix_count = 0
    for item in commits:
        item_message = str(((item.get("commit") or {}).get("message")) or "")
        item_subject = item_message.splitlines()[0] if item_message else ""
        if item_subject == CODEX_AUTOFIX_COMMIT_SUBJECT:
            autofix_count += 1
    return autofix_count <= AUTOFIX_MAX_COMMITS


def resolve_previous_review_event(
    *,
    event_name: str,
    event: dict[str, Any],
    repo: str,
    actor: str,
    triggering_actor: str,
    fetch_pr: Any = github_api,
) -> dict[str, str]:
    if event_name in {"pull_request", "pull_request_target"}:
        if actor != TRUSTED_USER or triggering_actor != TRUSTED_USER:
            return skipped_resolve_checker()
        pr = event.get("pull_request") or {}
    elif event_name == "workflow_dispatch":
        if actor != TRUSTED_USER or triggering_actor != TRUSTED_USER:
            return skipped_resolve_checker()
        pr_number = str((event.get("inputs") or {}).get("pr_number") or "").strip()
        if not pr_number:
            return skipped_resolve_checker()
        pr = fetch_pr(f"/repos/{repo}/pulls/{pr_number}")
    elif event_name == "workflow_run":
        if not is_trusted_workflow_actor(actor) or not is_trusted_workflow_actor(triggering_actor):
            return skipped_resolve_checker()
        workflow_run = event.get("workflow_run") or {}
        if (
            workflow_run.get("name") != "Codex PR Review"
            or workflow_run.get("event") not in {"pull_request_target", "issue_comment"}
            or workflow_run.get("conclusion") != "success"
        ):
            return skipped_resolve_checker()
        pull_requests = workflow_run.get("pull_requests") or []
        if len(pull_requests) != 1:
            return skipped_resolve_checker()
        pr_number = str(pull_requests[0].get("number") or "").strip()
        if not pr_number:
            return skipped_resolve_checker()
        pr = fetch_pr(f"/repos/{repo}/pulls/{pr_number}")
        completed_head_sha = str(workflow_run.get("head_sha") or "").strip()
        current_head_sha = str(((pr.get("head") or {}).get("sha")) or "")
        if completed_head_sha and completed_head_sha != current_head_sha:
            return skipped_resolve_checker()
    else:
        return skipped_resolve_checker()

    base_ref = ((pr.get("base") or {}).get("ref")) or ""
    head_repo = ((pr.get("head") or {}).get("repo") or {}).get("full_name")
    author = (pr.get("user") or {}).get("login")
    if not pr or pr.get("draft") or base_ref != "main" or head_repo != repo or author != TRUSTED_USER:
        return skipped_resolve_checker()
    return {
        "should_collect": "true",
        "pr_number": str(pr["number"]),
        "head_sha": str(pr["head"]["sha"]),
        "head_repo": str(head_repo),
        "base_ref": "main",
        "base_sha": str(pr["base"]["sha"]),
        "upstream_run_id": str(((event.get("workflow_run") or {}).get("id")) or ""),
        "upstream_run_attempt": str(((event.get("workflow_run") or {}).get("run_attempt")) or ""),
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
    if not re.fullmatch(FINDING_ID_PATTERN, finding_id):
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
    root_cause_key = normalize_root_cause_key(item.get("root_cause_key"), context=finding_id)
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
        "root_cause_key": root_cause_key,
        "scope": str(item.get("scope") or "current_pr"),
        "public_api_risk": bool(item.get("public_api_risk")),
        "autofix_eligible_hint": bool(item.get("autofix_eligible_hint")),
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
            primary_root_present = decision.get("primary_root_cause_key") is not None
            raw_primary_root = trim_text(decision.get("primary_root_cause_key"), 120).strip()
            by_id[decision_id] = {
                "action": normalize_tech_lead_action(decision),
                "reason": trim_text(decision.get("reason"), 300).strip() or "No decision reason provided.",
                "primary_root_cause_key": raw_primary_root,
                "primary_root_cause_key_present": primary_root_present,
            }
    judgment = payload.get("judgment") if isinstance(payload.get("judgment"), dict) else None
    merge_notes = payload.get("merge_notes") if isinstance(payload.get("merge_notes"), list) else []
    return {"by_id": by_id, "judgment": judgment, "merge_notes": merge_notes}


def validate_decision_coverage(findings: list[dict[str, Any]], decisions: dict[str, Any]) -> None:
    expected = {finding["id"] for finding in findings}
    actual = set(decisions["by_id"])
    if actual != expected:
        missing = sorted(expected - actual)
        extra = sorted(actual - expected)
        detail = []
        if missing:
            detail.append("missing=" + ",".join(missing))
        if extra:
            detail.append("extra=" + ",".join(extra))
        raise SystemExit("tech-lead decisions do not exactly cover findings: " + "; ".join(detail))
    known_roots = {finding["root_cause_key"] for finding in findings}
    for finding in findings:
        decision = decisions["by_id"].get(finding["id"]) or {}
        raw_primary = str(decision.get("primary_root_cause_key") or "").strip()
        if decision.get("primary_root_cause_key_present") and not raw_primary:
            raise SystemExit(f"{finding['id']} primary_root_cause_key must not be empty when present")
        if not raw_primary:
            continue
        primary = normalize_root_cause_key(raw_primary, context=f"{finding['id']} primary_root_cause_key")
        if primary not in known_roots:
            raise SystemExit(f"{finding['id']} primary_root_cause_key does not match any known root cause: {primary}")
        decision["primary_root_cause_key"] = primary


def normalize_tech_lead_action(decision: dict[str, Any]) -> str:
    action = str(decision.get("action") or "").strip()
    if action in TECH_LEAD_ACTIONS:
        return action
    raise SystemExit(f"unknown tech-lead action: {action or '<missing>'}")


def action_for_finding(finding: dict[str, Any], decision: dict[str, Any] | None) -> str:
    if decision:
        return str(decision.get("action") or normalize_tech_lead_action(decision))
    raise SystemExit(f"missing tech-lead decision for {finding['id']}")


def autofix_block_reason(finding: dict[str, Any], action: str) -> str | None:
    if action != "publish_and_fix_now":
        return f"tech-lead action is {action}"
    if finding.get("agent") == "security":
        return "security finding requires human review"
    if finding.get("scope") != "current_pr":
        return f"finding scope is {finding.get('scope')}"
    if not finding.get("file"):
        return "cross-cutting finding is not eligible for bounded autofix"
    if finding.get("public_api_risk"):
        return "public API risk requires human review"
    if not finding.get("autofix_eligible_hint"):
        return "reviewer did not mark this finding as autofix eligible"
    haystack = " ".join(
        str(finding.get(key) or "")
        for key in ("agent", "file", "title", "reason", "rule_ref", "root_cause_key")
    ).lower()
    for keyword in AUTOFIX_DANGEROUS_KEYWORDS:
        if keyword in haystack:
            return f"dangerous autofix keyword matched: {keyword}"
    return None


def build_autofix_manifest(findings: list[dict[str, Any]], decisions: dict[str, Any]) -> dict[str, Any]:
    eligible: list[dict[str, Any]] = []
    blocked: list[dict[str, Any]] = []
    seen_roots: set[str] = set()
    for finding in findings:
        decision = decisions["by_id"].get(finding["id"])
        action = action_for_finding(finding, decision)
        reason = autofix_block_reason(finding, action)
        root = normalize_root_cause_key(finding.get("root_cause_key"), context=finding["id"])
        if reason is None and root in seen_roots:
            reason = f"root cause already represented by an earlier eligible finding: {root}"
        if reason:
            blocked.append({"id": finding["id"], "root_cause_key": root, "action": action, "reason": reason})
            continue
        seen_roots.add(root)
        eligible.append(
            {
                "id": finding["id"],
                "agent": finding["agent"],
                "type": finding["type"],
                "file": finding.get("file"),
                "line": finding.get("line"),
                "title": finding["title"],
                "reason": finding["reason"],
                "root_cause_key": root,
                "tech_lead_reason": (decision or {}).get("reason") or "",
            }
        )
    return {
        "schema_version": AUTOFIX_MANIFEST_SCHEMA,
        "eligible": eligible,
        "blocked": blocked,
        "limits": {
            "max_files": AUTOFIX_MAX_FILES,
            "max_patch_bytes": AUTOFIX_MAX_PATCH_BYTES,
            "forbidden_prefixes": list(AUTOFIX_FORBIDDEN_PREFIXES),
            "forbidden_files": sorted(AUTOFIX_FORBIDDEN_FILES),
        },
    }


def command_plan_autofix(args: argparse.Namespace) -> None:
    findings = load_current_findings(Path(args.artifacts))
    decisions = load_decisions(Path(args.decisions))
    validate_decision_coverage(findings, decisions)
    manifest = build_autofix_manifest(findings, decisions)
    Path(args.output).write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    eligible_count = len(manifest["eligible"])
    write_github_output({"should_run": "true" if eligible_count else "false", "eligible_count": str(eligible_count)})
    print(f"planned bounded autofix; eligible={eligible_count} blocked={len(manifest['blocked'])}")


def patch_changed_files(patch_text: str) -> list[str]:
    files: list[str] = []
    for match in re.finditer(r"^diff --git a/(.*?) b/(.*?)$", patch_text, flags=re.MULTILINE):
        path = match.group(2)
        if path not in files:
            files.append(path)
    return files


def validate_autofix_patch_text(patch_text: str, manifest: dict[str, Any]) -> None:
    if manifest.get("schema_version") != AUTOFIX_MANIFEST_SCHEMA:
        raise SystemExit(f"invalid autofix manifest schema: {manifest.get('schema_version')}")
    if not manifest.get("eligible"):
        raise SystemExit("autofix manifest has no eligible findings")
    encoded_size = len(patch_text.encode("utf-8"))
    if encoded_size == 0 or not patch_text.strip():
        raise SystemExit("autofix patch is empty")
    if encoded_size > AUTOFIX_MAX_PATCH_BYTES:
        raise SystemExit(f"autofix patch is too large: {encoded_size} bytes")
    assert_no_secret_patterns(patch_text, "autofix patch")
    files = patch_changed_files(patch_text)
    if not files:
        raise SystemExit("autofix patch has no changed files")
    if len(files) > AUTOFIX_MAX_FILES:
        raise SystemExit(f"autofix patch changes too many files: {len(files)}")
    eligible_files = {str(item.get("file")) for item in manifest.get("eligible", []) if item.get("file")}
    if not eligible_files:
        raise SystemExit("autofix manifest has no file-scoped eligible findings")
    for path in files:
        if path not in eligible_files:
            raise SystemExit(f"autofix patch touches file outside eligible findings: {path}")
        if path in AUTOFIX_FORBIDDEN_FILES or any(path.startswith(prefix) for prefix in AUTOFIX_FORBIDDEN_PREFIXES):
            raise SystemExit(f"autofix patch touches forbidden path: {path}")
        if not any(path.startswith(prefix) for prefix in AUTOFIX_ALLOWED_PREFIXES):
            raise SystemExit(f"autofix patch touches unsupported path: {path}")
    for line in patch_text.splitlines():
        if line.startswith(("GIT binary patch", "Binary files ")):
            raise SystemExit("autofix patch contains binary changes")
        if line.startswith(
            (
                "deleted file mode ",
                "new file mode 120000",
                "old mode ",
                "new mode ",
                "similarity index ",
                "dissimilarity index ",
                "rename from ",
                "rename to ",
                "copy from ",
                "copy to ",
            )
        ):
            raise SystemExit(f"autofix patch contains unsupported file operation: {line}")
        if not (line.startswith("+") or line.startswith("-")) or line.startswith(("+++", "---")):
            continue
        changed = line[1:]
        assert_no_secret_patterns(changed, "autofix patch")
        lowered = changed.lower()
        if re.search(
            r"\bpub(?:\([^)]*\))?\s+(?:async\s+)?(fn|struct|enum|mod|trait|type|use|const|static)\b",
            changed,
        ):
            raise SystemExit("autofix patch changes public Rust API surface")
        for keyword in AUTOFIX_DANGEROUS_KEYWORDS:
            if keyword in lowered:
                raise SystemExit(f"autofix patch contains guarded keyword: {keyword}")


def command_validate_autofix_patch(args: argparse.Namespace) -> None:
    manifest = json.loads(Path(args.manifest).read_text(encoding="utf-8"))
    patch_text = Path(args.patch).read_text(encoding="utf-8")
    validate_autofix_patch_text(patch_text, manifest)
    print(f"validated bounded autofix patch for {len(patch_changed_files(patch_text))} files")


def root_cause_marker_key(finding: dict[str, Any], decision: dict[str, Any] | None) -> str:
    fallback = normalize_root_cause_key(finding.get("root_cause_key"), context=str(finding.get("id") or "finding"))
    raw = (decision or {}).get("primary_root_cause_key")
    if not raw:
        return fallback
    try:
        return normalize_root_cause_key(raw, context=f"{finding.get('id')} primary_root_cause_key")
    except SystemExit:
        return fallback


def render_current_inline(finding: dict[str, Any], decision: dict[str, Any] | None) -> str:
    root_cause_key = root_cause_marker_key(finding, decision)
    area = marker_value(thread_area(str(finding.get("file") or "")) or "general")
    failure_kind = marker_value(finding.get("agent") or "unknown")
    lines = [
        INLINE_MARKER,
        f"<!-- codex-review-id: {finding['id']} -->",
        f"<!-- codex-root-cause-key: {root_cause_key} -->",
        f"<!-- codex-root-cause-area: {area} -->",
        f"<!-- codex-root-cause-failure-kind: {failure_kind} -->",
        f"**[{finding['type']}][{finding['agent']}] {redact_secrets(str(finding['title']))}**",
        "",
        redact_secrets(str(finding["reason"])),
    ]
    if decision:
        lines.extend(["", f"테크리드: {redact_secrets(str(decision['reason']))}"])
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
        REVIEW_SUMMARY_MARKER,
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
                f"- 테크리드 요약: {redact_secrets(str(judgment.get('headline', '')))}",
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
                f"{redact_secrets(str(finding['title']))}: {redact_secrets(str(finding['reason']))}{redact_secrets(suffix)}"
            )
    merge_notes = decisions.get("merge_notes") or []
    if merge_notes:
        lines.extend(["", "병합 메모:"])
        for note in merge_notes[:10]:
            lines.append(
                f"- {note.get('primary_id')}: 병합됨 {', '.join(note.get('merged_ids') or [])} - "
                f"{redact_secrets(str(note.get('reason', '')))}"
            )
    return "\n".join(lines)


def render_current_review_body(event: str) -> str:
    return "\n".join(
        [
            REVIEW_MARKER,
            "Codex 리뷰 inline 결과가 게시되었습니다.",
            "",
            f"- 이벤트: {event}",
            "- 상세 요약은 sticky `codex-review-summary` 코멘트를 확인하세요.",
        ]
    )


def command_post_current(args: argparse.Namespace) -> None:
    repo = require_env("GITHUB_REPOSITORY")
    pr_number = require_env("PR_NUMBER")
    head_sha = require_env("HEAD_SHA")
    findings = load_current_findings(Path(args.artifacts))
    decisions = load_decisions(Path(args.decisions))
    validate_decision_coverage(findings, decisions)
    changed_by_file = build_changed_line_map(repo, pr_number)

    allowed: list[tuple[dict[str, Any], dict[str, Any] | None]] = []
    summary_only: list[tuple[dict[str, Any], dict[str, Any] | None]] = []
    denied_count = 0
    for finding in findings:
        decision = decisions["by_id"].get(finding["id"])
        action = action_for_finding(finding, decision)
        if action in INLINE_ACTIONS:
            allowed.append((finding, decision))
        elif action in SUMMARY_ACTIONS:
            summary_only.append((finding, decision))
        else:
            denied_count += 1

    comments: list[dict[str, Any]] = []
    unplaced: list[tuple[dict[str, Any], dict[str, Any] | None]] = []
    published_root_causes: set[str] = set()
    per_file_counts: dict[str, int] = {}
    for finding, decision in allowed:
        file_path = finding["file"]
        line = finding["line"]
        root_cause_key = root_cause_marker_key(finding, decision)
        if (
            isinstance(file_path, str)
            and isinstance(line, int)
            and line in changed_by_file.get(file_path, set())
            and root_cause_key not in published_root_causes
            and per_file_counts.get(file_path, 0) < MAX_INLINE_COMMENTS_PER_FILE
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
            published_root_causes.add(root_cause_key)
            per_file_counts[file_path] = per_file_counts.get(file_path, 0) + 1
        else:
            unplaced.append((finding, decision))
    unplaced.extend(summary_only)

    judgment = decisions.get("judgment") or {}
    blocking = bool(allowed)
    event = "REQUEST_CHANGES" if blocking or judgment.get("status") == "NEEDS_WORK" else "COMMENT"
    summary_body = render_current_body(
        event=event,
        allowed=allowed,
        denied_count=denied_count,
        unplaced=unplaced,
        decisions=decisions,
    )
    upsert_marker_comment(repo=repo, pr_number=pr_number, marker=REVIEW_SUMMARY_MARKER, body=summary_body)

    if comments or event == "REQUEST_CHANGES":
        payload = {
            "commit_id": head_sha,
            "event": event,
            "body": render_current_review_body(event),
            "comments": comments,
        }
        github_api(f"/repos/{repo}/pulls/{pr_number}/reviews", method="POST", payload=payload)
        print(f"posted {event} review with {len(comments)} inline comments and {len(unplaced)} unplaced findings")
    else:
        print(f"updated sticky review summary with {len(unplaced)} unplaced findings")


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
              comments(first: 51) {
                totalCount
                pageInfo { hasNextPage endCursor }
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
    redacted = body
    for pattern in SECRET_PATTERNS:
        redacted = re.sub(pattern, "[redacted]", redacted, flags=re.DOTALL)
    return redacted


def assert_no_secret_patterns(text: str, context: str) -> None:
    for pattern in SECRET_PATTERNS:
        if re.search(pattern, text, flags=re.DOTALL):
            raise SystemExit(f"{context} contains secret-like material")


def redact_comment_body(body: str) -> str:
    return trim_text(redact_secrets(body), 1200)


def extract_html_marker_value(body: str, name: str) -> str | None:
    match = re.search(rf"<!--\s*{re.escape(name)}:\s*([^>]*?)\s*-->", body)
    return match.group(1).strip() if match else None


def extract_marker_key(body: str) -> str | None:
    return extract_html_marker_value(body, "codex-review-id")


def extract_root_cause_metadata(body: str) -> dict[str, str]:
    metadata: dict[str, str] = {}
    key = extract_html_marker_value(body, "codex-root-cause-key")
    if key is not None:
        try:
            metadata["root_cause_key"] = normalize_root_cause_key(key, context="codex-root-cause-key marker")
            metadata["root_cause_key_source"] = "codex-root-cause-key"
        except SystemExit as exc:
            normalized_key = slug_key(key, fallback="")
            if normalized_key:
                metadata["root_cause_key"] = normalized_key
            metadata["root_cause_key_source"] = "invalid-codex-root-cause-key"
            metadata["root_cause_key_invalid_reason"] = str(exc)
    area = extract_html_marker_value(body, "codex-root-cause-area")
    if area:
        metadata["root_cause_area"] = slug_key(area, fallback="general")
    failure_kind = extract_html_marker_value(body, "codex-root-cause-failure-kind")
    if failure_kind:
        metadata["root_cause_failure_kind"] = slug_key(failure_kind, fallback="unknown")
    return metadata


def parse_thread_lifecycle_marker(body: str) -> dict[str, Any] | None:
    match = re.search(r"<!--\s*codex-thread-lifecycle:v3\s*(\{.*?\})?\s*-->", body, flags=re.DOTALL)
    if not match:
        return None
    raw = (match.group(1) or "{}").strip()
    try:
        payload = json.loads(raw) if raw else {}
    except json.JSONDecodeError:
        return {"state": "needs_human", "reason": "invalid lifecycle marker json"}
    state = str(payload.get("state") or "").strip()
    if state and state not in LIFECYCLE_STATES:
        payload["state"] = "needs_human"
        payload["reason"] = f"invalid lifecycle state: {state}"
    return payload


def thread_has_terminal_lifecycle_marker(thread: dict[str, Any]) -> bool:
    for comment in (thread.get("comments") or {}).get("nodes") or []:
        author = ((comment.get("author") or {}).get("login")) or ""
        if not is_trusted_codex_review_author(author):
            continue
        payload = parse_thread_lifecycle_marker(str(comment.get("body") or ""))
        if (
            payload
            and payload.get("state") in TERMINAL_LIFECYCLE_STATES
            and payload.get("resolved") is True
            and payload.get("resolved_at")
        ):
            return True
    return False


def comment_commit_oid(comment: dict[str, Any]) -> str:
    return str(((comment.get("commit") or {}).get("oid")) or "")


def comment_original_commit_oid(comment: dict[str, Any]) -> str:
    return str(((comment.get("originalCommit") or {}).get("oid")) or "")


def is_current_head_inline_comment(comment: dict[str, Any], head_sha: str) -> bool:
    original_oid = comment_original_commit_oid(comment)
    if original_oid:
        return original_oid == head_sha
    return comment_commit_oid(comment) == head_sha


def build_resolve_item(thread: dict[str, Any], comment: dict[str, Any]) -> dict[str, Any]:
    line = comment.get("line") or thread.get("line") or comment.get("originalLine") or thread.get("originalLine")
    try:
        line_int = int(line) if line is not None else None
    except (TypeError, ValueError):
        line_int = None
    file_path = comment.get("path") or thread.get("path")
    body = comment.get("body") or ""
    item = {
        "thread_id": thread["id"],
        "comment_node_id": comment["id"],
        "comment_id": int(comment["fullDatabaseId"]),
        "file": file_path,
        "line": line_int,
        "marker_key": extract_marker_key(body),
        "current_commit_oid": comment_commit_oid(comment) or None,
        "original_commit_oid": comment_original_commit_oid(comment) or None,
        "created_at": comment.get("createdAt") or "",
        "body_sha256": hashlib.sha256(str(body).encode("utf-8")).hexdigest(),
        "body_excerpt": redact_comment_body(body),
        "url": comment.get("url"),
    }
    item.update(extract_root_cause_metadata(body))
    return item


def root_cause_key_for_comment(item: dict[str, Any]) -> str:
    root_cause_key = str(item.get("root_cause_key") or "").strip()
    if root_cause_key:
        return root_cause_key
    marker_key = str(item.get("marker_key") or "").strip()
    if marker_key:
        return marker_key
    file_path = str(item.get("file") or "general")
    return thread_area(file_path)


def aggregate_thread_root_cause_metadata(
    comments: list[dict[str, Any]],
    thread: dict[str, Any],
) -> tuple[dict[str, Any], list[str]]:
    first = comments[0]
    first_file_path = first.get("file") or thread.get("path")
    metadata = {
        "file": first_file_path,
        "area": str(first.get("root_cause_area") or thread_area(str(first_file_path) if first_file_path else None)),
        "root_cause_key": root_cause_key_for_comment(first),
        "root_cause_key_source": str(first.get("root_cause_key_source") or "legacy-codex-review-id"),
    }
    if first.get("root_cause_failure_kind"):
        metadata["root_cause_failure_kind"] = str(first["root_cause_failure_kind"])

    needs_human_hints: list[str] = []
    valid_metadata: list[tuple[str, str, str]] = []
    has_missing_marker = False
    has_invalid_marker = False
    for comment in comments:
        source = str(comment.get("root_cause_key_source") or "legacy-codex-review-id")
        if source == "codex-root-cause-key":
            file_path = comment.get("file") or thread.get("path")
            area = str(comment.get("root_cause_area") or thread_area(str(file_path) if file_path else None))
            failure_kind = str(comment.get("root_cause_failure_kind") or "unknown")
            valid_metadata.append((str(comment["root_cause_key"]), area, failure_kind))
        elif source == "invalid-codex-root-cause-key":
            has_invalid_marker = True
        else:
            has_missing_marker = True

    if valid_metadata:
        root_cause_key, area, failure_kind = valid_metadata[0]
        metadata["root_cause_key"] = root_cause_key
        metadata["area"] = area
        metadata["root_cause_key_source"] = "codex-root-cause-key"
        metadata["root_cause_failure_kind"] = failure_kind
        if len(set(valid_metadata)) > 1:
            needs_human_hints.append("conflicting root-cause metadata; deferred issue handoff requires human review")
    if has_invalid_marker:
        needs_human_hints.append("invalid root-cause metadata; deferred issue handoff requires human review")
    if has_missing_marker:
        needs_human_hints.append("missing trusted root-cause metadata; deferred issue handoff requires human review")
    return metadata, needs_human_hints


def thread_area(path: str | None) -> str:
    if not path:
        return "general"
    if path.startswith("src/deposit_wallet/") or "deposit_wallet" in path:
        return "deposit-wallet"
    if path.startswith(".github/"):
        return "workflow"
    if path.startswith("docs/"):
        return "docs"
    if path.startswith("src/"):
        return "rust-api"
    return "general"


def comments_connection_has_more_than_limit(thread: dict[str, Any], limit: int = 50) -> bool:
    comments = thread.get("comments") or {}
    total = comments.get("totalCount")
    try:
        if total is not None and int(total) > limit:
            return True
    except (TypeError, ValueError):
        pass
    page_info = comments.get("pageInfo") or {}
    return bool(page_info.get("hasNextPage"))


def build_thread_lifecycle_inventory(threads: list[dict[str, Any]], *, head_sha: str) -> list[dict[str, Any]]:
    inventory: list[dict[str, Any]] = []
    for thread in threads:
        if thread.get("isResolved") or thread_has_terminal_lifecycle_marker(thread):
            continue
        comments = []
        for comment in (thread.get("comments") or {}).get("nodes") or []:
            body = comment.get("body") or ""
            author = ((comment.get("author") or {}).get("login")) or ""
            if INLINE_MARKER not in body:
                continue
            if not is_trusted_codex_review_author(author):
                continue
            if is_current_head_inline_comment(comment, head_sha):
                continue
            comments.append(build_resolve_item(thread, comment))
        if not comments:
            continue
        comments.sort(key=lambda item: int(item["comment_id"]))
        metadata, needs_human_hints = aggregate_thread_root_cause_metadata(comments, thread)
        item = {
            "thread_id": str(thread["id"]),
            "file": metadata["file"],
            "area": metadata["area"],
            "root_cause_key": metadata["root_cause_key"],
            "root_cause_key_source": metadata["root_cause_key_source"],
            "comments": comments,
        }
        if metadata.get("root_cause_failure_kind"):
            item["root_cause_failure_kind"] = metadata["root_cause_failure_kind"]
        if comments_connection_has_more_than_limit(thread):
            needs_human_hints.append("thread has more than 50 comments; GitHub comments connection may be incomplete")
        if needs_human_hints:
            item["forced_state"] = "needs_human"
            item["needs_human_hint"] = "; ".join(needs_human_hints)
        inventory.append(item)
    return sorted(
        inventory,
        key=lambda item: (
            str(item.get("area") or ""),
            str(item.get("root_cause_key") or ""),
            str(item.get("file") or ""),
            str(item.get("thread_id") or ""),
        ),
    )


def flatten_batch_comments(threads: list[dict[str, Any]]) -> list[dict[str, Any]]:
    return [comment for thread in threads for comment in thread.get("comments", [])]


def plan_thread_lifecycle_batches(inventory: list[dict[str, Any]]) -> list[dict[str, Any]]:
    batches: list[dict[str, Any]] = []
    current: list[dict[str, Any]] = []
    current_chars = 0

    def emit() -> None:
        nonlocal current, current_chars
        if not current:
            return
        threads = current
        batches.append(
            {
                "schema_version": THREAD_LIFECYCLE_BATCH_SCHEMA,
                "threads": threads,
                "comments": flatten_batch_comments(threads),
            }
        )
        current = []
        current_chars = 0

    for thread in inventory:
        thread_chars = len(json.dumps(thread, ensure_ascii=False, sort_keys=True))
        should_split = (
            len(current) >= THREAD_LIFECYCLE_BATCH_SIZE
            or len(current) >= THREAD_LIFECYCLE_HARD_MAX
            or (current and current_chars + thread_chars > THREAD_LIFECYCLE_BATCH_CHAR_BUDGET)
        )
        if should_split:
            emit()
        current.append(thread)
        current_chars += thread_chars
    emit()
    return batches


def batch_filename(index: int) -> str:
    return f"batch-{index:03d}.json"


def result_filename(index: int) -> str:
    return f"result-{index:03d}.json"


def thread_snapshot_metadata(thread: dict[str, Any]) -> dict[str, Any]:
    comments = thread.get("comments") or []
    created_values = [str(comment.get("created_at") or "") for comment in comments if comment.get("created_at")]
    return {
        "file": thread.get("file"),
        "area": thread.get("area"),
        "root_cause_key": thread.get("root_cause_key"),
        "source_comment_node_ids": [
            str(comment.get("comment_node_id"))
            for comment in comments
            if str(comment.get("comment_node_id") or "").strip()
        ],
        "source_comment_body_sha256": {
            str(comment.get("comment_node_id")): str(comment.get("body_sha256"))
            for comment in comments
            if str(comment.get("comment_node_id") or "").strip() and str(comment.get("body_sha256") or "").strip()
        },
        "latest_comment_created_at": max(created_values, default=""),
    }


def build_resolve_manifest(
    *,
    repo: str,
    pr_number: str,
    base_sha: str,
    head_sha: str,
    head_repo: str,
    upstream_run_id: str,
    upstream_run_attempt: str,
    batches: list[dict[str, Any]],
) -> dict[str, Any]:
    batch_items = []
    threads: dict[str, Any] = {}
    for index, batch in enumerate(batches):
        thread_ids = [str(thread["thread_id"]) for thread in batch.get("threads") or []]
        batch_items.append(
            {
                "index": index,
                "batch_filename": batch_filename(index),
                "result_filename": result_filename(index),
                "thread_ids": thread_ids,
            }
        )
        for thread in batch.get("threads") or []:
            threads[str(thread["thread_id"])] = thread_snapshot_metadata(thread)
    return {
        "schema_version": RESOLVE_MANIFEST_SCHEMA,
        "repository": repo,
        "pr_number": str(pr_number),
        "base_sha": base_sha,
        "head_sha": head_sha,
        "head_repo": head_repo,
        "upstream_run_id": upstream_run_id,
        "upstream_run_attempt": upstream_run_attempt,
        "batch_count": len(batches),
        "batches": batch_items,
        "expected_batch_filenames": [item["batch_filename"] for item in batch_items],
        "expected_result_filenames": [item["result_filename"] for item in batch_items],
        "threads": threads,
    }


def command_collect_resolutions(args: argparse.Namespace) -> None:
    repo = require_env("GITHUB_REPOSITORY")
    pr_number = require_env("PR_NUMBER")
    base_sha = require_env("BASE_SHA")
    head_sha = require_env("HEAD_SHA")
    head_repo = require_env("HEAD_REPO")
    upstream_run_id = os.environ.get("UPSTREAM_RUN_ID", "").strip()
    upstream_run_attempt = os.environ.get("UPSTREAM_RUN_ATTEMPT", "").strip()
    batch_dir = Path(args.batch_dir)
    batch_dir.mkdir(parents=True, exist_ok=True)
    for stale_json in batch_dir.glob("*.json"):
        stale_json.unlink()

    inventory = build_thread_lifecycle_inventory(collect_review_threads(repo, pr_number), head_sha=head_sha)
    batches = plan_thread_lifecycle_batches(inventory)
    manifest = build_resolve_manifest(
        repo=repo,
        pr_number=pr_number,
        base_sha=base_sha,
        head_sha=head_sha,
        head_repo=head_repo,
        upstream_run_id=upstream_run_id,
        upstream_run_attempt=upstream_run_attempt,
        batches=batches,
    )
    (batch_dir / "thread-inventory.v3.json").write_text(
        json.dumps({"schema_version": THREAD_INVENTORY_SCHEMA, "threads": inventory}, ensure_ascii=False, indent=2)
        + "\n",
        encoding="utf-8",
    )
    (batch_dir / "resolve-manifest.v3.json").write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )

    if not inventory:
        write_github_output({"has_comments": "false", "batch_indexes": "[]"})
        print("no previous Codex inline comments to resolve")
        return

    batch_indexes: list[int] = []
    for index, batch in enumerate(batches):
        batch["source_manifest_schema_version"] = RESOLVE_MANIFEST_SCHEMA
        batch["batch_index"] = index
        (batch_dir / batch_filename(index)).write_text(
            json.dumps(batch, ensure_ascii=False, indent=2) + "\n",
            encoding="utf-8",
        )
        batch_indexes.append(index)
    write_github_output({"has_comments": "true", "batch_indexes": json.dumps(batch_indexes)})
    comment_count = sum(len(item["comments"]) for item in inventory)
    print(f"collected {comment_count} previous Codex inline comments in {len(batch_indexes)} lifecycle batches")


def load_resolve_manifest(batches: Path) -> dict[str, Any]:
    manifest_path = batches / "resolve-manifest.v3.json"
    if not manifest_path.exists():
        raise SystemExit(f"missing resolve manifest: {manifest_path}")
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    if manifest.get("schema_version") != RESOLVE_MANIFEST_SCHEMA:
        raise SystemExit(f"invalid resolve manifest schema: {manifest.get('schema_version')}")
    if not isinstance(manifest.get("batches"), list):
        raise SystemExit("resolve manifest requires batches")
    expected_results = manifest.get("expected_result_filenames")
    if not isinstance(expected_results, list):
        raise SystemExit("resolve manifest requires expected_result_filenames")
    return manifest


def load_resolution_inputs(batches: Path, manifest: dict[str, Any] | None = None) -> dict[str, dict[str, Any]]:
    manifest = manifest or load_resolve_manifest(batches)
    expected_batches = {str(name) for name in manifest.get("expected_batch_filenames") or []}
    actual_batches = {path.name for path in batches.glob("batch-*.json") if path.is_file()}
    if actual_batches != expected_batches:
        raise SystemExit(f"batch artifact set mismatch: expected {sorted(expected_batches)}, got {sorted(actual_batches)}")
    if int(manifest.get("batch_count") or -1) != len(expected_batches):
        raise SystemExit(
            f"resolve manifest batch_count mismatch: expected {len(expected_batches)}, got {manifest.get('batch_count')}"
        )
    items: dict[str, dict[str, Any]] = {}
    for batch in manifest.get("batches") or []:
        path = batches / str(batch.get("batch_filename") or "")
        if not path.exists():
            raise SystemExit(f"missing resolve batch: {path}")
        payload = json.loads(path.read_text(encoding="utf-8"))
        if payload.get("schema_version") != THREAD_LIFECYCLE_BATCH_SCHEMA:
            raise SystemExit(f"invalid resolve batch schema in {path.name}: {payload.get('schema_version')}")
        if payload.get("source_manifest_schema_version") != RESOLVE_MANIFEST_SCHEMA:
            raise SystemExit(
                f"invalid source manifest schema in {path.name}: {payload.get('source_manifest_schema_version')}"
            )
        if int(payload.get("batch_index")) != int(batch.get("index")):
            raise SystemExit(f"batch index mismatch in {path.name}")
        for thread in payload.get("threads") or []:
            thread_id = str(thread["thread_id"])
            if thread_id in items:
                raise SystemExit(f"duplicate thread in resolve batches: {thread_id}")
            items[thread_id] = thread
    if not items:
        raise SystemExit("no resolve-check comments found")
    return items


def lifecycle_payloads_from_results(results: Path, manifest: dict[str, Any]) -> list[dict[str, Any]]:
    if not results.exists():
        raise SystemExit(f"missing resolve-check results directory: {results}")
    expected = {str(name) for name in manifest.get("expected_result_filenames") or []}
    actual = {path.name for path in results.glob("*.json") if path.is_file()}
    if not actual or actual != expected:
        raise SystemExit(f"result artifact set mismatch: expected {sorted(expected)}, got {sorted(actual)}")
    payloads = []
    batch_by_result = {str(batch["result_filename"]): batch for batch in manifest.get("batches") or []}
    for name in sorted(expected):
        path = results / name
        payload = json.loads(path.read_text(encoding="utf-8"))
        batch = batch_by_result.get(name)
        if not batch:
            raise SystemExit(f"result {name} is not declared in manifest batches")
        if payload.get("schema_version") != THREAD_LIFECYCLE_RESULT_SCHEMA:
            raise SystemExit(f"invalid lifecycle schema in {name}: {payload.get('schema_version')}")
        if payload.get("source_manifest_schema_version") != RESOLVE_MANIFEST_SCHEMA:
            raise SystemExit(
                f"invalid lifecycle source manifest schema in {name}: {payload.get('source_manifest_schema_version')}"
            )
        if int(payload.get("batch_index")) != int(batch.get("index")):
            raise SystemExit(f"result batch index mismatch in {name}")
        expected_thread_ids = {str(thread_id) for thread_id in batch.get("thread_ids") or []}
        actual_thread_ids = {str(item.get("thread_id") or "") for item in payload.get("threads") or []}
        if actual_thread_ids != expected_thread_ids:
            raise SystemExit(
                f"lifecycle thread ids mismatch in {name}: expected {sorted(expected_thread_ids)}, got {sorted(actual_thread_ids)}"
            )
        payloads.append(payload)
    return payloads


def normalize_lifecycle_outputs(payloads: list[dict[str, Any]], expected_thread_ids: set[str]) -> dict[str, dict[str, Any]]:
    decisions: dict[str, dict[str, Any]] = {}
    for payload in payloads:
        if payload.get("schema_version") != THREAD_LIFECYCLE_RESULT_SCHEMA:
            raise SystemExit(f"invalid lifecycle schema: {payload.get('schema_version')}")
        for raw in payload.get("threads") or []:
            if not isinstance(raw, dict):
                raise SystemExit("lifecycle thread decision must be an object")
            thread_id = str(raw.get("thread_id") or "")
            state = str(raw.get("state") or "")
            if thread_id in decisions:
                raise SystemExit(f"duplicate lifecycle decision for thread {thread_id}")
            if state not in LIFECYCLE_STATES:
                raise SystemExit(f"unknown lifecycle state for {thread_id}: {state}")
            reason = trim_text(raw.get("reason"), 300).strip()
            evidence = trim_text(raw.get("evidence"), 500).strip()
            if not reason:
                raise SystemExit(f"{thread_id} lifecycle decision requires reason")
            if state in {"defer_to_issue", "duplicate_of_issue", "false_positive", "stale_obsolete"} and not evidence:
                raise SystemExit(f"{thread_id} lifecycle decision requires evidence")
            issue = raw.get("issue") if isinstance(raw.get("issue"), dict) else None
            issue_url = str(raw.get("issue_url") or "").strip()
            if state == "defer_to_issue":
                if not issue or not str(issue.get("key") or "").strip():
                    raise SystemExit(f"{thread_id} defer_to_issue requires issue.key")
                if not str(issue.get("title") or "").strip() or not str(issue.get("body") or "").strip():
                    raise SystemExit(f"{thread_id} defer_to_issue requires issue title and body")
            if state == "duplicate_of_issue" and not issue_url:
                raise SystemExit(f"{thread_id} duplicate_of_issue requires issue_url")
            decisions[thread_id] = {
                "thread_id": thread_id,
                "state": state,
                "reason": reason,
                "evidence": evidence,
                "issue": issue,
                "issue_url": issue_url,
            }
    actual_ids = set(decisions)
    if actual_ids != expected_thread_ids:
        raise SystemExit(f"lifecycle thread ids mismatch: expected {sorted(expected_thread_ids)}, got {sorted(actual_ids)}")
    return decisions


def load_resolution_outputs(results: Path, manifest: dict[str, Any], expected_ids: set[str]) -> dict[str, dict[str, Any]]:
    lifecycle_payloads = lifecycle_payloads_from_results(results, manifest)
    return normalize_lifecycle_outputs(lifecycle_payloads, expected_ids)


def resolve_thread(thread_id: str) -> None:
    mutation = """
    mutation($threadId: ID!) {
      resolveReviewThread(input: {threadId: $threadId}) {
        thread { id isResolved }
      }
    }
    """
    github_graphql(mutation, {"threadId": thread_id})


def try_resolve_thread(thread_id: str) -> str | None:
    try:
        resolve_thread(thread_id)
    except SystemExit as exc:
        return str(exc)
    return None


def reply_to_review_thread(thread_id: str, body: str) -> None:
    mutation = """
    mutation($threadId: ID!, $body: String!) {
      addPullRequestReviewThreadReply(input: {pullRequestReviewThreadId: $threadId, body: $body}) {
        comment { id }
      }
    }
    """
    github_graphql(mutation, {"threadId": thread_id, "body": body})


def try_reply_to_review_thread(thread_id: str, body: str) -> str | None:
    try:
        reply_to_review_thread(thread_id, body)
    except SystemExit as exc:
        return str(exc)
    return None


def issue_body_with_marker(request: dict[str, Any]) -> str:
    key = str(request["key"])
    machine = {
        "schema_version": "codex.issue.v3",
        "idempotency_key": key,
        "source_pr": os.environ.get("PR_NUMBER", ""),
        "root_cause": request.get("root_cause") or {},
        "source_threads": request.get("source_threads") or [],
    }
    body = redact_secrets(str(request.get("body") or ""))
    return "\n".join(
        [
            f"<!-- {ISSUE_KEY_MARKER} {key} -->",
            "<!-- codex-issue-schema: v3 -->",
            body,
            "",
            "## Machine-readable",
            "```json",
            redact_secrets(json.dumps(machine, ensure_ascii=False, indent=2, sort_keys=True)),
            "```",
        ]
    )


def find_issue_by_key(repo: str, key: str) -> dict[str, Any] | None:
    marker = f"{ISSUE_KEY_MARKER} {key}"
    issues = github_paginated(f"/repos/{repo}/issues?state=all&per_page=100")
    for issue in issues:
        if "pull_request" in issue:
            continue
        if marker in str(issue.get("body") or ""):
            return issue
    return None


def trusted_issue_key_for_thread(repo: str, thread: dict[str, Any]) -> str:
    root = re.sub(r"[^a-z0-9-]+", "-", str(thread.get("root_cause_key") or "thread").lower()).strip("-")
    if not root:
        root = "thread"
    failure_kind = str(thread.get("root_cause_failure_kind") or "")
    seed = "|".join(
        [
            "codex-v3",
            repo,
            str(thread.get("area") or ""),
            failure_kind,
            root,
        ]
    )
    digest = hashlib.sha256(seed.encode("utf-8")).hexdigest()[:20]
    return trim_text(f"{root}-{digest}", 80).replace("\n", "")


def trusted_deferred_issue_request(repo: str, thread: dict[str, Any], request: dict[str, Any]) -> dict[str, Any]:
    trusted = dict(request)
    trusted["key"] = trusted_issue_key_for_thread(repo, thread)
    trusted["source_threads"] = [thread.get("thread_id")]
    trusted["root_cause"] = {
        "key": thread.get("root_cause_key"),
        "area": thread.get("area"),
        "failure_kind": thread.get("root_cause_failure_kind"),
        "file": thread.get("file"),
    }
    labels = [str(label) for label in (request.get("labels") or []) if str(label).strip()]
    if "codex/deferred" not in labels:
        labels.insert(0, "codex/deferred")
    trusted["labels"] = labels
    return trusted


def parse_same_repo_issue_number(repo: str, issue_url: str) -> str | None:
    owner, name = split_repo(repo)
    pattern = rf"^https://github\.com/{re.escape(owner)}/{re.escape(name)}/issues/([0-9]+)(?:[#?].*)?$"
    match = re.match(pattern, issue_url.strip())
    return match.group(1) if match else None


def get_same_repo_issue_by_url(repo: str, issue_url: str) -> dict[str, Any] | None:
    number = parse_same_repo_issue_number(repo, issue_url)
    if not number:
        return None
    return github_api(f"/repos/{repo}/issues/{number}")


def is_codex_deferred_issue(issue: dict[str, Any]) -> bool:
    if "pull_request" in issue or issue.get("state") != "open":
        return False
    labels: set[str] = set()
    for label in issue.get("labels") or []:
        if isinstance(label, dict):
            name = label.get("name")
        else:
            name = label
        if name:
            labels.add(str(name))
    body = str(issue.get("body") or "")
    return "codex/deferred" in labels or ISSUE_KEY_MARKER in body


def extract_issue_source_threads(body: str) -> list[str]:
    marker_index = body.rfind("## Machine-readable")
    if marker_index < 0:
        return []
    match = re.search(r"```json\s*(\{.*?\})\s*```", body[marker_index:], flags=re.DOTALL)
    if not match:
        return []
    try:
        machine = json.loads(match.group(1))
    except json.JSONDecodeError:
        return []
    source_threads = machine.get("source_threads")
    if not isinstance(source_threads, list):
        return []
    return [str(item) for item in source_threads if str(item).strip()]


def merge_existing_issue_source_threads(request: dict[str, Any], existing: dict[str, Any]) -> dict[str, Any]:
    merged = list(
        dict.fromkeys(
            [
                *extract_issue_source_threads(str(existing.get("body") or "")),
                *[str(item) for item in (request.get("source_threads") or []) if str(item).strip()],
            ]
        )
    )
    updated = dict(request)
    updated["source_threads"] = merged
    return updated


def create_or_update_deferred_issue(*, repo: str, request: dict[str, Any]) -> dict[str, Any]:
    key = str(request.get("key") or "").strip()
    if not key:
        raise SystemExit("deferred issue request requires key")
    existing = find_issue_by_key(repo, key)
    if existing:
        if existing.get("state") == "closed":
            return existing
        request = merge_existing_issue_source_threads(request, existing)
        body = issue_body_with_marker(request)
        return github_api(
            f"/repos/{repo}/issues/{existing['number']}",
            method="PATCH",
            payload={"title": redact_secrets(str(request["title"])), "body": body},
        )
    body = issue_body_with_marker(request)
    payload = {
        "title": redact_secrets(str(request["title"])),
        "body": body,
        "labels": request.get("labels") or ["codex/deferred"],
    }
    return github_api(f"/repos/{repo}/issues", method="POST", payload=payload)


def render_lifecycle_reply(
    *,
    thread: dict[str, Any],
    decision: dict[str, Any],
    issue_url: str | None = None,
) -> str:
    reason = redact_secrets(str(decision["reason"]))
    evidence = redact_secrets(str(decision.get("evidence") or ""))
    marker_payload = {
        "state": decision["state"],
        "reason": reason,
        "resolved": False,
    }
    if issue_url:
        marker_payload["issue_url"] = issue_url
    marker = f"<!-- {THREAD_LIFECYCLE_MARKER} {json.dumps(marker_payload, ensure_ascii=False, sort_keys=True)} -->"
    lines = [
        marker,
        f"Codex thread lifecycle: `{decision['state']}`",
        "",
        f"- reason: {reason}",
    ]
    if evidence:
        lines.append(f"- evidence: {evidence}")
    if issue_url:
        lines.append(f"- issue: {issue_url}")
    if thread.get("comments"):
        lines.append(f"- source comments: {len(thread.get('comments') or [])}")
    return "\n".join(lines)


def validate_current_pr_head(repo: str, pr_number: str, manifest: dict[str, Any]) -> None:
    pr = github_api(f"/repos/{repo}/pulls/{pr_number}")
    current_head = str(((pr.get("head") or {}).get("sha")) or "")
    expected_head = str(manifest.get("head_sha") or "")
    if current_head != expected_head:
        raise SystemExit(f"PR head SHA changed since collection: expected {expected_head}, got {current_head}")
    current_head_repo = str((((pr.get("head") or {}).get("repo") or {}).get("full_name")) or "")
    expected_head_repo = str(manifest.get("head_repo") or "")
    if expected_head_repo and current_head_repo != expected_head_repo:
        raise SystemExit(f"PR head repo changed since collection: expected {expected_head_repo}, got {current_head_repo}")


def validate_resolution_artifact_contract(
    *,
    repo: str,
    pr_number: str,
    batches: Path,
    results: Path,
    check_current_head: bool,
) -> tuple[dict[str, Any], dict[str, dict[str, Any]], dict[str, dict[str, Any]]]:
    manifest = load_resolve_manifest(batches)
    if str(manifest.get("repository") or "") != repo:
        raise SystemExit(f"resolve manifest repository mismatch: {manifest.get('repository')} != {repo}")
    if str(manifest.get("pr_number") or "") != str(pr_number):
        raise SystemExit(f"resolve manifest PR mismatch: {manifest.get('pr_number')} != {pr_number}")
    inputs = load_resolution_inputs(batches, manifest)
    expected_ids = set(inputs)
    decisions = load_resolution_outputs(results, manifest, expected_ids)
    if check_current_head:
        validate_current_pr_head(repo, pr_number, manifest)
    return manifest, inputs, decisions


def current_thread_lookup(repo: str, pr_number: str) -> dict[str, dict[str, Any]]:
    return {str(thread.get("id")): thread for thread in collect_review_threads(repo, pr_number)}


def comment_lookup(thread: dict[str, Any]) -> dict[str, dict[str, Any]]:
    return {
        str(comment.get("id")): comment
        for comment in (thread.get("comments") or {}).get("nodes") or []
        if str(comment.get("id") or "").strip()
    }


def mark_thread_needs_human(thread: dict[str, Any], reason: str) -> None:
    thread["forced_state"] = "needs_human"
    existing = str(thread.get("needs_human_hint") or "").strip()
    thread["needs_human_hint"] = f"{existing}; {reason}" if existing else reason


def revalidate_thread_snapshots(
    *,
    repo: str,
    pr_number: str,
    threads: dict[str, dict[str, Any]],
) -> None:
    current_threads = current_thread_lookup(repo, pr_number)
    for thread_id, thread in threads.items():
        current = current_threads.get(thread_id)
        if not current:
            mark_thread_needs_human(thread, "source review thread is no longer visible")
            continue
        if current.get("isResolved"):
            thread["_already_resolved"] = True
            continue
        current_comments = comment_lookup(current)
        source_comments = thread.get("comments") or []
        latest_source_created_at = max(
            [str(comment.get("created_at") or "") for comment in source_comments if comment.get("created_at")],
            default="",
        )
        for source_comment in source_comments:
            node_id = str(source_comment.get("comment_node_id") or "")
            if not node_id:
                continue
            current_comment = current_comments.get(node_id)
            if not current_comment:
                mark_thread_needs_human(thread, "source review comment disappeared before apply")
                continue
            expected_body_sha = str(source_comment.get("body_sha256") or "")
            if expected_body_sha:
                actual_body_sha = hashlib.sha256(str(current_comment.get("body") or "").encode("utf-8")).hexdigest()
                if actual_body_sha != expected_body_sha:
                    mark_thread_needs_human(thread, "source review comment body changed before apply")
        if latest_source_created_at:
            for comment in current_comments.values():
                created_at = str(comment.get("createdAt") or "")
                author = str(((comment.get("author") or {}).get("login")) or "")
                if created_at > latest_source_created_at and not is_trusted_codex_review_author(author):
                    mark_thread_needs_human(thread, "human comment was added after lifecycle collection")


def command_validate_resolution_artifacts(args: argparse.Namespace) -> None:
    repo = require_env("GITHUB_REPOSITORY")
    pr_number = require_env("PR_NUMBER")
    _manifest, inputs, _decisions = validate_resolution_artifact_contract(
        repo=repo,
        pr_number=pr_number,
        batches=Path(args.batches),
        results=Path(args.results),
        check_current_head=True,
    )
    revalidate_thread_snapshots(repo=repo, pr_number=pr_number, threads=inputs)
    forced = [
        f"{thread_id}: {thread.get('needs_human_hint')}"
        for thread_id, thread in inputs.items()
        if thread.get("forced_state") == "needs_human"
    ]
    if forced:
        raise SystemExit("thread snapshot validation failed: " + "; ".join(forced[:10]))
    print("resolve-check manifest and results are complete")


def command_post_resolve_failure(args: argparse.Namespace) -> None:
    del args
    repo = require_env("GITHUB_REPOSITORY")
    pr_number = os.environ.get("PR_NUMBER", "").strip()
    lines = [
        RESOLVE_MARKER,
        "Codex thread lifecycle apply를 건너뛰었습니다.",
        "",
        "- 이벤트: REQUEST_CHANGES",
        "- terminal 처리된 스레드: 0",
        "- 열어둔 스레드: workflow failure",
        "",
        "원인:",
        f"- validate-upstream: {os.environ.get('VALIDATE_UPSTREAM_RESULT', '')}",
        f"- should_collect: {os.environ.get('VALIDATE_UPSTREAM_SHOULD_COLLECT', '')}",
        f"- collect: {os.environ.get('COLLECT_RESULT', '')}",
        f"- resolve-check: {os.environ.get('RESOLVE_CHECK_RESULT', '')}",
        f"- apply: {os.environ.get('APPLY_RESULT', '')}",
        "",
        "조치:",
        "- upstream run, broker token exchange, resolve-check artifacts, App permission 상태를 확인해야 합니다.",
        "- 이 reporter는 App token을 만들지 않았고 thread reply/resolve나 deferred issue 생성을 시도하지 않았습니다.",
    ]
    if not pr_number:
        print("resolve-check failure summary skipped because PR_NUMBER is empty")
        print("\n".join(lines))
        return
    upsert_marker_comment(repo=repo, pr_number=pr_number, marker=RESOLVE_MARKER, body="\n".join(lines))
    print("posted resolve-check failure summary")


def command_apply_resolutions(args: argparse.Namespace) -> None:
    repo = require_env("GITHUB_REPOSITORY")
    pr_number = require_env("PR_NUMBER")
    _manifest, inputs, resolutions = validate_resolution_artifact_contract(
        repo=repo,
        pr_number=pr_number,
        batches=Path(args.batches),
        results=Path(args.results),
        check_current_head=True,
    )
    revalidate_thread_snapshots(repo=repo, pr_number=pr_number, threads=inputs)
    apply_lifecycle_resolutions(repo=repo, pr_number=pr_number, threads=inputs, decisions=resolutions)


def apply_lifecycle_resolutions(
    *,
    repo: str,
    pr_number: str,
    threads: dict[str, dict[str, Any]],
    decisions: dict[str, dict[str, Any]],
) -> None:
    resolved: list[tuple[dict[str, Any], dict[str, Any]]] = []
    unresolved: list[tuple[dict[str, Any], dict[str, Any]]] = []
    no_issue_terminal = {"resolved_by_code", "false_positive", "stale_obsolete"}

    for thread_id, thread in threads.items():
        decision = decisions[thread_id]
        state = decision["state"]
        if thread.get("_already_resolved"):
            continue
        if thread.get("forced_state") == "needs_human":
            unresolved.append(
                (
                    thread,
                    {
                        **decision,
                        "state": "needs_human",
                        "reason": str(thread.get("needs_human_hint") or "trusted collector forced needs_human"),
                    },
                )
            )
            continue
        if state in {"fix_now", "needs_human"}:
            unresolved.append((thread, decision))
            continue

        issue_url = decision.get("issue_url") or ""
        if state == "defer_to_issue":
            issue_request = decision.get("issue") or {}
            if not issue_request:
                unresolved.append((thread, {**decision, "reason": "defer_to_issue requires issue request"}))
                continue
            trusted_request = trusted_deferred_issue_request(repo, thread, issue_request)
            try:
                issue = create_or_update_deferred_issue(repo=repo, request=trusted_request)
            except SystemExit as exc:
                unresolved.append((thread, {**decision, "reason": f"deferred issue handling failed: {exc}"}))
                continue
            if issue.get("state") == "closed":
                unresolved.append(
                    (
                        thread,
                        {
                            **decision,
                            "reason": "matching deferred issue is closed; needs human decision before resolving",
                        },
                    )
                )
                continue
            issue_url = str(issue.get("html_url") or issue_url)
            if not issue_url:
                unresolved.append((thread, {**decision, "reason": "issue url missing after deferred issue handling"}))
                continue
        elif state == "duplicate_of_issue":
            if not issue_url:
                unresolved.append((thread, {**decision, "reason": "duplicate_of_issue requires issue_url"}))
                continue
            try:
                issue = get_same_repo_issue_by_url(repo, issue_url)
            except SystemExit as exc:
                unresolved.append((thread, {**decision, "reason": f"duplicate issue lookup failed: {exc}"}))
                continue
            if not issue:
                unresolved.append((thread, {**decision, "reason": "duplicate issue_url must point to this repository"}))
                continue
            if issue.get("state") == "closed":
                unresolved.append(
                    (
                        thread,
                        {
                            **decision,
                            "state": "needs_human",
                            "reason": "duplicate issue is closed; needs human decision before resolving",
                        },
                    )
                )
                continue
            if not is_codex_deferred_issue(issue):
                unresolved.append(
                    (
                        thread,
                        {
                            **decision,
                            "state": "needs_human",
                            "reason": "duplicate issue_url must point to an open Codex deferred issue, not a PR or general issue",
                        },
                    )
                )
                continue
        elif state not in no_issue_terminal:
            unresolved.append((thread, decision))
            continue

        reply_error = try_reply_to_review_thread(
            thread_id,
            render_lifecycle_reply(thread=thread, decision=decision, issue_url=issue_url or None),
        )
        if reply_error:
            unresolved.append((thread, {**decision, "reason": f"thread reply failed: {reply_error}"}))
            continue
        error = try_resolve_thread(thread_id)
        if error:
            unresolved.append((thread, {**decision, "reason": f"thread resolve failed: {error}"}))
            continue
        resolved.append((thread, decision))

    event = "REQUEST_CHANGES" if unresolved else "COMMENT"
    body = render_lifecycle_resolution_body(event=event, resolved=resolved, unresolved=unresolved)
    upsert_marker_comment(repo=repo, pr_number=pr_number, marker=RESOLVE_MARKER, body=body)
    print(f"updated sticky lifecycle summary; resolved={len(resolved)} unresolved={len(unresolved)}")


def render_lifecycle_resolution_body(
    *,
    event: str,
    resolved: list[tuple[dict[str, Any], dict[str, Any]]],
    unresolved: list[tuple[dict[str, Any], dict[str, Any]]],
) -> str:
    lines = [
        RESOLVE_MARKER,
        "Codex thread lifecycle 확인이 완료되었습니다.",
        "",
        f"- 이벤트: {event}",
        f"- terminal 처리된 스레드: {len(resolved)}",
        f"- 열어둔 스레드: {len(unresolved)}",
    ]
    if unresolved:
        lines.extend(["", "열어둔 스레드:"])
        for thread, decision in unresolved[:25]:
            location = thread.get("file") or "일반"
            reason = redact_secrets(str(decision.get("reason", "")))
            lines.append(f"- {location} - `{decision.get('state')}` {reason}".rstrip())
    if resolved:
        lines.extend(["", "이번에 terminal 처리됨:"])
        for thread, decision in resolved[:25]:
            location = thread.get("file") or "일반"
            reason = redact_secrets(str(decision.get("reason", "")))
            lines.append(f"- {location} - `{decision.get('state')}` {reason}".rstrip())
    return "\n".join(lines)


def latest_marker_comment(comments: list[dict[str, Any]], marker: str) -> dict[str, Any] | None:
    matches = [comment for comment in comments if marker in str(comment.get("body") or "")]
    if not matches:
        return None
    return sorted(matches, key=lambda item: str(item.get("updated_at") or item.get("created_at") or ""))[-1]


def upsert_marker_comment(
    *,
    repo: str,
    pr_number: str,
    marker: str,
    body: str,
    list_comments: Any = github_paginated,
    api: Any = github_api,
) -> str:
    comments = list_comments(f"/repos/{repo}/issues/{pr_number}/comments?per_page=100")
    existing = latest_marker_comment(comments, marker)
    if existing:
        api(f"/repos/{repo}/issues/comments/{existing['id']}", method="PATCH", payload={"body": body})
        return "updated"
    api(f"/repos/{repo}/issues/{pr_number}/comments", method="POST", payload={"body": body})
    return "created"


def review_marker_kind(body: str) -> str | None:
    if REVIEW_SUMMARY_MARKER in body:
        return "review-summary"
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
    latest_review_summary = latest_marker_comment(issue_comments, REVIEW_SUMMARY_MARKER)
    latest_resolve_summary = latest_marker_comment(issue_comments, RESOLVE_MARKER)
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
    sticky_summaries = [
        ("review-summary", latest_review_summary),
        ("resolve", latest_resolve_summary),
    ]
    for marker_kind, comment in sticky_summaries:
        if not comment:
            continue
        author = ((comment.get("user") or {}).get("login")) or ""
        updated_at = comment.get("updated_at") or comment.get("created_at") or ""
        lines.extend(
            [
                f"### sticky-{marker_kind} {updated_at}",
                "",
                f"- author: {author}",
                "",
                trim_text(redact_comment_body(str(comment.get("body") or "")), 1800),
                "",
            ]
        )
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
    elif not any(comment for _, comment in sticky_summaries):
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


def design_blockers(findings: list[dict[str, Any]], decisions: dict[str, Any]) -> list[dict[str, Any]]:
    blockers: list[dict[str, Any]] = []
    for finding in findings:
        decision = decisions["by_id"].get(finding["id"])
        if decision and decision.get("action") in {"publish_and_fix_now", "summary_only_fix_now"}:
            blockers.append(finding)
    return blockers


def should_run_design(findings: list[dict[str, Any]], decisions: dict[str, Any]) -> tuple[bool, int]:
    judgment = decisions.get("judgment") or {}
    blockers = design_blockers(findings, decisions)
    return judgment.get("status") == "NEEDS_WORK" or bool(blockers), len(blockers)


def command_classify_design_need(args: argparse.Namespace) -> None:
    findings = load_current_findings(Path(args.artifacts))
    decisions = load_decisions(Path(args.decisions))
    validate_decision_coverage(findings, decisions)
    needs_design, blocking_count = should_run_design(findings, decisions)
    write_github_output({"needs_design": "true" if needs_design else "false", "blocking_count": str(blocking_count)})
    print(f"needs_design={needs_design} blocking_count={blocking_count}")


def design_plan_list(plan: dict[str, Any], key: str) -> list[str]:
    value = plan.get(key)
    if not isinstance(value, list):
        return []
    return [trim_text(redact_secrets(str(item)), 600).strip() for item in value if str(item).strip()]


def render_design_plan_body(plan: dict[str, Any]) -> str:
    summary = trim_text(redact_secrets(str(plan.get("summary") or "")), 800).strip() or "설계 요약이 제공되지 않았습니다."
    root_cause = (
        trim_text(redact_secrets(str(plan.get("root_cause") or "")), 1200).strip()
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
            redact_secrets(json.dumps(plan, ensure_ascii=False, indent=2)),
            "```",
        ]
    )
    return "\n".join(lines).rstrip() + "\n"


def command_render_design_plan(args: argparse.Namespace) -> None:
    plan = json.loads(Path(args.plan).read_text(encoding="utf-8"))
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
    return upsert_marker_comment(
        repo=repo,
        pr_number=pr_number,
        marker=DESIGN_MARKER,
        body=body,
        list_comments=list_comments,
        api=api,
    )


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

    plan_autofix = subparsers.add_parser("plan-autofix")
    plan_autofix.add_argument("--artifacts", required=True)
    plan_autofix.add_argument("--decisions", required=True)
    plan_autofix.add_argument("--output", required=True)
    plan_autofix.set_defaults(func=command_plan_autofix)

    validate_autofix = subparsers.add_parser("validate-autofix-patch")
    validate_autofix.add_argument("--manifest", required=True)
    validate_autofix.add_argument("--patch", required=True)
    validate_autofix.set_defaults(func=command_validate_autofix_patch)

    resolve_current = subparsers.add_parser("resolve-current")
    resolve_current.set_defaults(func=command_resolve_current)

    resolve_previous = subparsers.add_parser("resolve-previous")
    resolve_previous.set_defaults(func=command_resolve_previous)

    collect = subparsers.add_parser("collect-resolutions")
    collect.add_argument("--batch-dir", required=True)
    collect.set_defaults(func=command_collect_resolutions)

    apply = subparsers.add_parser("apply-resolutions")
    apply.add_argument("--batches", required=True)
    apply.add_argument("--results", required=True)
    apply.set_defaults(func=command_apply_resolutions)

    validate_resolutions = subparsers.add_parser("validate-resolution-artifacts")
    validate_resolutions.add_argument("--batches", required=True)
    validate_resolutions.add_argument("--results", required=True)
    validate_resolutions.set_defaults(func=command_validate_resolution_artifacts)

    failure = subparsers.add_parser("post-resolve-failure")
    failure.set_defaults(func=command_post_resolve_failure)

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
