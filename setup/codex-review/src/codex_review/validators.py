"""Small fail-closed validators for v3 artifact contracts."""

from __future__ import annotations

from collections.abc import Iterable, Mapping
from pathlib import PurePosixPath
import shlex
from typing import Any


class ContractViolation(ValueError):
    """Raised when a stage artifact does not satisfy its contract."""


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


def parse_unified_diff_paths(patch: str) -> list[str]:
    paths: set[str] = set()
    saw_file_header = False
    for line in patch.splitlines():
        candidates: list[str] = []
        if line.startswith("diff --git "):
            candidates.extend(_diff_git_paths(line))
            saw_file_header = True
        elif line.startswith(("--- ", "+++ ")):
            candidates.append(line[4:])
        elif line.startswith("rename from "):
            candidates.append(line.removeprefix("rename from "))
        elif line.startswith("rename to "):
            candidates.append(line.removeprefix("rename to "))
        elif line.startswith("copy from "):
            candidates.append(line.removeprefix("copy from "))
        elif line.startswith("copy to "):
            candidates.append(line.removeprefix("copy to "))
        elif line.startswith("Binary files ") and line.endswith(" differ"):
            left, _, right = line.removeprefix("Binary files ").removesuffix(" differ").partition(" and ")
            if not right:
                raise ContractViolation("malformed binary patch header")
            candidates.extend([left, right])
        elif line == "GIT binary patch" and not saw_file_header:
            raise ContractViolation("binary patch missing diff --git header")

        for candidate in candidates:
            normalized = _normalize_patch_path(candidate)
            if normalized:
                paths.add(normalized)
    return sorted(paths)
