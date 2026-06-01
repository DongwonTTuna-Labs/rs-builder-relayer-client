"""JSON schema and lightweight validation helpers."""
from __future__ import annotations

import json
from typing import Any, Iterable

from .errors import ValidationError
from .paths import schema_path


def load_schema_text(name: str) -> str:
    return schema_path(name).read_text(encoding="utf-8")


def load_schema_json(name: str) -> dict[str, Any]:
    return json.loads(load_schema_text(name))


def require_schema_version(payload: dict[str, Any], expected: str) -> None:
    actual = payload.get("schema_version")
    if actual != expected:
        raise ValidationError(f"expected schema_version {expected!r}, got {actual!r}")


def validate_required_keys(payload: dict[str, Any], keys: Iterable[str], context: str = "payload") -> None:
    missing = [key for key in keys if key not in payload]
    if missing:
        raise ValidationError(f"{context} missing required keys: {', '.join(missing)}")


def validate_enum(value: Any, allowed: Iterable[Any], context: str = "value") -> None:
    allowed_set = set(allowed)
    if value not in allowed_set:
        raise ValidationError(f"{context} must be one of {sorted(map(str, allowed_set))}, got {value!r}")


def validate_json_schema(payload: dict[str, Any], schema_name: str) -> None:
    try:
        import jsonschema  # type: ignore
    except Exception:
        return
    jsonschema.validate(payload, load_schema_json(schema_name))
