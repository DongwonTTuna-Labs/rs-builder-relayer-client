"""Coordinate final design plan."""
from __future__ import annotations
import hashlib, json
from pathlib import Path
from typing import Any
from codex_review.artifacts import write_json
from codex_review.errors import ValidationError


def build_coordinate_prompt(design_context: dict[str, Any], clusters: dict[str, Any], analyses: list[dict[str, Any]]) -> str:
    instructions = (
        "Coordinate a candidate design plan. Return stage03-design-plan.v1 JSON with edit_sequence and tests.\n"
        "Do not include human-routing fields in this artifact. stage04 design chief decides whether the "
        "candidate is approved_for_fix, needs_human, rejected_plan, or no_fix_needed.\n"
    )
    return instructions + str({"context":design_context,"clusters":clusters,"analyses":analyses})


def validate_design_plan(plan: dict[str, Any], design_context: dict[str, Any], config: dict[str, Any]) -> dict[str, Any]:
    if "open_questions" in plan:
        raise ValidationError("stage03 design plan does not accept open_questions; use stage04 needs_human routing")
    out=dict(plan); out["schema_version"]="stage03-design-plan.v1"
    out.setdefault("edit_sequence", out.get("tasks") or [])
    out.setdefault("tests", [])
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
