"""Validate complete stage03 artifact chain."""
from __future__ import annotations
from typing import Any
from codex_review.errors import ValidationError


def validate_plan_scope(plan: dict[str, Any], techlead_decision: dict[str, Any]) -> None:
    allowed=set()
    for item in techlead_decision.get("decisions", []):
        if item.get("file"): allowed.add(item["file"])
        allowed.update(item.get("files", []) or [])
    if not allowed: return
    for step in plan.get("edit_sequence", []):
        for f in step.get("files", []) or step.get("allowed_files", []) or []:
            if f not in allowed and not any(str(f).startswith(str(a).rstrip('/') + '/') for a in allowed):
                raise ValidationError(f"plan edits file outside techlead scope: {f}")


def validate_plan_tests(plan: dict[str, Any]) -> None:
    if not plan.get("tests"):
        raise ValidationError("design plan missing tests")


def validate_no_unsafe_design(plan: dict[str, Any], config: dict[str, Any]) -> None:
    text=str(plan).lower(); dangerous=[k.lower() for k in config.get("autofix", {}).get("dangerous_keywords", [])]
    if any(k and k in text for k in dangerous) and not plan.get("requires_human_override"):
        raise ValidationError("design plan includes dangerous keyword and requires human review")


def validate_stage03_artifact_chain(context: dict[str, Any], inventory: dict[str, Any], clusters: dict[str, Any], analyses: list[dict[str, Any]], plan: dict[str, Any]) -> dict[str, Any]:
    if context.get("schema_version") != "stage03-design-context.v1": raise ValidationError("invalid design context schema")
    if inventory.get("schema_version") != "stage03-design-inventory.v1": raise ValidationError("invalid design inventory schema")
    if clusters.get("schema_version") != "stage03-design-clusters.v1": raise ValidationError("invalid design clusters schema")
    if plan.get("schema_version") != "stage03-design-plan.v1": raise ValidationError("invalid design plan schema")
    cluster_ids={c.get("cluster_id") for c in clusters.get("clusters", [])}
    analysis_ids={a.get("cluster_id") for a in analyses}
    if cluster_ids and not analysis_ids.issuperset(cluster_ids): raise ValidationError("not all clusters analyzed")
    return {"ok": True, "cluster_count": len(cluster_ids), "analysis_count": len(analysis_ids), "plan_hash": plan.get("plan_hash")}
