import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "src"))

from codex_review.stage01 import build_review_result


def request(axes=None):
    return {
        "schema_version": "codex.stage01.review_request.v1",
        "repository": "DongwonTTuna-Labs/rs-builder-relayer-client",
        "pr_number": "36",
        "base_sha": "a" * 40,
        "head_sha": "b" * 40,
        "changed_files": [
            {
                "path": "src/lib.rs",
                "status": "modified",
            }
        ],
        "axes": axes or ["correctness", "tests"],
    }


def model_review(status="needs_work", findings=None, axis_results=None):
    return {
        "schema_version": "codex.stage01.model_review.v1",
        "status": status,
        "summary": "review summary",
        "axis_results": axis_results
        or [
            {"axis": "correctness", "status": status, "summary": "correctness summary"},
            {"axis": "tests", "status": "lgtm", "summary": "tests summary"},
        ],
        "findings": findings
        if findings is not None
        else [
            {
                "finding_id": "REV-001",
                "axis": "correctness",
                "severity": "must",
                "file": "src/lib.rs",
                "line": 10,
                "title": "Fix incorrect value",
                "body": "The value is computed from the wrong input.",
                "root_cause_key": "incorrect-value-source",
            }
        ],
    }


class Stage01Tests(unittest.TestCase):
    def test_needs_work_review_result_normalizes_model_output(self):
        result = build_review_result(request(), model_review())

        self.assertEqual("codex.stage01.review.v1", result["schema_version"])
        self.assertEqual("stage01-review", result["stage"])
        self.assertEqual("needs_work", result["status"])
        self.assertTrue(result["can_continue"])
        self.assertEqual("DongwonTTuna-Labs/rs-builder-relayer-client", result["repository"])
        self.assertEqual("36", result["pr_number"])
        self.assertEqual(["correctness", "tests"], result["reviewed_axes"])
        self.assertEqual(1, result["finding_count"])
        self.assertEqual("REV-001", result["findings"][0]["finding_id"])
        self.assertEqual("incorrect-value-source", result["findings"][0]["root_cause_key"])

    def test_lgtm_review_requires_no_findings(self):
        result = build_review_result(
            request(),
            model_review(
                status="lgtm",
                findings=[],
                axis_results=[
                    {"axis": "correctness", "status": "lgtm", "summary": "correctness summary"},
                    {"axis": "tests", "status": "lgtm", "summary": "tests summary"},
                ],
            ),
        )

        self.assertEqual("lgtm", result["status"])
        self.assertEqual(0, result["finding_count"])
        self.assertEqual([], result["findings"])

    def test_axis_results_must_match_request_axes(self):
        with self.assertRaises(ValueError) as ctx:
            build_review_result(
                request(),
                model_review(axis_results=[{"axis": "correctness", "status": "needs_work", "summary": "summary"}]),
            )

        self.assertIn("axis_results must cover exactly request axes", str(ctx.exception))

    def test_finding_axis_must_be_requested(self):
        bad = model_review()
        bad["findings"][0]["axis"] = "security"

        with self.assertRaises(ValueError) as ctx:
            build_review_result(request(), bad)

        self.assertIn("finding axis is not requested", str(ctx.exception))

    def test_root_cause_key_must_not_equal_finding_id(self):
        bad = model_review()
        bad["findings"][0]["root_cause_key"] = "REV-001"

        with self.assertRaises(ValueError) as ctx:
            build_review_result(request(), bad)

        self.assertIn("root_cause_key must not equal finding_id", str(ctx.exception))

    def test_lgtm_with_findings_fails_closed(self):
        with self.assertRaises(ValueError) as ctx:
            build_review_result(request(), model_review(status="lgtm"))

        self.assertIn("lgtm review must not include findings", str(ctx.exception))

    def test_needs_work_without_findings_fails_closed(self):
        with self.assertRaises(ValueError) as ctx:
            build_review_result(
                request(),
                model_review(
                    status="needs_work",
                    findings=[],
                    axis_results=[
                        {"axis": "correctness", "status": "needs_work", "summary": "correctness summary"},
                        {"axis": "tests", "status": "lgtm", "summary": "tests summary"},
                    ],
                ),
            )

        self.assertIn("needs_work review requires at least one finding", str(ctx.exception))

    def test_cli_reads_request_and_model_output_then_writes_review_artifact(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            request_path = tmp_path / "request.json"
            model_output_path = tmp_path / "model.json"
            out_path = tmp_path / "review.json"
            request_path.write_text(json.dumps(request()), encoding="utf-8")
            model_output_path.write_text(json.dumps(model_review()), encoding="utf-8")

            result = subprocess.run(
                [
                    sys.executable,
                    "-m",
                    "codex_review.cli",
                    "stage01-review",
                    "--request",
                    str(request_path),
                    "--model-output",
                    str(model_output_path),
                    "--out",
                    str(out_path),
                ],
                cwd=ROOT,
                env={"PYTHONPATH": str(ROOT / "src")},
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )

            self.assertEqual("", result.stderr)
            self.assertEqual(0, result.returncode)
            payload = json.loads(out_path.read_text(encoding="utf-8"))
            self.assertEqual("codex.stage01.review.v1", payload["schema_version"])
            self.assertEqual("needs_work", payload["status"])


if __name__ == "__main__":
    unittest.main()
