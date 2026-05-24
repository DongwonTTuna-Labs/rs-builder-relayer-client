"""Unit tests for merge_and_gate.gate() — hard rule priority."""

from __future__ import annotations

import json
import os
import sys
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(REPO_ROOT / ".github" / "scripts"))

from merge_and_gate import combine, gate  # noqa: E402


class CombineArtifactShapeTest(unittest.TestCase):
    def test_malformed_findings_json_hard_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            art_dir = Path(tmp)
            (art_dir / "findings-correctness.json").write_text("{not json", encoding="utf-8")
            with self.assertRaises(SystemExit) as raised:
                combine(art_dir)
        self.assertEqual(raised.exception.code, 1)

    def test_non_object_findings_artifact_hard_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            art_dir = Path(tmp)
            (art_dir / "findings-correctness.json").write_text("[]", encoding="utf-8")
            with self.assertRaises(SystemExit) as raised:
                combine(art_dir)
        self.assertEqual(raised.exception.code, 1)

    def test_missing_findings_array_hard_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            art_dir = Path(tmp)
            (art_dir / "findings-correctness.json").write_text(
                json.dumps({"agent": "correctness"}),
                encoding="utf-8",
            )
            with self.assertRaises(SystemExit) as raised:
                combine(art_dir)
        self.assertEqual(raised.exception.code, 1)


class GateHardRuleTest(unittest.TestCase):
    def _run(self, combined: list[dict], decisions_doc: dict) -> list[dict]:
        with tempfile.TemporaryDirectory() as tmp:
            art_dir = Path(tmp)
            (art_dir / "combined.json").write_text(
                json.dumps(combined), encoding="utf-8"
            )
            (art_dir / "decisions.json").write_text(
                json.dumps(decisions_doc), encoding="utf-8"
            )
            gate(art_dir)
            with_decisions = json.loads(
                (art_dir / "with-decisions.json").read_text(encoding="utf-8")
            )
            allowed = json.loads((art_dir / "allowed.json").read_text(encoding="utf-8"))
        return with_decisions, allowed

    def test_must_finding_survives_merge_note(self) -> None:
        combined = [
            {"id": "f1", "type": "MUST", "agent": "correctness", "title": "x"},
        ]
        decisions_doc = {
            "decisions": [],
            "merge_notes": [{"primary_id": "fX", "merged_ids": ["f1"], "reason": "."}],
        }
        with_decisions, allowed = self._run(combined, decisions_doc)
        self.assertEqual(with_decisions[0]["gate_reason"], "hard_rule:must")
        self.assertEqual(len(allowed), 1)

    def test_security_finding_survives_merge_note(self) -> None:
        combined = [
            {"id": "s1", "type": "SUGGEST", "agent": "security", "title": "x"},
        ]
        decisions_doc = {
            "decisions": [],
            "merge_notes": [{"primary_id": "sX", "merged_ids": ["s1"], "reason": "."}],
        }
        _, allowed = self._run(combined, decisions_doc)
        self.assertEqual(len(allowed), 1)
        self.assertEqual(allowed[0]["gate_reason"], "hard_rule:security")

    def test_domain_critical_survives_merge_note(self) -> None:
        combined = [
            {
                "id": "d1",
                "type": "SUGGEST",
                "agent": "domain",
                "rule_ref": "framework-fidelity-critical",
                "title": "x",
            },
        ]
        decisions_doc = {
            "decisions": [],
            "merge_notes": [{"primary_id": "dX", "merged_ids": ["d1"], "reason": "."}],
        }
        _, allowed = self._run(combined, decisions_doc)
        self.assertEqual(len(allowed), 1)
        self.assertEqual(allowed[0]["gate_reason"], "hard_rule:domain-critical")

    def test_plain_suggest_still_merged_out(self) -> None:
        combined = [
            {"id": "p1", "type": "SUGGEST", "agent": "correctness", "title": "x"},
        ]
        decisions_doc = {
            "decisions": [],
            "merge_notes": [{"primary_id": "pX", "merged_ids": ["p1"], "reason": "."}],
        }
        with_decisions, allowed = self._run(combined, decisions_doc)
        self.assertEqual(with_decisions[0]["gate_reason"], "merge_note:consolidated")
        self.assertEqual(len(allowed), 0)

    def test_default_deny_when_no_decision(self) -> None:
        combined = [
            {"id": "u1", "type": "SUGGEST", "agent": "performance", "title": "x"},
        ]
        decisions_doc = {"decisions": [], "merge_notes": []}
        with_decisions, allowed = self._run(combined, decisions_doc)
        self.assertEqual(with_decisions[0]["gate_reason"], "default_deny:no_decision")
        self.assertEqual(len(allowed), 0)

    def test_tech_lead_decision_applied(self) -> None:
        combined = [
            {"id": "t1", "type": "SUGGEST", "agent": "performance", "title": "x"},
        ]
        decisions_doc = {
            "decisions": [{"id": "t1", "allow": True, "reason": "tl-approved"}],
            "merge_notes": [],
        }
        _, allowed = self._run(combined, decisions_doc)
        self.assertEqual(len(allowed), 1)
        self.assertEqual(allowed[0]["gate_reason"], "tl-approved")


if __name__ == "__main__":
    unittest.main()
