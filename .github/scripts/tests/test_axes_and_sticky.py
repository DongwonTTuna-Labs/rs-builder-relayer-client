"""Unit tests for axes_status emission (F1) and inline-skip sticky markers (F3)."""

from __future__ import annotations

import json
import sys
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(REPO_ROOT / ".github" / "scripts"))

from merge_and_gate import EXPECTED_AXES, combine  # noqa: E402
from post_review_comments import render_axis_status, render_block  # noqa: E402


class CombineAxisStatusTest(unittest.TestCase):
    def test_missing_axes_fail_before_posting_sticky(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            art_dir = Path(tmp)
            # Only correctness + security present; performance/test-coverage/domain missing.
            (art_dir / "findings-correctness.json").write_text(
                json.dumps({"agent": "correctness", "findings": []}), encoding="utf-8"
            )
            (art_dir / "findings-security.json").write_text(
                json.dumps({"agent": "security", "findings": []}), encoding="utf-8"
            )
            with self.assertRaises(SystemExit) as raised:
                combine(art_dir)
            self.assertEqual(raised.exception.code, 1)
            data = json.loads(
                (art_dir / "axes_status.json").read_text(encoding="utf-8")
            )
        self.assertEqual(set(data["expected"]), EXPECTED_AXES)
        self.assertEqual(set(data["received"]), {"correctness", "security"})
        self.assertEqual(
            set(data["missing"]), {"performance", "test-coverage", "domain"}
        )

    def test_all_axes_present_no_missing(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            art_dir = Path(tmp)
            for axis in EXPECTED_AXES:
                (art_dir / f"findings-{axis}.json").write_text(
                    json.dumps({"agent": axis, "findings": []}), encoding="utf-8"
                )
            combine(art_dir)
            data = json.loads(
                (art_dir / "axes_status.json").read_text(encoding="utf-8")
            )
        self.assertEqual(data["missing"], [])


class AxisStatusRenderTest(unittest.TestCase):
    def test_empty_when_no_status_file(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            self.assertEqual(render_axis_status(Path(tmp)), "")

    def test_empty_when_no_missing(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            art_dir = Path(tmp)
            (art_dir / "axes_status.json").write_text(
                json.dumps(
                    {"expected": [], "received": [], "missing": []}
                ),
                encoding="utf-8",
            )
            self.assertEqual(render_axis_status(art_dir), "")

    def test_warning_emitted_when_missing(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            art_dir = Path(tmp)
            (art_dir / "axes_status.json").write_text(
                json.dumps(
                    {
                        "expected": ["security", "performance"],
                        "received": ["performance"],
                        "missing": ["security"],
                    }
                ),
                encoding="utf-8",
            )
            rendered = render_axis_status(art_dir)
        self.assertIn("⚠️", rendered)
        self.assertIn("`security`", rendered)
        self.assertIn("부분 리뷰", rendered)


class WorkflowPostGateTest(unittest.TestCase):
    def test_post_job_waits_for_successful_tech_lead(self) -> None:
        workflow = (REPO_ROOT / ".github" / "workflows" / "codex-pr-review-pipeline.yml").read_text(
            encoding="utf-8"
        )
        post_block = workflow.split("\n  post:\n", 1)[1].split(
            "\n  # ============================================================",
            1,
        )[0]
        self.assertIn("name: post", post_block)
        self.assertIn("needs: [resolve-check, tech-lead]", post_block)
        self.assertRegex(
            post_block,
            r"(?m)^    if: always\(\) && !cancelled\(\) && needs\.tech-lead\.result == 'success'$",
        )


class StickyInlineSkipMarkerTest(unittest.TestCase):
    def test_no_inline_marker_inserted_for_out_of_range(self) -> None:
        finding = {
            "type": "MUST",
            "agent": "security",
            "file": "src/a.py",
            "line": 99,
            "title": "t",
            "reason": "r",
            "_no_inline": "outside_changed_lines",
        }
        block = render_block([finding], {"MUST"}, checkbox=True)
        self.assertIn("no inline", block)
        self.assertIn("변경 라인 밖", block)

    def test_no_inline_marker_for_missing_location(self) -> None:
        finding = {
            "type": "MUST",
            "agent": "security",
            "title": "t",
            "reason": "r",
            "_no_inline": "no_location",
        }
        block = render_block([finding], {"MUST"}, checkbox=True)
        self.assertIn("위치 정보 없음", block)

    def test_no_marker_for_normal_finding(self) -> None:
        finding = {
            "type": "MUST",
            "agent": "security",
            "file": "src/a.py",
            "line": 10,
            "title": "t",
            "reason": "r",
        }
        block = render_block([finding], {"MUST"}, checkbox=True)
        self.assertNotIn("no inline", block)


if __name__ == "__main__":
    unittest.main()
