"""Small fail-closed validators for v3 artifact contracts."""

from __future__ import annotations

from collections.abc import Iterable, Mapping
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
