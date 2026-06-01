"""Coordinate final design plan."""
from __future__ import annotations
import hashlib, json
from pathlib import Path
from typing import Any
from codex_review.artifacts import write_json
from codex_review.errors import ValidationError


def build_coordinate_prompt(design_context: dict[str, Any], clusters: dict[str, Any], analyses: list[dict[str, Any]]) -> str:
    return "Coordinate a final design plan. Return stage03-design-plan.v1 JSON with edit_sequence and tests.\n" + str({"context":design_context,"clusters":clusters,"analyses":analyses})


def validate_design_plan(plan: dict[str, Any], design_context: dict[str, Any], config: dict[str, Any]) -> dict[str, Any]:
    out=dict(plan); out["schema_version"]="stage03-design-plan.v1"
    out.setdefault("open_questions", [])
    out.setdefault("edit_sequence", out.get("tasks") or [])
    out.setdefault("tests", [])
    if config.get("design", {}).get("fail_on_open_questions", True) and out.get("open_questions"):
        raise ValidationError("design plan has open questions")
    if not out.get("edit_sequence") and design_context.get("findings"):
        raise ValidationError("design plan needs edit_sequence for design findings")
    if not out.get("tests") and design_context.get("findings"):
        raise ValidationError("design plan needs tests")
    out["plan_hash"]=compute_design_plan_hash(out)
    return out


def write_design_plan(plan: dict[str, Any], out_path: str | Path) -> Path:
    return write_json(out_path, plan, "stage03-design-plan.v1")


def compute_design_plan_hash(plan: dict[str, Any]) -> str:
    clean={k:v for k,v in plan.items() if k!="plan_hash"}
    return hashlib.sha256(json.dumps(clean, sort_keys=True, default=str).encode()).hexdigest()[:24]
