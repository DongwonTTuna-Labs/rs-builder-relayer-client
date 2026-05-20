#!/usr/bin/env python3
"""Merge per-axis findings, apply tech-lead decisions and hard rules.

Two phases share this script:

  --phase combine
    Read every ``findings-<axis>.json`` from ``$ART_DIR`` and write
    ``combined.json`` (flat array, with ``agent`` injected into each row).

  --phase gate
    Read ``combined.json`` and ``decisions.json``, merge tech-lead decisions,
    apply hard rules, and write ``with-decisions.json`` and ``allowed.json``.

Hard rules (post-script overrides tech-lead AND merge_notes):
  - ``type == "MUST"``                                 → allow=true
  - ``agent == "security"``                            → allow=true
  - ``rule_ref`` endswith ``"-critical"`` (e.g.
    ``framework-fidelity-critical``)                   → allow=true
  - ``merge_notes`` 의 merged ids                       → allow=false
  - otherwise → tech-lead's ``allow`` value, default false.

Hard rule 이 ``merge_notes`` 보다 우선한다. tech-lead 가 보안/MUST/critical
finding 을 동급 finding 으로 묶어 merge 해도 hard rule 이 살아남도록 한다.

Required env:
  ART_DIR — directory holding artifacts (default: ``./artifacts``).
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path


EXPECTED_AXES = {
    "correctness",
    "security",
    "performance",
    "test-coverage",
    "domain",
}


def warn(msg: str) -> None:
    sys.stderr.write(f"::warning::{msg}\n")


def combine(art_dir: Path) -> None:
    findings_files = sorted(art_dir.glob("findings-*.json"))
    combined: list[dict] = []
    received_axes: set[str] = set()
    if not findings_files:
        warn("no findings-*.json found; combined.json will be empty")
    for path in findings_files:
        try:
            data = json.loads(path.read_text(encoding="utf-8"))
        except json.JSONDecodeError as exc:
            warn(f"skipping {path.name}: {exc}")
            continue
        agent = data.get("agent")
        if agent:
            received_axes.add(str(agent))
        for finding in data.get("findings") or []:
            row = dict(finding)
            row["agent"] = agent
            combined.append(row)

    missing_axes = sorted(EXPECTED_AXES - received_axes)
    if missing_axes:
        warn(
            "axis artifact missing for: "
            + ", ".join(missing_axes)
            + " — sticky summary will surface a partial-review warning"
        )

    (art_dir / "combined.json").write_text(
        json.dumps(combined, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )
    (art_dir / "axes_status.json").write_text(
        json.dumps(
            {
                "expected": sorted(EXPECTED_AXES),
                "received": sorted(received_axes),
                "missing": missing_axes,
            },
            ensure_ascii=False,
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )
    print(f"combined findings: {len(combined)}")


def is_domain_critical(finding: dict) -> bool:
    rule_ref = str(finding.get("rule_ref") or "")
    return rule_ref.endswith("-critical")


def gate(art_dir: Path) -> None:
    combined_path = art_dir / "combined.json"
    decisions_path = art_dir / "decisions.json"
    combined = json.loads(combined_path.read_text(encoding="utf-8"))
    if not decisions_path.exists():
        warn("decisions.json missing; falling back to hard rules only")
        decisions_doc: dict = {"decisions": [], "merge_notes": []}
    else:
        decisions_doc = json.loads(decisions_path.read_text(encoding="utf-8"))

    decisions_by_id: dict[str, dict] = {
        str(d.get("id")): d for d in decisions_doc.get("decisions") or []
    }
    merged_ids: set[str] = set()
    for note in decisions_doc.get("merge_notes") or []:
        for mid in note.get("merged_ids") or []:
            merged_ids.add(str(mid))

    with_decisions: list[dict] = []
    for finding in combined:
        finding_id = str(finding.get("id") or "")
        decision = decisions_by_id.get(finding_id)
        # Hard rules first — never let merge_notes hide MUST / security /
        # domain-critical findings (tech-lead 가 동일 카테고리 finding 들을
        # merge 해도 hard-rule allow 가 이기도록).
        if finding.get("type") == "MUST":
            allow = True
            gate_reason = "hard_rule:must"
        elif finding.get("agent") == "security":
            allow = True
            gate_reason = "hard_rule:security"
        elif finding.get("agent") == "domain" and is_domain_critical(finding):
            allow = True
            gate_reason = "hard_rule:domain-critical"
        elif finding_id in merged_ids:
            allow = False
            gate_reason = "merge_note:consolidated"
        elif decision is not None:
            allow = bool(decision.get("allow"))
            gate_reason = decision.get("reason") or ""
        else:
            allow = False
            gate_reason = "default_deny:no_decision"
        with_decisions.append(
            {**finding, "allow": allow, "gate_reason": gate_reason}
        )

    allowed = [f for f in with_decisions if f.get("allow")]

    (art_dir / "with-decisions.json").write_text(
        json.dumps(with_decisions, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )
    (art_dir / "allowed.json").write_text(
        json.dumps(allowed, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )
    print(f"allowed: {len(allowed)} / {len(with_decisions)}")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--phase", required=True, choices=["combine", "gate"])
    args = parser.parse_args()

    art_dir = Path(os.environ.get("ART_DIR", "./artifacts"))
    art_dir.mkdir(parents=True, exist_ok=True)

    if args.phase == "combine":
        combine(art_dir)
    else:
        gate(art_dir)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
