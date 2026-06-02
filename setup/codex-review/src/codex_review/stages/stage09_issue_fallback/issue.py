"""First-class issue fallback for work the PR loop cannot safely mutate."""
from __future__ import annotations

import hashlib
from typing import Any

from codex_review.errors import ValidationError
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


def _key_material(reason: str, pr_context: dict[str, Any], openspec_context: dict[str, Any], deferred_items: list[dict[str, Any]] | None = None) -> str:
    sources = ",".join(str(x) for x in openspec_context.get("source_summary", []))
    deferred = ",".join(
        str(item.get("root_cause_key") or item.get("finding_id") or item.get("id") or item.get("title") or "deferred")
        for item in (deferred_items or [])
    )
    return f"{_repository(pr_context)}#{pr_context.get('pr_number')}:{reason}:{sources}:{deferred}"


def make_issue_fallback_key(reason: str, pr_context: dict[str, Any], openspec_context: dict[str, Any], deferred_items: list[dict[str, Any]] | None = None) -> str:
    return hashlib.sha256(_key_material(reason, pr_context, openspec_context, deferred_items).encode("utf-8")).hexdigest()[:24]


def render_issue_fallback_body(
    *,
    idempotency_key: str,
    reason: str,
    pr_context: dict[str, Any],
    openspec_context: dict[str, Any],
    attempted_stages: list[str],
    required_follow_up: str,
    deferred_items: list[dict[str, Any]] | None = None,
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
    deferred = deferred_items or []
    if deferred:
        lines.extend(["", "## Deferred items"])
        for item in deferred:
            finding = item.get("finding_id") or item.get("id") or "unknown"
            title = item.get("title") or item.get("summary") or "Deferred review item"
            location = item.get("file") or item.get("path") or ""
            line = item.get("line")
            where = f" ({location}:{line})" if location and line else (f" ({location})" if location else "")
            root = item.get("root_cause_key") or item.get("root_cause") or ""
            root_text = f"; root cause `{root}`" if root else ""
            recommendation = item.get("recommendation") or item.get("reason") or item.get("summary") or "Handle this outside the current PR mutation loop."
            lines.append(f"- `{finding}`{where}: {title}{root_text}. Follow-up: {recommendation}")
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
    deferred_items: list[dict[str, Any]] | None = None,
) -> dict[str, Any]:
    key = make_issue_fallback_key(reason, pr_context, openspec_context, deferred_items)
    follow_up = required_follow_up or _default_follow_up(reason, openspec_context)
    body = render_issue_fallback_body(
        idempotency_key=key,
        reason=reason,
        pr_context=pr_context,
        openspec_context=openspec_context,
        attempted_stages=attempted_stages or [],
        required_follow_up=follow_up,
        deferred_items=deferred_items or [],
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
        "deferred_items": deferred_items or [],
        "deferred_count": len(deferred_items or []),
    }


def _default_follow_up(reason: str, openspec_context: dict[str, Any]) -> str:
    if reason in {"missing_openspec_spec", "unresolved_openspec_source"} or not openspec_context.get("present"):
        return "Add or link the OpenSpec change artifacts in the PR title/body, then rerun Codex Review."
    if reason in {"fork_pr_push_blocked", "out_of_scope"}:
        return "Move the implementation into a same-repository branch or handle the out-of-scope work in a separate PR."
    if reason == "stage02_defer_to_issue":
        return "Create or update a follow-up issue for work that is valid but outside the current PR mutation scope, while continuing the PR fix loop for implementable items."
    if reason in {"no-diff-repeat", "no_diff_repeat", "empty_patch"}:
        return "Inspect the generated fix artifacts and adjust the OpenSpec tasks or implementation plan so the next run can produce a non-empty patch."
    return "Resolve the blocking condition, then rerun Codex Review."


def apply_issue_fallback(plan: dict[str, Any], pr_context: dict[str, Any], token: str | None, *, dry_run: bool) -> dict[str, Any]:
    owner = pr_context.get("owner")
    repo = pr_context.get("repo")
    if dry_run:
        return {**plan, "status": "dry_run", "issue_url": None}
    if not token:
        raise ValidationError("stage09 actual issue fallback requires a GitHub App installation token")
    result = create_or_update_deferred_issue(str(owner), str(repo), plan["idempotency_key"], plan["title"], plan["body"], token)
    return {**plan, "status": "applied", "issue_url": result.get("html_url") or result.get("url")}
