"""Validate stage08 loop reentry records."""
from __future__ import annotations

from pathlib import Path
from typing import Any

from codex_review.core.artifacts import write_json
from codex_review.core.errors import ValidationError


VALID_NEXT_ENTRIES = {"stage00_on_synchronize", "none", "stage00"}


def validate_reentry_record(record: dict[str, Any], previous_loop_state: dict[str, Any] | None = None) -> dict[str, Any]:
    if record.get("schema_version") and record.get("schema_version") != "stage08-loop-reentry.v1":
        raise ValidationError("invalid stage08 reentry schema_version")
    out = dict(record)
    out["schema_version"] = "stage08-loop-reentry.v1"
    if "pushed" not in out:
        raise ValidationError("stage08 reentry record missing pushed")
    pushed = bool(out.get("pushed"))
    next_entry = out.get("next_entry") or ("stage00_on_synchronize" if pushed else "none")
    if next_entry not in VALID_NEXT_ENTRIES:
        raise ValidationError(f"invalid stage08 next_entry: {next_entry}")
    if pushed and not out.get("commit_sha"):
        raise ValidationError("pushed reentry record requires commit_sha")
    if pushed and not out.get("persisted") and not out.get("persistence_optional"):
        raise ValidationError("pushed reentry record must be persisted for the next workflow run")
    if not pushed and next_entry not in {"none"}:
        raise ValidationError("non-pushed reentry must not expect a synchronize run")
    loop_state = out.get("loop_state") or previous_loop_state or {}
    if loop_state and not isinstance(loop_state, dict):
        raise ValidationError("loop_state must be an object")
    out["next_entry"] = next_entry
    out["loop_state"] = loop_state
    return out


def write_validated_reentry_record(record: dict[str, Any], out_path: str | Path) -> Path:
    return write_json(out_path, record, "stage08-loop-reentry.v1")
