"""Stage02 techlead reducer contract logic."""

from __future__ import annotations

from typing import Any

from .validators import require_keys, require_schema_version


REVIEW_SCHEMA = "codex.stage01.review.v1"
TECHLEAD_SCHEMA = "codex.stage02.techlead.v1"
REVIEW_STATUSES = {"lgtm", "needs_work"}
FINDING_SEVERITIES = {"must", "should", "nit"}
BLOCKING_SEVERITIES = {"must", "should"}
COMMENT_MARKER = "<!-- codex-review-v3-stage02 -->"


def _require_string(value: Any, field: str) -> str:
    text = str(value or "").strip()
    if not text:
        raise ValueError(f"{field} is required")
    return text


def _validate_axes(raw_axes: Any) -> list[str]:
    if not isinstance(raw_axes, list) or not raw_axes:
        raise ValueError("reviewed_axes must be a non-empty array")
    axes: list[str] = []
    seen: set[str] = set()
    for raw_axis in raw_axes:
        axis = _require_string(raw_axis, "reviewed_axes entry")
        if axis in seen:
            raise ValueError(f"duplicate reviewed_axis: {axis}")
        seen.add(axis)
        axes.append(axis)
    return axes


def _validate_findings(raw_findings: Any, reviewed_axes: list[str]) -> list[dict[str, Any]]:
    if not isinstance(raw_findings, list):
        raise ValueError("findings must be an array")
    findings: list[dict[str, Any]] = []
    seen: set[str] = set()
    for raw_finding in raw_findings:
        if not isinstance(raw_finding, dict):
            raise ValueError("findings entries must be objects")
        require_keys(raw_finding, ["finding_id", "axis", "severity", "title", "body", "root_cause_key"])
        finding_id = _require_string(raw_finding.get("finding_id"), "finding_id")
        if finding_id in seen:
            raise ValueError(f"duplicate finding_id: {finding_id}")
        seen.add(finding_id)
        axis = _require_string(raw_finding.get("axis"), f"{finding_id} axis")
        if axis not in reviewed_axes:
            raise ValueError(f"{finding_id} finding axis is not reviewed: {axis}")
        severity = _require_string(raw_finding.get("severity"), f"{finding_id} severity")
        if severity not in FINDING_SEVERITIES:
            raise ValueError(f"{finding_id} has unknown severity: {severity}")
        root_cause_key = _require_string(raw_finding.get("root_cause_key"), f"{finding_id} root_cause_key")
        if root_cause_key == finding_id:
            raise ValueError(f"{finding_id} root_cause_key must not equal finding_id")
        findings.append(
            {
                **raw_finding,
                "finding_id": finding_id,
                "axis": axis,
                "severity": severity,
                "title": _require_string(raw_finding.get("title"), f"{finding_id} title"),
                "body": _require_string(raw_finding.get("body"), f"{finding_id} body"),
                "root_cause_key": root_cause_key,
            }
        )
    return findings


def validate_review_artifact(payload: dict[str, Any]) -> dict[str, Any]:
    require_schema_version(payload, REVIEW_SCHEMA)
    require_keys(
        payload,
        ["status", "repository", "pr_number", "base_sha", "head_sha", "reviewed_axes", "finding_count", "findings"],
    )
    status = _require_string(payload.get("status"), "status")
    if status not in REVIEW_STATUSES:
        raise ValueError(f"unknown review status: {status}")
    reviewed_axes = _validate_axes(payload.get("reviewed_axes"))
    findings = _validate_findings(payload.get("findings"), reviewed_axes)
    if payload.get("finding_count") != len(findings):
        raise ValueError("finding_count does not match findings")
    if status == "lgtm" and findings:
        raise ValueError("lgtm review must not include findings")
    if status == "needs_work" and not findings:
        raise ValueError("needs_work review requires at least one finding")
    return {
        "status": status,
        "repository": _require_string(payload.get("repository"), "repository"),
        "pr_number": _require_string(payload.get("pr_number"), "pr_number"),
        "base_sha": _require_string(payload.get("base_sha"), "base_sha"),
        "head_sha": _require_string(payload.get("head_sha"), "head_sha"),
        "reviewed_axes": reviewed_axes,
        "summary": str(payload.get("summary") or "").strip(),
        "findings": findings,
    }


def build_techlead_result(review_payload: dict[str, Any]) -> dict[str, Any]:
    review = validate_review_artifact(review_payload)
    blocking_ids = [
        str(finding["finding_id"])
        for finding in review["findings"]
        if str(finding.get("severity") or "") in BLOCKING_SEVERITIES
    ]
    non_blocking_ids = [
        str(finding["finding_id"])
        for finding in review["findings"]
        if str(finding.get("severity") or "") == "nit"
    ]
    requires_design = bool(blocking_ids)
    return {
        "schema_version": TECHLEAD_SCHEMA,
        "stage": "stage02-techlead",
        "status": "needs_design" if requires_design else "approved",
        "can_continue": True,
        "requires_design": requires_design,
        "requires_fix": requires_design,
        "repository": review["repository"],
        "pr_number": review["pr_number"],
        "base_sha": review["base_sha"],
        "head_sha": review["head_sha"],
        "reviewed_axes": review["reviewed_axes"],
        "finding_count": len(review["findings"]),
        "blocking_finding_ids": blocking_ids,
        "non_blocking_finding_ids": non_blocking_ids,
        "summary": review["summary"],
    }


def _format_location(finding: dict[str, Any]) -> str:
    file = str(finding.get("file") or "").strip()
    line = finding.get("line")
    if file and isinstance(line, int) and line > 0:
        return f"{file}:{line}"
    return file or "general"


def build_review_comment_body(
    review_payload: dict[str, Any], techlead_payload: dict[str, Any], *, run_url: str = ""
) -> str:
    review = validate_review_artifact(review_payload)
    require_schema_version(techlead_payload, TECHLEAD_SCHEMA)
    require_keys(techlead_payload, ["status", "blocking_finding_ids", "non_blocking_finding_ids", "summary"])
    status = _require_string(techlead_payload.get("status"), "techlead status")
    blocking_ids = [str(item) for item in techlead_payload.get("blocking_finding_ids") or []]
    non_blocking_ids = [str(item) for item in techlead_payload.get("non_blocking_finding_ids") or []]
    summary = str(techlead_payload.get("summary") or review["summary"] or "").strip()
    lines = [
        COMMENT_MARKER,
        "## Codex Review V3",
        "",
        f"- status: `{status}`",
        f"- findings: `{len(review['findings'])}`",
        f"- blocking: `{', '.join(blocking_ids) if blocking_ids else 'none'}`",
        f"- non-blocking: `{', '.join(non_blocking_ids) if non_blocking_ids else 'none'}`",
    ]
    if run_url.strip():
        lines.append(f"- run: {run_url.strip()}")
    if summary:
        lines.extend(["", "### Summary", summary])
    if review["findings"]:
        lines.extend(["", "### Findings"])
        for finding in review["findings"]:
            lines.append(
                f"- `{finding['finding_id']}` `{finding['severity']}` `{finding['axis']}` "
                f"{_format_location(finding)} - {finding['title']}"
            )
            lines.append(f"  {finding['body']}")
    else:
        lines.extend(["", "No findings."])
    return "\n".join(lines).rstrip() + "\n"
