"""Cluster analysis helpers."""
from __future__ import annotations
from pathlib import Path
from typing import Any
from codex_review.artifacts import read_json, write_json
from codex_review.errors import ValidationError


def build_cluster_analysis_prompt(batch: dict[str, Any], design_context: dict[str, Any]) -> str:
    return "Analyze design cluster batch. Return stage03-cluster-analysis.v1 JSON.\n" + str({"batch": batch, "context": design_context})


def validate_cluster_analysis(analysis: dict[str, Any], batch: dict[str, Any]) -> dict[str, Any]:
    analyses=analysis.get("analyses") or analysis.get("clusters") or []
    expected={c.get("cluster_id") for c in batch.get("clusters", [])}
    got={a.get("cluster_id") for a in analyses}
    if expected and got != expected:
        raise ValidationError(f"cluster analysis coverage mismatch missing={sorted(expected-got)} unknown={sorted(got-expected)}")
    out=dict(analysis); out["schema_version"]="stage03-cluster-analysis.v1"; out["analyses"]=analyses
    return out


def write_cluster_analysis(analysis: dict[str, Any], out_path: str | Path) -> Path:
    return write_json(out_path, analysis, "stage03-cluster-analysis.v1")


def combine_cluster_analyses(analysis_artifacts: list[dict[str, Any] | str | Path]) -> list[dict[str, Any]]:
    out=[]
    for a in analysis_artifacts:
        payload=read_json(a) if isinstance(a,(str,Path)) else a
        out.extend(payload.get("analyses") or [])
    return out
