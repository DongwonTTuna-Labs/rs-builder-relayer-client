import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "src"))

from codex_review.stage06 import build_fix_merge_result


def dispatch():
    return {
        "schema_version": "codex.stage05.fix_dispatch.v1",
        "stage": "stage05-fix-dispatch",
        "status": "ready",
        "can_continue": True,
        "push_allowed": False,
        "allowed_side_effects": [],
        "repository": "DongwonTTuna-Labs/rs-builder-relayer-client",
        "pr_number": "36",
        "base_sha": "a" * 40,
        "head_sha": "b" * 40,
        "task_count": 1,
        "tasks": [
            {
                "task_id": "FIX-DES-001",
                "source_step_id": "DES-001",
                "title": "Use the correct input",
                "instructions": "Change the value source and keep the existing return type.",
                "allowed_files": ["src/lib.rs", "tests/lib.rs"],
                "target_finding_ids": ["REV-001"],
                "test_plan": ["Run the existing Rust test suite."],
            }
        ],
    }


def fix_outputs(status="completed", touched_files=None):
    if touched_files is None:
        touched_files = ["src/lib.rs"]
    return {
        "schema_version": "codex.stage06.fix_outputs.v1",
        "outputs": [
            {
                "task_id": "FIX-DES-001",
                "status": status,
                "patch": "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-old\n+new\n",
                "touched_files": touched_files,
                "tests": ["cargo test --workspace --all-features"],
                "conflict_reason": "manual merge required" if status == "conflict" else "",
            }
        ],
    }


class Stage06Tests(unittest.TestCase):
    def test_completed_fix_outputs_merge_into_candidate_patch(self):
        result = build_fix_merge_result(dispatch(), fix_outputs())

        self.assertEqual("codex.stage06.fix_merge.v1", result["schema_version"])
        self.assertEqual("stage06-fix-merge", result["stage"])
        self.assertEqual("ready", result["status"])
        self.assertTrue(result["can_continue"])
        self.assertFalse(result["push_allowed"])
        self.assertEqual(["src/lib.rs"], result["touched_files"])
        self.assertIn("diff --git", result["candidate_patch"])
        self.assertEqual([], result["conflicts"])

    def test_missing_task_output_fails_closed(self):
        outputs = fix_outputs()
        outputs["outputs"] = []

        with self.assertRaises(ValueError) as ctx:
            build_fix_merge_result(dispatch(), outputs)

        self.assertIn("fix output task ids must match dispatch task ids", str(ctx.exception))

    def test_touched_files_must_be_allowed_by_task(self):
        with self.assertRaises(ValueError) as ctx:
            build_fix_merge_result(dispatch(), fix_outputs(touched_files=["src/other.rs"]))

        self.assertIn("touched file is not allowed", str(ctx.exception))

    def test_workflow_file_changes_are_blocked_before_patch_policy(self):
        payload = dispatch()
        payload["tasks"][0]["allowed_files"].append(".github/workflows/codex-pr-review.yml")

        with self.assertRaises(ValueError) as ctx:
            build_fix_merge_result(payload, fix_outputs(touched_files=[".github/workflows/codex-pr-review.yml"]))

        self.assertIn("workflow files cannot be touched by fix outputs", str(ctx.exception))

    def test_conflict_output_stops_pipeline_with_conflict_report(self):
        result = build_fix_merge_result(dispatch(), fix_outputs(status="conflict"))

        self.assertEqual("conflict", result["status"])
        self.assertFalse(result["can_continue"])
        self.assertEqual(["FIX-DES-001: manual merge required"], result["conflicts"])
        self.assertEqual("", result["candidate_patch"])

    def test_cli_reads_inputs_and_writes_fix_merge_artifact(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            dispatch_path = tmp_path / "dispatch.json"
            outputs_path = tmp_path / "fix-outputs.json"
            out_path = tmp_path / "fix-merge.json"
            dispatch_path.write_text(json.dumps(dispatch()), encoding="utf-8")
            outputs_path.write_text(json.dumps(fix_outputs()), encoding="utf-8")

            result = subprocess.run(
                [
                    sys.executable,
                    "-m",
                    "codex_review.cli",
                    "stage06-fix-merge",
                    "--dispatch",
                    str(dispatch_path),
                    "--fix-outputs",
                    str(outputs_path),
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
            self.assertEqual("codex.stage06.fix_merge.v1", payload["schema_version"])
            self.assertEqual("ready", payload["status"])


if __name__ == "__main__":
    unittest.main()
