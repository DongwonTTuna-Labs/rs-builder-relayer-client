"""Combine stage01 axis findings."""
from __future__ import annotations
from pathlib import Path
from typing import Any
from codex_review.artifacts import read_json, write_json
from codex_review.errors import ValidationError


def detect_cross_axis_duplicates(findings: list[dict[str, Any]]) -> list[dict[str, Any]]:
    groups={}
    for f in findings:
        groups.setdefault(f.get("root_cause_key") or f.get("finding_id"), []).append(f.get("finding_id") or f.get("id"))
    return [{"root_cause_key": k, "finding_ids": v} for k,v in groups.items() if len(v)>1]


def summarize_findings_by_axis(findings: list[dict[str, Any]]) -> dict[str, int]:
    summary={}
    for f in findings: summary[f.get("axis", "unknown")]=summary.get(f.get("axis", "unknown"),0)+1
    return summary


def combine_axis_findings(axis_artifacts: list[dict[str, Any] | str | Path]) -> dict[str, Any]:
    all_findings=[]; ids=set()
    for art in axis_artifacts:
        payload=read_json(art) if isinstance(art, (str, Path)) else art
        axis=payload.get("axis")
        for f in payload.get("findings", []) or []:
            fid=f.get("finding_id") or f.get("id")
            if fid in ids: raise ValidationError(f"duplicate finding id across axes: {fid}")
            ids.add(fid)
            nf=dict(f); nf.setdefault("axis", axis); nf.setdefault("finding_id", fid)
            all_findings.append(nf)
    return {"schema_version": "stage01-combined-findings.v1", "findings": all_findings, "finding_count": len(all_findings), "duplicates": detect_cross_axis_duplicates(all_findings), "summary_by_axis": summarize_findings_by_axis(all_findings)}


def write_combined_findings(combined: dict[str, Any], out_path: str | Path) -> Path:
    return write_json(out_path, combined, "stage01-combined-findings.v1")
