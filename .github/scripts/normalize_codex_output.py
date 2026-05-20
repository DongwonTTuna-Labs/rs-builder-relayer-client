#!/usr/bin/env python3
"""Validate and normalize a Codex output JSON for the v2 pipeline.

The pipeline distinguishes two output shapes:
  - ``findings`` — emitted by the 5 axes in Stage 1. Validated against
    ``schemas/findings.schema.json``. On schema violation, an empty findings
    object for the given axis is written so downstream stages can continue.
  - ``decisions`` — emitted by the tech-lead in Stage 2. Validated against
    ``schemas/decisions.schema.json``. On violation, an empty decisions
    object is written.

Required env:
  RUNNER_TEMP — directory containing the raw Codex output.

Args:
  --schema {findings,decisions} — required.
  --axis <name>                 — required for findings; ignored for decisions.

Reads ``$RUNNER_TEMP/codex-output-<axis|tech-lead>.json`` and writes the
normalized version to ``$RUNNER_TEMP/findings-<axis>.json`` (findings) or
``$RUNNER_TEMP/decisions.json`` (decisions).
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path

VALID_TYPES = {"MUST", "SUGGEST", "IMO", "NITS", "ASK"}
VALID_AGENTS = {"correctness", "security", "performance", "test-coverage", "domain"}
VALID_STATUSES = {"LGTM", "NEEDS_CLARIFICATION", "NEEDS_WORK"}


def warn(msg: str) -> None:
    sys.stderr.write(f"::warning::{msg}\n")


def normalize_findings(raw: dict, axis: str) -> dict:
    """Coerce findings JSON into the canonical shape; drop invalid rows."""
    if not isinstance(raw, dict) or raw.get("agent") != axis:
        warn(f"findings JSON missing/agent mismatch for axis={axis}; emitting empty")
        return {"agent": axis, "findings": [], "positive": [], "impact_summary": None}

    findings_raw = raw.get("findings")
    if not isinstance(findings_raw, list):
        warn(f"findings is not an array for axis={axis}; emitting empty")
        findings_raw = []

    normalized_findings: list[dict] = []
    for entry in findings_raw[:13]:
        if not isinstance(entry, dict):
            continue
        finding_type = str(entry.get("type") or "").upper()
        if finding_type not in VALID_TYPES:
            continue
        title = str(entry.get("title") or "").strip()
        reason = str(entry.get("reason") or "").strip()
        finding_id = str(entry.get("id") or "").strip()
        if not title or not reason or not finding_id:
            continue
        row: dict = {
            "id": finding_id,
            "type": finding_type,
            "title": title[:200],
            "reason": reason[:1000],
            "cross_cutting": bool(entry.get("cross_cutting") or False),
        }
        if entry.get("file"):
            row["file"] = str(entry["file"]).strip()
        if entry.get("line"):
            try:
                line_no = int(entry["line"])
                if line_no >= 1:
                    row["line"] = line_no
            except (TypeError, ValueError):
                pass
        if entry.get("rule_ref"):
            row["rule_ref"] = str(entry["rule_ref"])[:200]
        normalized_findings.append(row)

    positive_raw = raw.get("positive") or []
    if not isinstance(positive_raw, list):
        positive_raw = []
    positive = [str(p).strip()[:200] for p in positive_raw if str(p).strip()][:2]

    impact_summary = raw.get("impact_summary")
    if axis != "domain":
        impact_summary = None
    elif not isinstance(impact_summary, dict):
        impact_summary = None

    return {
        "agent": axis,
        "findings": normalized_findings,
        "positive": positive,
        "impact_summary": impact_summary,
    }


def normalize_decisions(raw: dict) -> dict:
    if not isinstance(raw, dict):
        warn("decisions JSON is not an object; emitting empty")
        return {"decisions": [], "merge_notes": []}

    decisions_raw = raw.get("decisions")
    if not isinstance(decisions_raw, list):
        warn("decisions is not an array; emitting empty")
        decisions_raw = []

    decisions: list[dict] = []
    for entry in decisions_raw:
        if not isinstance(entry, dict):
            continue
        decision_id = str(entry.get("id") or "").strip()
        reason = str(entry.get("reason") or "").strip()
        allow = bool(entry.get("allow") or False)
        if not decision_id or not reason:
            continue
        decisions.append({"id": decision_id, "allow": allow, "reason": reason[:300]})

    judgment = raw.get("judgment")
    if isinstance(judgment, dict):
        status = str(judgment.get("status") or "").upper()
        headline = str(judgment.get("headline") or "").strip()
        if status in VALID_STATUSES and headline:
            judgment = {"status": status, "headline": headline[:200]}
        else:
            judgment = None
    else:
        judgment = None

    merge_notes_raw = raw.get("merge_notes") or []
    if not isinstance(merge_notes_raw, list):
        merge_notes_raw = []
    merge_notes: list[dict] = []
    for note in merge_notes_raw:
        if not isinstance(note, dict):
            continue
        primary = str(note.get("primary_id") or "").strip()
        merged = note.get("merged_ids") or []
        merge_reason = str(note.get("reason") or "").strip()
        if not primary or not isinstance(merged, list) or not merge_reason:
            continue
        merged_ids = [str(m).strip() for m in merged if str(m).strip()]
        if not merged_ids:
            continue
        merge_notes.append(
            {
                "primary_id": primary,
                "merged_ids": merged_ids,
                "reason": merge_reason[:300],
            }
        )

    out: dict = {"decisions": decisions, "merge_notes": merge_notes}
    if judgment:
        out["judgment"] = judgment
    return out


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--schema", required=True, choices=["findings", "decisions"])
    parser.add_argument("--axis")
    args = parser.parse_args()

    runner_temp = Path(os.environ["RUNNER_TEMP"])

    if args.schema == "findings":
        if not args.axis:
            raise SystemExit("--axis is required when --schema=findings")
        if args.axis not in VALID_AGENTS:
            raise SystemExit(f"unknown axis: {args.axis}")
        raw_path = runner_temp / f"codex-output-{args.axis}.json"
        out_path = runner_temp / f"findings-{args.axis}.json"
        try:
            raw = json.loads(raw_path.read_text(encoding="utf-8"))
        except (FileNotFoundError, json.JSONDecodeError) as exc:
            warn(f"failed to parse {raw_path.name}: {exc}; emitting empty findings")
            raw = {}
        normalized = normalize_findings(raw, args.axis)
    else:
        raw_path = runner_temp / "codex-output-tech-lead.json"
        out_path = runner_temp / "decisions.json"
        try:
            raw = json.loads(raw_path.read_text(encoding="utf-8"))
        except (FileNotFoundError, json.JSONDecodeError) as exc:
            warn(f"failed to parse {raw_path.name}: {exc}; emitting empty decisions")
            raw = {}
        normalized = normalize_decisions(raw)

    out_path.write_text(
        json.dumps(normalized, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
