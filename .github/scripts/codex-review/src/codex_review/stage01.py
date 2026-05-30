"""Stage01 review artifact contract logic."""

from __future__ import annotations

from typing import Any

from .validators import require_keys, require_schema_version


REVIEW_REQUEST_SCHEMA = "codex.stage01.review_request.v1"
MODEL_REVIEW_SCHEMA = "codex.stage01.model_review.v1"
REVIEW_SCHEMA = "codex.stage01.review.v1"
REVIEW_STATUSES = {"lgtm", "needs_work"}
FINDING_SEVERITIES = {"must", "should", "nit"}


def _require_string(value: Any, field: str) -> str:
    text = str(value or "").strip()
    if not text:
        raise ValueError(f"{field} is required")
    return text


def _validate_axes(raw_axes: Any) -> list[str]:
    if not isinstance(raw_axes, list) or not raw_axes:
        raise ValueError("axes must be a non-empty array")
    axes: list[str] = []
    seen: set[str] = set()
    for raw_axis in raw_axes:
        axis = _require_string(raw_axis, "axis")
        if axis in seen:
            raise ValueError(f"duplicate axis: {axis}")
        seen.add(axis)
        axes.append(axis)
    return axes


def _validate_changed_files(raw_files: Any) -> list[dict[str, Any]]:
    if not isinstance(raw_files, list):
        raise ValueError("changed_files must be an array")
    files: list[dict[str, Any]] = []
    for raw_file in raw_files:
        if not isinstance(raw_file, dict):
            raise ValueError("changed_files entries must be objects")
        require_keys(raw_file, ["path", "status"])
        files.append(
            {
                **raw_file,
                "path": _require_string(raw_file.get("path"), "changed_files path"),
                "status": _require_string(raw_file.get("status"), "changed_files status"),
            }
        )
    return files


def validate_review_request(payload: dict[str, Any]) -> dict[str, Any]:
    require_schema_version(payload, REVIEW_REQUEST_SCHEMA)
    require_keys(payload, ["repository", "pr_number", "base_sha", "head_sha", "changed_files", "axes"])
    return {
        "repository": _require_string(payload.get("repository"), "repository"),
        "pr_number": _require_string(payload.get("pr_number"), "pr_number"),
        "base_sha": _require_string(payload.get("base_sha"), "base_sha"),
        "head_sha": _require_string(payload.get("head_sha"), "head_sha"),
        "changed_files": _validate_changed_files(payload.get("changed_files")),
        "axes": _validate_axes(payload.get("axes")),
    }


def _validate_axis_results(raw_results: Any, expected_axes: list[str]) -> list[dict[str, Any]]:
    if not isinstance(raw_results, list):
        raise ValueError("axis_results must be an array")
    results: list[dict[str, Any]] = []
    seen: set[str] = set()
    for raw_result in raw_results:
        if not isinstance(raw_result, dict):
            raise ValueError("axis_results entries must be objects")
        require_keys(raw_result, ["axis", "status", "summary"])
        axis = _require_string(raw_result.get("axis"), "axis_result axis")
        status = _require_string(raw_result.get("status"), f"{axis} status")
        if status not in REVIEW_STATUSES:
            raise ValueError(f"{axis} has unknown status: {status}")
        if axis in seen:
            raise ValueError(f"duplicate axis_result: {axis}")
        seen.add(axis)
        results.append(
            {
                "axis": axis,
                "status": status,
                "summary": _require_string(raw_result.get("summary"), f"{axis} summary"),
            }
        )
    if set(seen) != set(expected_axes):
        raise ValueError("axis_results must cover exactly request axes")
    return results


def _validate_findings(raw_findings: Any, expected_axes: list[str]) -> list[dict[str, Any]]:
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
        if axis not in expected_axes:
            raise ValueError(f"{finding_id} finding axis is not requested: {axis}")
        severity = _require_string(raw_finding.get("severity"), f"{finding_id} severity")
        if severity not in FINDING_SEVERITIES:
            raise ValueError(f"{finding_id} has unknown severity: {severity}")
        root_cause_key = _require_string(raw_finding.get("root_cause_key"), f"{finding_id} root_cause_key")
        if root_cause_key == finding_id:
            raise ValueError(f"{finding_id} root_cause_key must not equal finding_id")
        finding: dict[str, Any] = {
            "finding_id": finding_id,
            "axis": axis,
            "severity": severity,
            "title": _require_string(raw_finding.get("title"), f"{finding_id} title"),
            "body": _require_string(raw_finding.get("body"), f"{finding_id} body"),
            "root_cause_key": root_cause_key,
        }
        if raw_finding.get("file") is not None:
            finding["file"] = _require_string(raw_finding.get("file"), f"{finding_id} file")
        if raw_finding.get("line") is not None:
            line = raw_finding.get("line")
            if not isinstance(line, int) or line <= 0:
                raise ValueError(f"{finding_id} line must be a positive integer")
            finding["line"] = line
        findings.append(finding)
    return findings


def validate_model_review(payload: dict[str, Any], expected_axes: list[str]) -> dict[str, Any]:
    require_schema_version(payload, MODEL_REVIEW_SCHEMA)
    require_keys(payload, ["status", "summary", "axis_results", "findings"])
    status = _require_string(payload.get("status"), "status")
    if status not in REVIEW_STATUSES:
        raise ValueError(f"unknown review status: {status}")
    axis_results = _validate_axis_results(payload.get("axis_results"), expected_axes)
    needs_work_axes = [result["axis"] for result in axis_results if result["status"] == "needs_work"]
    if status == "lgtm" and needs_work_axes:
        raise ValueError(
            f"top-level status lgtm conflicts with needs_work axis_results: {', '.join(needs_work_axes)}"
        )
    findings = _validate_findings(payload.get("findings"), expected_axes)
    if status == "lgtm" and findings:
        raise ValueError("lgtm review must not include findings")
    if status == "needs_work" and not findings:
        raise ValueError("needs_work review requires at least one finding")
    finding_axes = {finding["axis"] for finding in findings}
    mismatched_finding_axes = sorted(finding_axes - set(needs_work_axes))
    if mismatched_finding_axes:
        raise ValueError(f"finding axes must have needs_work axis_results: {', '.join(mismatched_finding_axes)}")
    return {
        "status": status,
        "summary": _require_string(payload.get("summary"), "summary"),
        "axis_results": axis_results,
        "findings": findings,
    }


def build_review_result(request_payload: dict[str, Any], model_payload: dict[str, Any]) -> dict[str, Any]:
    request = validate_review_request(request_payload)
    model = validate_model_review(model_payload, request["axes"])
    return {
        "schema_version": REVIEW_SCHEMA,
        "stage": "stage01-review",
        "status": model["status"],
        "can_continue": True,
        "repository": request["repository"],
        "pr_number": request["pr_number"],
        "base_sha": request["base_sha"],
        "head_sha": request["head_sha"],
        "reviewed_axes": request["axes"],
        "changed_files": request["changed_files"],
        "summary": model["summary"],
        "axis_results": model["axis_results"],
        "finding_count": len(model["findings"]),
        "findings": model["findings"],
    }
