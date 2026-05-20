"""Unit tests for outdated comment fallback in check_inline_resolutions."""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(REPO_ROOT / ".github" / "scripts"))

from check_inline_resolutions import normalize_batch_output  # noqa: E402


class NormalizeBatchOutputTest(unittest.TestCase):
    def test_missing_file_returns_resolved_false(self) -> None:
        rows = normalize_batch_output(Path("/nonexistent.json"), {1, 2})
        ids = sorted(r["comment_id"] for r in rows)
        self.assertEqual(ids, [1, 2])
        self.assertTrue(all(r["resolved"] is False for r in rows))

    def test_invalid_json_returns_resolved_false(self, tmp_path: Path | None = None) -> None:
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            bad = Path(tmp) / "bad.json"
            bad.write_text("not json", encoding="utf-8")
            rows = normalize_batch_output(bad, {7})
        self.assertEqual(len(rows), 1)
        self.assertFalse(rows[0]["resolved"])
        self.assertIn("invalid", rows[0]["reason"].lower())

    def test_partial_output_fills_missing_ids(self) -> None:
        import json
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "out.json"
            path.write_text(
                json.dumps(
                    {
                        "resolutions": [
                            {
                                "comment_id": 11,
                                "resolved": True,
                                "reason": "fixed",
                            }
                        ]
                    }
                ),
                encoding="utf-8",
            )
            rows = normalize_batch_output(path, {11, 22})
        rows_by_id = {r["comment_id"]: r for r in rows}
        self.assertTrue(rows_by_id[11]["resolved"])
        self.assertFalse(rows_by_id[22]["resolved"])
        self.assertIn("did not emit", rows_by_id[22]["reason"])


if __name__ == "__main__":
    unittest.main()
