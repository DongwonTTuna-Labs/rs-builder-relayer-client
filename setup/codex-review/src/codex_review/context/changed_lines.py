"""Changed RIGHT-side line map helpers."""
from __future__ import annotations

from typing import Any

from .diff import extract_changed_right_lines, parse_unified_diff


def _normalize_lines(lines: Any) -> set[int]:
    if isinstance(lines, set):
        return {int(x) for x in lines}
    if isinstance(lines, list):
        return {int(x) for x in lines}
    return set()


def build_changed_line_map(pr_files: list[dict[str, Any]] | dict[str, Any] | str) -> dict[str, set[int]]:
    if isinstance(pr_files, str):
        return extract_changed_right_lines(pr_files)
    if isinstance(pr_files, dict):
        if "changed_lines" in pr_files:
            return {str(k): _normalize_lines(v) for k, v in pr_files["changed_lines"].items()}
        pr_files = pr_files.get("files") or pr_files.get("changed_files") or []
    changed: dict[str, set[int]] = {}
    for f in pr_files:
        filename = f.get("filename") or f.get("path") or f.get("new_path")
        patch = f.get("patch") or f.get("diff") or ""
        if not filename:
            continue
        if patch and not patch.startswith("diff --git"):
            patch = f"diff --git a/{filename} b/{filename}\n--- a/{filename}\n+++ b/{filename}\n{patch}"
        lines = extract_changed_right_lines(parse_unified_diff(patch)) if patch else set()
        if not lines and f.get("changed_lines"):
            lines = _normalize_lines(f["changed_lines"])
        changed[str(filename)] = lines
    return changed


def is_changed_right_line(changed_map: dict[str, Any], file: str, line: int | str) -> bool:
    try:
        n = int(line)
    except Exception:
        return False
    return n in _normalize_lines(changed_map.get(file, set()))


def nearest_changed_line(changed_map: dict[str, Any], file: str, line: int | str) -> int | None:
    lines = sorted(_normalize_lines(changed_map.get(file, set())))
    if not lines:
        return None
    try:
        n = int(line)
    except Exception:
        return lines[0]
    return min(lines, key=lambda x: (abs(x - n), x))


def serialize_changed_line_map(changed_map: dict[str, Any]) -> dict[str, list[int]]:
    return {path: sorted(_normalize_lines(lines)) for path, lines in changed_map.items()}
