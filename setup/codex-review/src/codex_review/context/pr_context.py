"""PR context artifact builder."""
from __future__ import annotations

from pathlib import Path
from typing import Any

from codex_review.artifacts import write_json
from codex_review.context.changed_lines import build_changed_line_map, serialize_changed_line_map
from codex_review.context.diff import summarize_diff


def build_pr_context(event: dict[str, Any], pr: dict[str, Any], files: list[dict[str, Any]], diff: str, config: dict[str, Any]) -> dict[str, Any]:
    head=pr.get("head", {}) or {}; base=pr.get("base", {}) or {}
    context={
        "schema_version": "pr-context.v1",
        "repository": event.get("repository", {}).get("full_name") or pr.get("base", {}).get("repo", {}).get("full_name"),
        "owner": (event.get("repository", {}).get("owner", {}) or {}).get("login"),
        "pr_number": pr.get("number") or event.get("number") or event.get("pull_request", {}).get("number"),
        "title": pr.get("title") or event.get("pull_request", {}).get("title"),
        "body": pr.get("body") or "",
        "state": pr.get("state"),
        "base_ref": base.get("ref"),
        "base_sha": base.get("sha"),
        "head_ref": head.get("ref"),
        "head_sha": head.get("sha"),
        "head_repo_full_name": (head.get("repo") or {}).get("full_name"),
        "base_repo_full_name": (base.get("repo") or {}).get("full_name"),
        "same_repo": (head.get("repo") or {}).get("full_name") == (base.get("repo") or {}).get("full_name") if head.get("repo") and base.get("repo") else None,
        "changed_files": files,
        "changed_line_map": serialize_changed_line_map(build_changed_line_map(files)),
        "diff_summary": summarize_diff(diff or "", 16000),
        "config_base_branch": config.get("base_branch"),
    }
    return include_changed_files_summary(include_repository_metadata(context))


def include_repository_metadata(context: dict[str, Any]) -> dict[str, Any]:
    repo=context.get("repository") or context.get("base_repo_full_name")
    if repo and "/" in repo:
        context.setdefault("owner", repo.split("/",1)[0])
        context.setdefault("repo", repo.split("/",1)[1])
    return context


def include_changed_files_summary(context: dict[str, Any]) -> dict[str, Any]:
    summary=[]
    for f in context.get("changed_files", []) or []:
        summary.append({"filename": f.get("filename") or f.get("path"), "status": f.get("status"), "additions": f.get("additions", 0), "deletions": f.get("deletions", 0)})
    context["changed_files_summary"] = summary
    return context


def write_pr_context(context: dict[str, Any], out_path: str | Path) -> Path:
    return write_json(out_path, context, "pr-context.v1")
