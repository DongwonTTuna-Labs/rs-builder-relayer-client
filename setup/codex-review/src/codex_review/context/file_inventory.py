"""Repository file inventory helpers."""
from __future__ import annotations

import fnmatch
from pathlib import Path
from typing import Any

from codex_review.errors import ValidationError


def _matches(path: str, patterns: list[str]) -> bool:
    return any(path.startswith(p.rstrip("*").rstrip("/")) or fnmatch.fnmatch(path, p) for p in patterns)


def list_repository_files(root: str | Path, include_patterns: list[str] | None = None, exclude_patterns: list[str] | None = None) -> list[str]:
    base=Path(root)
    include_patterns=include_patterns or ["*"]
    exclude_patterns=exclude_patterns or [".git/*", "codex-review-artifacts/*"]
    files=[]
    for p in base.rglob("*"):
        if not p.is_file():
            continue
        rel=p.relative_to(base).as_posix()
        if _matches(rel, exclude_patterns):
            continue
        if include_patterns and not _matches(rel, include_patterns):
            continue
        files.append(rel)
    return sorted(files)


def select_relevant_files(findings: list[dict[str, Any]], design_plan: dict[str, Any] | None, changed_files: list[str]) -> list[str]:
    selected=set(changed_files or [])
    for f in findings or []:
        if f.get("file"):
            selected.add(str(f["file"]))
        for file in f.get("files", []) or []:
            selected.add(str(file))
    for step in (design_plan or {}).get("edit_sequence", []) or []:
        for file in step.get("files", []) or step.get("allowed_files", []) or []:
            selected.add(str(file))
    return sorted(x for x in selected if x)


def read_source_files(files: list[str | Path], budget: int = 40000) -> list[dict[str, Any]]:
    out=[]; remaining=budget
    for f in files:
        p=Path(f)
        if not p.exists() or not p.is_file() or remaining <= 0:
            continue
        text=p.read_text(encoding="utf-8", errors="replace")
        if len(text) > remaining:
            text=text[:remaining] + "\n...[truncated]"
        remaining -= len(text)
        out.append({"path": p.as_posix(), "text": text})
    return out


def build_allowed_file_set(design_chief_decision: dict[str, Any], policy: dict[str, Any]) -> list[str]:
    allowed = set(design_chief_decision.get("allowed_files") or design_chief_decision.get("fix_policy", {}).get("allowed_files") or [])
    if not allowed:
        allowed.update(design_chief_decision.get("fix_policy", {}).get("allowed_prefixes") or policy.get("allowed_prefixes") or [])
    forbidden_files=set(policy.get("forbidden_files", [])) | set(design_chief_decision.get("fix_policy", {}).get("forbidden_files", []))
    result=[f for f in allowed if f not in forbidden_files]
    if not result:
        raise ValidationError("allowed file set is empty")
    return sorted(result)
