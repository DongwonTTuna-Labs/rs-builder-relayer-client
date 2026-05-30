import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "src"))

from codex_review.stage07 import build_push_result


def fix_merge():
    return {
        "schema_version": "codex.stage06.fix_merge.v1",
        "stage": "stage06-fix-merge",
        "status": "ready",
        "can_continue": True,
        "push_allowed": False,
        "repository": "DongwonTTuna-Labs/rs-builder-relayer-client",
        "pr_number": "36",
        "base_sha": "a" * 40,
        "head_sha": "b" * 40,
        "task_ids": ["FIX-DES-001"],
        "touched_files": ["src/lib.rs"],
        "validation_commands": ["python3 -m unittest discover -s .github/scripts/codex-review/tests"],
        "candidate_patch": "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-old\n+new\n",
        "conflicts": [],
    }


def trusted_push():
    return {
        "schema_version": "codex.stage07.trusted_push.v1",
        "trusted_job": True,
        "trusted_ref": "refs/heads/main",
        "actor": "DongwonTTuna",
        "target_branch": "feature/codex-review-v3-orchestrator",
        "pushed_head_sha": "c" * 40,
        "applied_patch_sha256": "d" * 64,
        "merge_commit": False,
        "pr_merged": False,
    }


class Stage07Tests(unittest.TestCase):
    def test_trusted_push_record_can_continue_without_merging_pr(self):
        result = build_push_result(fix_merge(), trusted_push())

        self.assertEqual("codex.stage07.push.v1", result["schema_version"])
        self.assertEqual("stage07-push", result["stage"])
        self.assertEqual("pushed", result["status"])
        self.assertTrue(result["can_continue"])
        self.assertFalse(result["pr_merged"])
        self.assertEqual("c" * 40, result["pushed_head_sha"])
        self.assertEqual(["src/lib.rs"], result["touched_files"])
        self.assertEqual(["python3 -m unittest discover -s .github/scripts/codex-review/tests"], result["validation_commands"])

    def test_stage07_requires_validation_commands(self):
        payload = fix_merge()
        payload["validation_commands"] = []

        with self.assertRaises(ValueError) as ctx:
            build_push_result(payload, trusted_push())

        self.assertIn("validation_commands", str(ctx.exception))

    def test_stage07_requires_ready_fix_merge(self):
        payload = fix_merge()
        payload["status"] = "conflict"
        payload["can_continue"] = False

        with self.assertRaises(ValueError) as ctx:
            build_push_result(payload, trusted_push())

        self.assertIn("stage07 requires ready fix_merge", str(ctx.exception))

    def test_trusted_job_proof_is_required(self):
        proof = trusted_push()
        proof["trusted_job"] = False

        with self.assertRaises(ValueError) as ctx:
            build_push_result(fix_merge(), proof)

        self.assertIn("trusted_job must be true", str(ctx.exception))

    def test_pr_merge_is_rejected(self):
        proof = trusted_push()
        proof["pr_merged"] = True

        with self.assertRaises(ValueError) as ctx:
            build_push_result(fix_merge(), proof)

        self.assertIn("stage07 must not merge PRs", str(ctx.exception))

    def test_patch_hash_must_be_sha256(self):
        proof = trusted_push()
        proof["applied_patch_sha256"] = "short"

        with self.assertRaises(ValueError) as ctx:
            build_push_result(fix_merge(), proof)

        self.assertIn("applied_patch_sha256 must be a sha256 hex digest", str(ctx.exception))

    def test_cli_reads_inputs_and_writes_push_artifact(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            merge_path = tmp_path / "fix-merge.json"
            trusted_push_path = tmp_path / "trusted-push.json"
            out_path = tmp_path / "push.json"
            merge_path.write_text(json.dumps(fix_merge()), encoding="utf-8")
            trusted_push_path.write_text(json.dumps(trusted_push()), encoding="utf-8")

            result = subprocess.run(
                [
                    sys.executable,
                    "-m",
                    "codex_review.cli",
                    "stage07-push",
                    "--fix-merge",
                    str(merge_path),
                    "--trusted-push",
                    str(trusted_push_path),
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
            self.assertEqual("codex.stage07.push.v1", payload["schema_version"])
            self.assertEqual("pushed", payload["status"])


if __name__ == "__main__":
    unittest.main()
