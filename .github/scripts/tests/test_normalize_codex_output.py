"""Unit tests for normalize_codex_output — schema contract preservation."""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(REPO_ROOT / ".github" / "scripts"))

from normalize_codex_output import normalize_decisions, normalize_findings  # noqa: E402


class NormalizeFindingsTest(unittest.TestCase):
    def test_missing_optional_fields_become_null(self) -> None:
        raw = {
            "agent": "correctness",
            "findings": [
                {
                    "id": "f1",
                    "type": "MUST",
                    "title": "t",
                    "reason": "r",
                    # file / line / rule_ref / cross_cutting 모두 누락
                }
            ],
        }
        result = normalize_findings(raw, "correctness")
        row = result["findings"][0]
        self.assertIsNone(row["file"])
        self.assertIsNone(row["line"])
        self.assertIsNone(row["rule_ref"])
        self.assertFalse(row["cross_cutting"])
        for required_key in ("id", "type", "file", "line", "title", "reason", "rule_ref", "cross_cutting"):
            self.assertIn(required_key, row)

    def test_valid_fields_are_kept(self) -> None:
        raw = {
            "agent": "security",
            "findings": [
                {
                    "id": "s1",
                    "type": "SUGGEST",
                    "title": "t",
                    "reason": "r",
                    "file": "src/a.py",
                    "line": 42,
                    "rule_ref": "auth-token-leak",
                    "cross_cutting": True,
                }
            ],
        }
        result = normalize_findings(raw, "security")
        row = result["findings"][0]
        self.assertEqual(row["file"], "src/a.py")
        self.assertEqual(row["line"], 42)
        self.assertEqual(row["rule_ref"], "auth-token-leak")
        self.assertTrue(row["cross_cutting"])

    def test_invalid_line_value_is_null(self) -> None:
        raw = {
            "agent": "correctness",
            "findings": [
                {
                    "id": "f1",
                    "type": "MUST",
                    "title": "t",
                    "reason": "r",
                    "line": "not-a-number",
                }
            ],
        }
        row = normalize_findings(raw, "correctness")["findings"][0]
        self.assertIsNone(row["line"])

    def test_zero_line_value_is_null(self) -> None:
        raw = {
            "agent": "correctness",
            "findings": [
                {
                    "id": "f1",
                    "type": "MUST",
                    "title": "t",
                    "reason": "r",
                    "line": 0,
                }
            ],
        }
        row = normalize_findings(raw, "correctness")["findings"][0]
        self.assertIsNone(row["line"])


class NormalizeDecisionsTest(unittest.TestCase):
    def test_invalid_judgment_emits_null_key(self) -> None:
        raw = {"decisions": [], "merge_notes": [], "judgment": {"status": "WAT"}}
        result = normalize_decisions(raw)
        self.assertIn("judgment", result)
        self.assertIsNone(result["judgment"])

    def test_missing_judgment_emits_null_key(self) -> None:
        raw = {"decisions": [], "merge_notes": []}
        result = normalize_decisions(raw)
        self.assertIn("judgment", result)
        self.assertIsNone(result["judgment"])

    def test_valid_judgment_kept(self) -> None:
        raw = {
            "decisions": [],
            "merge_notes": [],
            "judgment": {"status": "LGTM", "headline": "looks good"},
        }
        result = normalize_decisions(raw)
        self.assertEqual(
            result["judgment"], {"status": "LGTM", "headline": "looks good"}
        )

    def test_non_dict_input_emits_judgment_null(self) -> None:
        result = normalize_decisions("garbage")
        self.assertIn("judgment", result)
        self.assertIsNone(result["judgment"])


if __name__ == "__main__":
    unittest.main()
