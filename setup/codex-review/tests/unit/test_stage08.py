import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "src"))

from codex_review.stages.stage08_reentry import build_reentry_result


def push():
    return {
        "schema_version": "codex.stage07.push.v1",
        "stage": "stage07-push",
        "status": "pushed",
        "can_continue": True,
        "repository": "DongwonTTuna-Labs/rs-builder-relayer-client",
        "pr_number": "36",
        "previous_head_sha": "b" * 40,
        "pushed_head_sha": "c" * 40,
        "target_branch": "feature/codex-review-v3-orchestrator",
        "trusted_ref": "refs/heads/main",
        "actor": "DongwonTTuna",
        "applied_patch_sha256": "d" * 64,
        "merge_commit": False,
        "pr_merged": False,
        "touched_files": ["src/lib.rs"],
    }


def run_state(loop_count=0, event_name="pull_request_target"):
    return {
        "schema_version": "codex.stage08.run_state.v1",
        "run_id": "123456789",
        "event_name": event_name,
        "loop_count": loop_count,
        "max_loops": 1,
    }


class Stage08Tests(unittest.TestCase):
    def test_push_builds_same_run_reentry_artifact(self):
        state = run_state(event_name="pull_request_target")
        result = build_reentry_result(push(), state)

        self.assertEqual("codex.stage08.reentry.v1", result["schema_version"])
        self.assertEqual("stage08-reentry", result["stage"])
        self.assertEqual("same_run_reentry_ready", result["status"])
        self.assertTrue(result["same_run_reentry"])
        self.assertTrue(result["can_continue"])
        self.assertEqual("stage00-resolve-gate", result["next_stage"])
        self.assertEqual("c" * 40, result["expected_head_sha"])
        self.assertEqual("pull_request_target", result["source_event_name"])
        self.assertEqual("pull_request_target", result["next_event_name"])
        self.assertNotEqual("synchronize", result["next_event_name"])
        self.assertEqual(state["loop_count"] + 1, result["loop_count"])
        self.assertEqual(state["max_loops"], result["max_loops"])

    def test_same_run_loop_is_rejected_when_loop_count_reaches_max(self):
        with self.assertRaises(ValueError) as ctx:
            build_reentry_result(push(), run_state(loop_count=1))

        self.assertIn("same-run reentry loop limit reached", str(ctx.exception))

    def test_stage08_requires_successful_push(self):
        payload = push()
        payload["status"] = "failed"

        with self.assertRaises(ValueError) as ctx:
            build_reentry_result(payload, run_state())

        self.assertIn("stage08 requires pushed stage07 artifact", str(ctx.exception))

    def test_pr_merge_is_rejected(self):
        payload = push()
        payload["pr_merged"] = True

        with self.assertRaises(ValueError) as ctx:
            build_reentry_result(payload, run_state())

        self.assertIn("stage08 must not follow a merged PR", str(ctx.exception))

    def test_cli_reads_inputs_and_writes_reentry_artifact(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            push_path = tmp_path / "push.json"
            run_state_path = tmp_path / "run-state.json"
            out_path = tmp_path / "reentry.json"
            push_path.write_text(json.dumps(push()), encoding="utf-8")
            run_state_path.write_text(json.dumps(run_state()), encoding="utf-8")

            result = subprocess.run(
                [
                    sys.executable,
                    "-m",
                    "codex_review.cli",
                    "stage08-reentry",
                    "--push",
                    str(push_path),
                    "--run-state",
                    str(run_state_path),
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
            self.assertEqual("codex.stage08.reentry.v1", payload["schema_version"])
            self.assertEqual("same_run_reentry_ready", payload["status"])
            self.assertTrue(payload["same_run_reentry"])
            self.assertNotEqual("synchronize", payload["next_event_name"])


if __name__ == "__main__":
    unittest.main()
