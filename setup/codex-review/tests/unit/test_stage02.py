import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "src"))

from codex_review.stages.stage02_techlead import build_review_comment_body, build_techlead_result


def finding(finding_id="REV-001", severity="must"):
    return {
        "finding_id": finding_id,
        "axis": "correctness",
        "severity": severity,
        "file": "src/lib.rs",
        "line": 10,
        "title": "Fix incorrect value",
        "body": "The value is computed from the wrong input.",
        "root_cause_key": f"root-{finding_id.lower()}",
    }


def review(status="needs_work", findings=None):
    if findings is None:
        findings = [finding()]
    return {
        "schema_version": "codex.stage01.review.v1",
        "stage": "stage01-review",
        "status": status,
        "can_continue": True,
        "repository": "DongwonTTuna-Labs/rs-builder-relayer-client",
        "pr_number": "36",
        "base_sha": "a" * 40,
        "head_sha": "b" * 40,
        "reviewed_axes": ["correctness", "tests"],
        "changed_files": [{"path": "src/lib.rs", "status": "modified"}],
        "summary": "review summary",
        "axis_results": [
            {"axis": "correctness", "status": status, "summary": "correctness summary"},
            {"axis": "tests", "status": "lgtm", "summary": "tests summary"},
        ],
        "finding_count": len(findings),
        "findings": findings,
    }


class Stage02Tests(unittest.TestCase):
    def test_lgtm_review_is_approved_without_design(self):
        result = build_techlead_result(review(status="lgtm", findings=[]))

        self.assertEqual("codex.stage02.techlead.v1", result["schema_version"])
        self.assertEqual("stage02-techlead", result["stage"])
        self.assertEqual("approved", result["status"])
        self.assertFalse(result["requires_design"])
        self.assertFalse(result["requires_fix"])
        self.assertEqual([], result["blocking_finding_ids"])

    def test_must_finding_requires_design_and_fix(self):
        result = build_techlead_result(review(findings=[finding("REV-001", "must")]))

        self.assertEqual("needs_design", result["status"])
        self.assertTrue(result["requires_design"])
        self.assertTrue(result["requires_fix"])
        self.assertEqual(["REV-001"], result["blocking_finding_ids"])
        self.assertEqual([], result["non_blocking_finding_ids"])

    def test_should_finding_is_blocking(self):
        result = build_techlead_result(review(findings=[finding("REV-002", "should")]))

        self.assertEqual("needs_design", result["status"])
        self.assertEqual(["REV-002"], result["blocking_finding_ids"])

    def test_nit_only_review_is_approved_with_non_blocking_findings(self):
        result = build_techlead_result(review(findings=[finding("REV-003", "nit")]))

        self.assertEqual("approved", result["status"])
        self.assertFalse(result["requires_design"])
        self.assertFalse(result["requires_fix"])
        self.assertEqual([], result["blocking_finding_ids"])
        self.assertEqual(["REV-003"], result["non_blocking_finding_ids"])

    def test_review_comment_body_lists_status_findings_and_run_url(self):
        review_payload = review(findings=[finding("REV-001", "must")])
        techlead_payload = build_techlead_result(review_payload)

        body = build_review_comment_body(review_payload, techlead_payload, run_url="https://example.invalid/run/1")

        self.assertIn("<!-- codex-review-v3-stage02 -->", body)
        self.assertIn("status: `needs_design`", body)
        self.assertIn("blocking: `REV-001`", body)
        self.assertIn("https://example.invalid/run/1", body)
        self.assertIn("REV-001", body)
        self.assertIn("Fix incorrect value", body)
        self.assertIn("src/lib.rs:10", body)

    def test_finding_count_mismatch_fails_closed(self):
        payload = review()
        payload["finding_count"] = 2

        with self.assertRaises(ValueError) as ctx:
            build_techlead_result(payload)

        self.assertIn("finding_count does not match findings", str(ctx.exception))

    def test_duplicate_finding_ids_fail_closed(self):
        with self.assertRaises(ValueError) as ctx:
            build_techlead_result(review(findings=[finding("REV-001"), finding("REV-001")]))

        self.assertIn("duplicate finding_id", str(ctx.exception))

    def test_lgtm_with_findings_fails_closed(self):
        with self.assertRaises(ValueError) as ctx:
            build_techlead_result(review(status="lgtm", findings=[finding("REV-001")]))

        self.assertIn("lgtm review must not include findings", str(ctx.exception))

    def test_cli_reads_review_and_writes_techlead_artifact(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            review_path = tmp_path / "review.json"
            out_path = tmp_path / "techlead.json"
            review_path.write_text(json.dumps(review()), encoding="utf-8")

            result = subprocess.run(
                [
                    sys.executable,
                    "-m",
                    "codex_review.cli",
                    "stage02-techlead",
                    "--review",
                    str(review_path),
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
            self.assertEqual("codex.stage02.techlead.v1", payload["schema_version"])
            self.assertEqual("needs_design", payload["status"])

    def test_cli_writes_review_comment_body(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            review_payload = review()
            techlead_payload = build_techlead_result(review_payload)
            review_path = tmp_path / "review.json"
            techlead_path = tmp_path / "techlead.json"
            out_path = tmp_path / "comment.md"
            review_path.write_text(json.dumps(review_payload), encoding="utf-8")
            techlead_path.write_text(json.dumps(techlead_payload), encoding="utf-8")

            result = subprocess.run(
                [
                    sys.executable,
                    "-m",
                    "codex_review.cli",
                    "stage02-comment",
                    "--review",
                    str(review_path),
                    "--techlead",
                    str(techlead_path),
                    "--run-url",
                    "https://example.invalid/run/2",
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
            body = out_path.read_text(encoding="utf-8")
            self.assertIn("<!-- codex-review-v3-stage02 -->", body)
            self.assertIn("https://example.invalid/run/2", body)


if __name__ == "__main__":
    unittest.main()
