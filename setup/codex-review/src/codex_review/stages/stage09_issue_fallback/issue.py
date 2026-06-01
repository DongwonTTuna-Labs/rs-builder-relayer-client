"""First-class issue fallback for work the PR loop cannot safely mutate."""
from __future__ import annotations

import hashlib
from typing import Any

from codex_review.github.issues import create_or_update_deferred_issue
from codex_review.github.markers import render_marker

SCHEMA_VERSION = "stage09-issue-fallback.v1"


def _repository(pr_context: dict[str, Any]) -> str:
    repository = pr_context.get("repository") or pr_context.get("base_repo_full_name")
    if repository:
        return str(repository)
    return f"{pr_context.get('owner')}/{pr_context.get('repo')}"


def _pr_reference(pr_context: dict[str, Any]) -> str:
    if pr_context.get("html_url"):
        return str(pr_context["html_url"])
    number = pr_context.get("pr_number")
    return f"#{number}" if number else "unknown PR"


def _key_material(reason: str, pr_context: dict[str, Any], openspec_context: dict[str, Any]) -> str:
    sources = ",".join(str(x) for x in openspec_context.get("source_summary", []))
    return f"{_repository(pr_context)}#{pr_context.get('pr_number')}:{reason}:{sources}"


def make_issue_fallback_key(reason: str, pr_context: dict[str, Any], openspec_context: dict[str, Any]) -> str:
    return hashlib.sha256(_key_material(reason, pr_context, openspec_context).encode("utf-8")).hexdigest()[:24]


def render_issue_fallback_body(
    *,
    idempotency_key: str,
    reason: str,
    pr_context: dict[str, Any],
    openspec_context: dict[str, Any],
    attempted_stages: list[str],
    required_follow_up: str,
) -> str:
    lines = [
        render_marker("codex-review:issue-fallback", {"key": idempotency_key, "reason": reason}),
        "# Codex review fallback",
        "",
        f"Reason: `{reason}`",
        f"Source PR: {_pr_reference(pr_context)}",
        "",
        "## OpenSpec source",
    ]
    sources = openspec_context.get("source_summary") or []
    if sources:
        lines.extend(f"- {source}" for source in sources)
    else:
        lines.append(f"- `{openspec_context.get('decision') or 'missing_openspec_spec'}`")
    lines.extend(["", "## Attempted stages"])
    lines.extend(f"- {stage}" for stage in attempted_stages or ["unknown"])
    lines.extend(["", "## Required follow-up", required_follow_up])
    return "\n".join(lines)


def build_issue_fallback_plan(
    *,
    reason: str,
    pr_context: dict[str, Any],
    openspec_context: dict[str, Any],
    attempted_stages: list[str] | None = None,
    required_follow_up: str | None = None,
) -> dict[str, Any]:
    key = make_issue_fallback_key(reason, pr_context, openspec_context)
    follow_up = required_follow_up or _default_follow_up(reason, openspec_context)
    body = render_issue_fallback_body(
        idempotency_key=key,
        reason=reason,
        pr_context=pr_context,
        openspec_context=openspec_context,
        attempted_stages=attempted_stages or [],
        required_follow_up=follow_up,
    )
    return {
        "schema_version": SCHEMA_VERSION,
        "status": "planned",
        "reason": reason,
        "idempotency_key": key,
        "title": f"Codex review fallback: {reason}",
        "body": body,
        "required_follow_up": follow_up,
        "attempted_stages": attempted_stages or [],
        "openspec_sources": openspec_context.get("source_summary") or [],
    }


def _default_follow_up(reason: str, openspec_context: dict[str, Any]) -> str:
    if reason in {"missing_openspec_spec", "unresolved_openspec_source"} or not openspec_context.get("present"):
        return "Add or link the OpenSpec change artifacts in the PR title/body, then rerun Codex Review."
    if reason in {"fork_pr_push_blocked", "out_of_scope"}:
        return "Move the implementation into a same-repository branch or handle the out-of-scope work in a separate PR."
    if reason in {"no-diff-repeat", "empty_patch"}:
        return "Inspect the generated fix artifacts and adjust the OpenSpec tasks or implementation plan so the next run can produce a non-empty patch."
    return "Resolve the blocking condition, then rerun Codex Review."


def apply_issue_fallback(plan: dict[str, Any], pr_context: dict[str, Any], token: str | None, *, dry_run: bool) -> dict[str, Any]:
    owner = pr_context.get("owner")
    repo = pr_context.get("repo")
    if dry_run or not token:
        return {**plan, "status": "dry_run", "issue_url": None}
    result = create_or_update_deferred_issue(str(owner), str(repo), plan["idempotency_key"], plan["title"], plan["body"], token)
    return {**plan, "status": "applied", "issue_url": result.get("html_url") or result.get("url")}
