import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "src"))

from codex_review.stage05 import build_fix_dispatch_result


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
                "files": ["src/lib.rs", "tests/lib.rs"],
            }
        ],
        "test_plan": ["Run the existing Rust test suite."],
        "risk_notes": ["The main risk is preserving current caller behavior."],
    }


def design_chief(status="approved"):
    return {
        "schema_version": "codex.stage04.design_chief.v1",
        "stage": "stage04-design-chief",
        "status": status,
        "can_continue": status == "approved",
        "repository": "DongwonTTuna-Labs/rs-builder-relayer-client",
        "pr_number": "36",
        "base_sha": "a" * 40,
        "head_sha": "b" * 40,
        "target_finding_ids": ["REV-001"],
        "approved_step_ids": ["DES-001"] if status == "approved" else [],
        "rejection_reasons": [] if status == "approved" else ["Design rejected."],
        "summary": "The design is ready to implement.",
    }


class Stage05Tests(unittest.TestCase):
    def test_approved_design_dispatches_fix_tasks_without_push_permission(self):
        result = build_fix_dispatch_result(design(), design_chief())

        self.assertEqual("codex.stage05.fix_dispatch.v1", result["schema_version"])
        self.assertEqual("stage05-fix-dispatch", result["stage"])
        self.assertEqual("ready", result["status"])
        self.assertTrue(result["can_continue"])
        self.assertFalse(result["push_allowed"])
        self.assertEqual([], result["allowed_side_effects"])
        self.assertEqual(1, result["task_count"])
        self.assertEqual("FIX-DES-001", result["tasks"][0]["task_id"])
        self.assertEqual(["src/lib.rs", "tests/lib.rs"], result["tasks"][0]["allowed_files"])

    def test_design_chief_must_be_approved(self):
        with self.assertRaises(ValueError) as ctx:
            build_fix_dispatch_result(design(), design_chief(status="rejected"))

        self.assertIn("stage05 requires approved design_chief", str(ctx.exception))

    def test_approved_step_ids_must_match_design_steps(self):
        chief = design_chief()
        chief["approved_step_ids"] = ["DES-002"]

        with self.assertRaises(ValueError) as ctx:
            build_fix_dispatch_result(design(), chief)

        self.assertIn("approved_step_ids must match design step ids", str(ctx.exception))

    def test_design_step_files_must_be_non_empty(self):
        payload = design()
        payload["implementation_steps"][0]["files"] = []

        with self.assertRaises(ValueError) as ctx:
            build_fix_dispatch_result(payload, design_chief())

        self.assertIn("step files must be a non-empty array", str(ctx.exception))

    def test_cli_reads_inputs_and_writes_fix_dispatch_artifact(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            design_path = tmp_path / "design.json"
            chief_path = tmp_path / "design-chief.json"
            out_path = tmp_path / "fix-dispatch.json"
            design_path.write_text(json.dumps(design()), encoding="utf-8")
            chief_path.write_text(json.dumps(design_chief()), encoding="utf-8")

            result = subprocess.run(
                [
                    sys.executable,
                    "-m",
                    "codex_review.cli",
                    "stage05-fix-dispatch",
                    "--design",
                    str(design_path),
                    "--design-chief",
                    str(chief_path),
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
            self.assertEqual("codex.stage05.fix_dispatch.v1", payload["schema_version"])
            self.assertEqual("ready", payload["status"])


if __name__ == "__main__":
    unittest.main()
