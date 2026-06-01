"""Normalize techlead findings for design."""
from __future__ import annotations
from pathlib import Path
from typing import Any
from codex_review.artifacts import write_json
from codex_review.errors import ValidationError


def build_normalize_prompt(design_context: dict[str, Any]) -> str:
    return "Normalize the following design-relevant findings into invariant-oriented items. Return stage03-design-inventory.v1 JSON.\n" + str(design_context)


def validate_design_inventory(inventory: dict[str, Any], techlead_decision: dict[str, Any]) -> dict[str, Any]:
    items=inventory.get("items") or inventory.get("findings") or []
    expected={i.get("finding_id") for i in techlead_decision.get("decisions", []) if i.get("action") in {"needs_design","publish_and_fix_now","summary_only_fix_now"}}
    got={i.get("finding_id") for i in items}
    if expected and got != expected:
        raise ValidationError(f"design inventory coverage mismatch missing={sorted(expected-got)} unknown={sorted(got-expected)}")
    out=dict(inventory); out["schema_version"]="stage03-design-inventory.v1"; out["items"]=items; out["item_count"]=len(items)
    return out


def write_design_inventory(inventory: dict[str, Any], out_path: str | Path) -> Path:
    return write_json(out_path, inventory, "stage03-design-inventory.v1")


def summarize_design_inventory(inventory: dict[str, Any]) -> str:
    return f"Design inventory items: {len(inventory.get('items', []))}"
