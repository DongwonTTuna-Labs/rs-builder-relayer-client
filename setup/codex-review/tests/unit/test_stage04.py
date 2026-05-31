import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "src"))

from codex_review.stages.stage04_design_chief import build_design_chief_result


def design():
    return {
        "schema_version": "codex.stage03.design.v1",
        "stage": "stage03-design",
        "status": "ready",
        "can_continue": True,
        "requires_design": True,
        "requires_fix": True,
        "repository": "DongwonTTuna-Labs/rs-builder-relayer-client",
        "pr_number": "36",
        "base_sha": "a" * 40,
        "head_sha": "b" * 40,
        "target_finding_ids": ["REV-001"],
        "summary": "Use the validated input source for the value.",
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


def model_approval(status="approved", reviewed_step_ids=None, rejection_reasons=None):
    if reviewed_step_ids is None:
        reviewed_step_ids = ["DES-001"]
    if rejection_reasons is None:
        rejection_reasons = [] if status == "approved" else ["Plan does not preserve the public API."]
    return {
        "schema_version": "codex.stage04.model_design_chief.v1",
        "status": status,
        "summary": "The design is ready to implement." if status == "approved" else "The design needs revision.",
        "reviewed_step_ids": reviewed_step_ids,
        "rejection_reasons": rejection_reasons,
    }


class Stage04Tests(unittest.TestCase):
    def test_approved_design_can_continue(self):
        result = build_design_chief_result(design(), model_approval())

        self.assertEqual("codex.stage04.design_chief.v1", result["schema_version"])
        self.assertEqual("stage04-design-chief", result["stage"])
        self.assertEqual("approved", result["status"])
        self.assertTrue(result["can_continue"])
        self.assertEqual(["DES-001"], result["approved_step_ids"])
        self.assertEqual([], result["rejection_reasons"])

    def test_rejected_design_stops_pipeline(self):
        result = build_design_chief_result(design(), model_approval(status="rejected"))

        self.assertEqual("rejected", result["status"])
        self.assertFalse(result["can_continue"])
        self.assertEqual(["Plan does not preserve the public API."], result["rejection_reasons"])

    def test_reviewed_step_ids_must_match_design_steps(self):
        with self.assertRaises(ValueError) as ctx:
            build_design_chief_result(design(), model_approval(reviewed_step_ids=["DES-002"]))

        self.assertIn("reviewed_step_ids must match design step ids", str(ctx.exception))

    def test_rejected_design_requires_reason(self):
        with self.assertRaises(ValueError) as ctx:
            build_design_chief_result(design(), model_approval(status="rejected", rejection_reasons=[]))

        self.assertIn("rejected design requires rejection_reasons", str(ctx.exception))

    def test_unknown_status_fails_closed(self):
        with self.assertRaises(ValueError) as ctx:
            build_design_chief_result(design(), model_approval(status="needs_more"))

        self.assertIn("unknown design chief status", str(ctx.exception))

    def test_cli_reads_inputs_and_writes_design_chief_artifact(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            design_path = tmp_path / "design.json"
            approval_path = tmp_path / "approval.json"
            out_path = tmp_path / "design-chief.json"
            design_path.write_text(json.dumps(design()), encoding="utf-8")
            approval_path.write_text(json.dumps(model_approval()), encoding="utf-8")

            result = subprocess.run(
                [
                    sys.executable,
                    "-m",
                    "codex_review.cli",
                    "stage04-design-chief",
                    "--design",
                    str(design_path),
                    "--model-approval",
                    str(approval_path),
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
            self.assertEqual("codex.stage04.design_chief.v1", payload["schema_version"])
            self.assertEqual("approved", payload["status"])


if __name__ == "__main__":
    unittest.main()
