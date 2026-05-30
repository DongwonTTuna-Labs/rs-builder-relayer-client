"""Deterministic artifact IO for Codex review v3 stages."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any


JsonObject = dict[str, Any]


def _require_write_object(payload: Any) -> JsonObject:
    if not isinstance(payload, dict):
        raise ValueError("artifact payload must be a JSON object")
    return payload


def write_json_artifact(path: str | Path, payload: JsonObject) -> None:
    """Write a stage artifact using stable formatting."""

    obj = _require_write_object(payload)
    target = Path(path)
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(json.dumps(obj, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def read_json_artifact(path: str | Path) -> JsonObject:
    """Read a stage artifact and require an object root."""

    source = Path(path)
    payload = json.loads(source.read_text(encoding="utf-8"))
    if not isinstance(payload, dict):
        raise ValueError(f"{source} must contain a JSON object")
    return payload
