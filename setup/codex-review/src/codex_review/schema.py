"""Small fail-closed validators for v3 artifact contracts."""

from __future__ import annotations

from collections.abc import Iterable, Mapping
from dataclasses import dataclass, field
from pathlib import PurePosixPath
import re
import shlex
from typing import Any


class ContractViolation(ValueError):
    """Raised when a stage artifact does not satisfy its contract."""


_HUNK_HEADER_RE = re.compile(
    r"^@@ -(?P<old_start>\d+)(?:,(?P<old_count>\d+))? "
    r"\+(?P<new_start>\d+)(?:,(?P<new_count>\d+))? @@(?: .*)?$"
)


def require_schema_version(payload: Mapping[str, Any], expected: str) -> None:
    actual = payload.get("schema_version")
    if actual != expected:
        raise ContractViolation(f"schema_version expected {expected}, got {actual!r}")


def require_keys(payload: Mapping[str, Any], keys: Iterable[str]) -> None:
    missing = [key for key in keys if key not in payload]
    if missing:
        raise ContractViolation("missing required keys: " + ", ".join(missing))


def _normalize_patch_path(raw_path: str) -> str | None:
    path = raw_path.strip().split("\t", 1)[0]
    if not path or path == "/dev/null":
        return None
    if path.startswith(("a/", "b/")):
        path = path[2:]
    parsed = PurePosixPath(path)
    if path.startswith("/") or any(part == ".." for part in parsed.parts):
        raise ContractViolation(f"unsafe patch path: {raw_path}")
    return path


def _diff_git_paths(line: str) -> tuple[str, str]:
    try:
        parts = shlex.split(line)
    except ValueError as exc:
        raise ContractViolation("malformed diff --git header") from exc
    if len(parts) != 4 or parts[0] != "diff" or parts[1] != "--git":
        raise ContractViolation("malformed diff --git header")
    return parts[2], parts[3]


@dataclass
class _PatchFile:
    git_source: str
    git_destination: str
    old_header: str | None = None
    new_header: str | None = None
    rename_from: str | None = None
    rename_to: str | None = None
    copy_from: str | None = None
    copy_to: str | None = None
    binary_paths: list[str] = field(default_factory=list)


def _normalized_paths(paths: Iterable[str | None]) -> set[str]:
    normalized_paths: set[str] = set()
    for path in paths:
        if path is None:
            continue
        normalized = _normalize_patch_path(path)
        if normalized:
            normalized_paths.add(normalized)
    return normalized_paths


def _parse_unified_diff_files(patch: str) -> list[_PatchFile]:
    files: list[_PatchFile] = []
    current: _PatchFile | None = None
    for line in patch.splitlines():
        if line.startswith("diff --git "):
            source, destination = _diff_git_paths(line)
            current = _PatchFile(source, destination)
            files.append(current)
        elif line.startswith("--- "):
            if current is None:
                raise ContractViolation("file header missing diff --git header")
            current.old_header = line[4:]
        elif line.startswith("+++ "):
            if current is None:
                raise ContractViolation("file header missing diff --git header")
            current.new_header = line[4:]
        elif line.startswith("rename from "):
            if current is None:
                raise ContractViolation("rename header missing diff --git header")
            current.rename_from = line.removeprefix("rename from ")
        elif line.startswith("rename to "):
            if current is None:
                raise ContractViolation("rename header missing diff --git header")
            current.rename_to = line.removeprefix("rename to ")
        elif line.startswith("copy from "):
            if current is None:
                raise ContractViolation("copy header missing diff --git header")
            current.copy_from = line.removeprefix("copy from ")
        elif line.startswith("copy to "):
            if current is None:
                raise ContractViolation("copy header missing diff --git header")
            current.copy_to = line.removeprefix("copy to ")
        elif line.startswith("Binary files ") and line.endswith(" differ"):
            left, _, right = line.removeprefix("Binary files ").removesuffix(" differ").partition(" and ")
            if not right:
                raise ContractViolation("malformed binary patch header")
            if current is None:
                raise ContractViolation("binary patch missing diff --git header")
            current.binary_paths.extend([left, right])
        elif line == "GIT binary patch" and current is None:
            raise ContractViolation("binary patch missing diff --git header")
    return files


def _hunk_count(raw_count: str | None) -> int:
    return 1 if raw_count is None else int(raw_count)


def validate_unified_diff_hunks(patch: str) -> None:
    """Reject unified diffs whose hunk headers do not match their line bodies."""
    expected_old: int | None = None
    expected_new: int | None = None
    actual_old = 0
    actual_new = 0
    hunk_start = 0

    def finish_hunk() -> None:
        if expected_old is None or expected_new is None:
            return
        if actual_old != expected_old or actual_new != expected_new:
            raise ContractViolation(
                f"malformed unified diff hunk at line {hunk_start}: "
                f"expected -{expected_old}/+{expected_new} lines, "
                f"saw -{actual_old}/+{actual_new}"
            )

    for lineno, line in enumerate(patch.splitlines(), 1):
        if line.startswith("@@ "):
            finish_hunk()
            match = _HUNK_HEADER_RE.match(line)
            if not match:
                raise ContractViolation(f"malformed unified diff hunk at line {lineno}")
            expected_old = _hunk_count(match.group("old_count"))
            expected_new = _hunk_count(match.group("new_count"))
            actual_old = 0
            actual_new = 0
            hunk_start = lineno
            continue
        if line.startswith("diff --git "):
            finish_hunk()
            expected_old = None
            expected_new = None
            actual_old = 0
            actual_new = 0
            hunk_start = 0
            continue
        if expected_old is None or expected_new is None:
            continue
        if line.startswith("\\"):
            continue
        if line.startswith(" "):
            actual_old += 1
            actual_new += 1
        elif line.startswith("-"):
            actual_old += 1
        elif line.startswith("+"):
            actual_new += 1
        else:
            raise ContractViolation(f"malformed unified diff hunk at line {lineno}")
    finish_hunk()


def parse_unified_diff_scope_paths(patch: str) -> list[str]:
    """Return every source and destination path a patch can affect."""
    paths: set[str] = set()
    for file in _parse_unified_diff_files(patch):
        paths.update(
            _normalized_paths(
                [
                    file.git_source,
                    file.git_destination,
                    file.old_header,
                    file.new_header,
                    file.rename_from,
                    file.rename_to,
                    file.copy_from,
                    file.copy_to,
                    *file.binary_paths,
                ]
            )
        )
    return sorted(paths)


def parse_unified_diff_paths(patch: str) -> list[str]:
    """Return Stage07 comparison paths after applying a unified diff.

    Rename and copy patches compare by destination, deletes by deleted path,
    and add/modify/binary patches by the changed post-apply path.
    """
    paths: set[str] = set()
    for file in _parse_unified_diff_files(patch):
        old_paths = _normalized_paths([file.old_header])
        new_paths = _normalized_paths([file.new_header])
        if file.rename_to:
            paths.update(_normalized_paths([file.rename_to]))
        elif file.copy_to:
            paths.update(_normalized_paths([file.copy_to]))
        elif old_paths and not new_paths:
            paths.update(old_paths)
        elif new_paths:
            paths.update(new_paths)
        else:
            paths.update(_normalized_paths([file.git_destination]))
    return sorted(paths)
