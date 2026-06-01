"""Design cluster validation."""
from __future__ import annotations
from pathlib import Path
from typing import Any
from codex_review.artifacts import write_json
from codex_review.errors import ValidationError


def build_cluster_prompt(design_inventory: dict[str, Any], design_context: dict[str, Any]) -> str:
    return "Cluster design inventory by invariant/root cause. Return stage03-design-clusters.v1 JSON.\n" + str({"inventory":design_inventory,"context":design_context})


def validate_design_clusters(clusters: dict[str, Any], inventory: dict[str, Any]) -> dict[str, Any]:
    items=clusters.get("clusters") or []
    expected={i.get("finding_id") for i in inventory.get("items", [])}
    covered=set()
    for c in items:
        if not c.get("cluster_id"): raise ValidationError("cluster missing cluster_id")
        ids=c.get("finding_ids") or c.get("items") or []
        covered.update(ids)
    if expected and covered != expected:
        raise ValidationError(f"cluster coverage mismatch missing={sorted(expected-covered)} unknown={sorted(covered-expected)}")
    out=dict(clusters); out["schema_version"]="stage03-design-clusters.v1"; out["clusters"]=items; out["cluster_count"]=len(items)
    return out


def write_design_clusters(clusters: dict[str, Any], out_path: str | Path) -> Path:
    return write_json(out_path, clusters, "stage03-design-clusters.v1")


def summarize_clusters(clusters: dict[str, Any]) -> str:
    return "\n".join(f"- {c.get('cluster_id')}: {c.get('summary','')}" for c in clusters.get("clusters", []))
