import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "src"))

from codex_review.stage03 import build_design_result


def finding(finding_id="REV-001"):
    return {
        "finding_id": finding_id,
        "axis": "correctness",
        "severity": "must",
        "file": "src/lib.rs",
        "line": 10,
        "title": "Fix incorrect value",
        "body": "The value is computed from the wrong input.",
        "root_cause_key": f"root-{finding_id.lower()}",
    }


def review(findings=None):
    if findings is None:
        findings = [finding()]
    return {
        "schema_version": "codex.stage01.review.v1",
        "stage": "stage01-review",
        "status": "needs_work",
        "can_continue": True,
        "repository": "DongwonTTuna-Labs/rs-builder-relayer-client",
        "pr_number": "36",
        "base_sha": "a" * 40,
        "head_sha": "b" * 40,
        "reviewed_axes": ["correctness", "tests"],
        "changed_files": [{"path": "src/lib.rs", "status": "modified"}],
        "summary": "review summary",
        "axis_results": [
            {"axis": "correctness", "status": "needs_work", "summary": "correctness summary"},
            {"axis": "tests", "status": "lgtm", "summary": "tests summary"},
        ],
        "finding_count": len(findings),
        "findings": findings,
    }


def techlead(blocking_ids=None, requires_design=True):
    if blocking_ids is None:
        blocking_ids = ["REV-001"]
    return {
        "schema_version": "codex.stage02.techlead.v1",
        "stage": "stage02-techlead",
        "status": "needs_design" if requires_design else "approved",
        "can_continue": True,
        "requires_design": requires_design,
        "requires_fix": requires_design,
        "repository": "DongwonTTuna-Labs/rs-builder-relayer-client",
        "pr_number": "36",
        "base_sha": "a" * 40,
        "head_sha": "b" * 40,
        "reviewed_axes": ["correctness", "tests"],
        "finding_count": 1 if blocking_ids else 0,
        "blocking_finding_ids": blocking_ids,
        "non_blocking_finding_ids": [],
        "summary": "review summary",
    }


def model_design(target_ids=None):
    if target_ids is None:
        target_ids = ["REV-001"]
    return {
        "schema_version": "codex.stage03.model_design.v1",
        "summary": "Use the validated input source for the value.",
        "target_finding_ids": target_ids,
        "assumptions": ["The existing public API must remain unchanged."],
        "implementation_steps": [
            {
                "step_id": "DES-001",
                "title": "Use the correct input",
                "description": "Change the value source and keep the existing return type.",
                "files": ["src/lib.rs"],
            }
        ],
        "test_plan": ["cargo test --workspace --all-features"],
        "risk_notes": ["The main risk is preserving current caller behavior."],
    }


class Stage03Tests(unittest.TestCase):
    def test_design_result_normalizes_model_plan(self):
        result = build_design_result(review(), techlead(), model_design())

        self.assertEqual("codex.stage03.design.v1", result["schema_version"])
        self.assertEqual("stage03-design", result["stage"])
        self.assertEqual("ready", result["status"])
        self.assertTrue(result["can_continue"])
        self.assertTrue(result["requires_design"])
        self.assertEqual(["REV-001"], result["target_finding_ids"])
        self.assertEqual("DES-001", result["implementation_steps"][0]["step_id"])
        self.assertEqual(["cargo test --workspace --all-features"], result["test_plan"])

    def test_stage03_requires_design_required_techlead(self):
        with self.assertRaises(ValueError) as ctx:
            build_design_result(review(), techlead(blocking_ids=[], requires_design=False), model_design())

        self.assertIn("stage03 requires techlead.requires_design", str(ctx.exception))

    def test_model_target_ids_must_match_blocking_findings(self):
        with self.assertRaises(ValueError) as ctx:
            build_design_result(review(), techlead(blocking_ids=["REV-001"]), model_design(target_ids=["REV-002"]))

        self.assertIn("target_finding_ids must match blocking_finding_ids", str(ctx.exception))

    def test_design_steps_must_be_non_empty(self):
        payload = model_design()
        payload["implementation_steps"] = []

        with self.assertRaises(ValueError) as ctx:
            build_design_result(review(), techlead(), payload)

        self.assertIn("implementation_steps must be a non-empty array", str(ctx.exception))

    def test_test_plan_must_be_non_empty(self):
        payload = model_design()
        payload["test_plan"] = []

        with self.assertRaises(ValueError) as ctx:
            build_design_result(review(), techlead(), payload)

        self.assertIn("test_plan must be a non-empty array", str(ctx.exception))

    def test_cli_reads_inputs_and_writes_design_artifact(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            review_path = tmp_path / "review.json"
            techlead_path = tmp_path / "techlead.json"
            model_design_path = tmp_path / "model-design.json"
            out_path = tmp_path / "design.json"
            review_path.write_text(json.dumps(review()), encoding="utf-8")
            techlead_path.write_text(json.dumps(techlead()), encoding="utf-8")
            model_design_path.write_text(json.dumps(model_design()), encoding="utf-8")

            result = subprocess.run(
                [
                    sys.executable,
                    "-m",
                    "codex_review.cli",
                    "stage03-design",
                    "--review",
                    str(review_path),
                    "--techlead",
                    str(techlead_path),
                    "--model-design",
                    str(model_design_path),
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
            self.assertEqual("codex.stage03.design.v1", payload["schema_version"])
            self.assertEqual("ready", payload["status"])


if __name__ == "__main__":
    unittest.main()
