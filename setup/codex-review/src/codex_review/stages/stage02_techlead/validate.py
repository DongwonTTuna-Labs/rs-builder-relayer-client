"""Stage02 techlead decision validation."""
from __future__ import annotations
from pathlib import Path
from typing import Any
from codex_review.artifacts import write_json
from codex_review.constants import TECHLEAD_ACTIONS, SEVERITIES
from codex_review.errors import ValidationError
from codex_review.schema import validate_enum


def validate_decision_action(item: dict[str, Any]) -> None:
    validate_enum(item.get("action"), TECHLEAD_ACTIONS, "techlead action")


def validate_scope_and_severity(item: dict[str, Any]) -> None:
    if item.get("severity"): validate_enum(item.get("severity"), SEVERITIES, "severity")


def validate_design_required(decision: dict[str, Any]) -> None:
    if decision.get("needs_design") and not any((i.get("action") in {"needs_design","publish_and_fix_now","summary_only_fix_now"}) for i in decision.get("decisions", [])):
        raise ValidationError("needs_design=true requires at least one design/fix action")


def validate_autofix_risk(item: dict[str, Any], config: dict[str, Any]) -> None:
    if item.get("action") in {"publish_and_fix_now", "summary_only_fix_now"}:
        text=" ".join(str(item.get(k,"")) for k in ["summary","risk","reason"] ).lower()
        dangerous=[k.lower() for k in config.get("autofix", {}).get("dangerous_keywords", [])]
        if any(k and k in text for k in dangerous) and not item.get("requires_human_override"):
            raise ValidationError("unsafe autofix decision requires human override")


def validate_techlead_decision(decision: dict[str, Any], combined_findings: dict[str, Any], config: dict[str, Any]) -> dict[str, Any]:
    items=decision.get("decisions") or decision.get("items") or []
    if not isinstance(items, list): raise ValidationError("techlead decisions must be a list")
    expected={f.get("finding_id") or f.get("id") for f in combined_findings.get("findings", [])}
    got=[i.get("finding_id") for i in items]
    if len(got)!=len(set(got)): raise ValidationError("duplicate finding_id in techlead decision")
    if set(got)!=expected:
        raise ValidationError(f"techlead decision must cover all findings exactly once missing={sorted(expected-set(got))} unknown={sorted(set(got)-expected)}")
    for item in items:
        validate_decision_action(item); validate_scope_and_severity(item); validate_autofix_risk(item, config)
    out=dict(decision); out["schema_version"]="stage02-techlead-decision.v1"; out["decisions"]=items
    out.setdefault("needs_design", any(i.get("action") in {"needs_design","publish_and_fix_now","summary_only_fix_now"} for i in items))
    out.setdefault("status", "needs_human" if any(i.get("action")=="needs_human" for i in items) else "ready")
    validate_design_required(out)
    return out


def write_validated_techlead_decision(decision: dict[str, Any], out_path: str | Path) -> Path:
    return write_json(out_path, decision, "stage02-techlead-decision.v1")
