import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "src"))

from codex_review.stage07 import (
    assert_workspace_changes_match,
    build_push_result,
    stage_workspace_files,
    validation_command_argv,
)


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
    def test_validation_command_argv_allows_known_validation_commands(self):
        cases = {
            "git diff --check": ["git", "diff", "--check"],
            "cargo fmt --all --check": ["cargo", "fmt", "--all", "--check"],
        }

        for command, expected in cases.items():
            with self.subTest(command=command):
                self.assertEqual(expected, validation_command_argv(command))

    def test_validation_command_argv_rejects_pr_head_test_discovery(self):
        commands = [
            "python3 -m unittest discover -s .github/scripts/codex-review/tests",
            "python3 -m unittest discover -s .github/scripts/tests",
            "python3 -m unittest discover -s .github/scripts/codex-review/tests -p test_stage07.py",
            "python3 .github/scripts/tests/test_codex_pr_review_workflow.py",
            "cargo test --workspace --all-features",
            "cargo clippy --workspace --all-targets --all-features -- -D warnings",
        ]

        for command in commands:
            with self.subTest(command=command):
                with self.assertRaises(ValueError) as ctx:
                    validation_command_argv(command)
                self.assertIn("unsupported validation command", str(ctx.exception))

    def test_validation_command_argv_rejects_shell_suffixes(self):
        commands = [
            "cargo test --workspace --all-features; echo injected",
            "cargo test --workspace --all-features && echo injected",
            "cargo test --workspace --all-features | tee out",
            "cargo test --workspace --all-features > out",
            "cargo test --workspace --all-features $(echo injected)",
            "cargo test --workspace --all-features `echo injected`",
            "python3 .github/scripts/tests/test_codex_pr_review_workflow.py; echo injected",
            "python3 ../scripts/tests/test_codex_pr_review_workflow.py",
        ]

        for command in commands:
            with self.subTest(command=command):
                with self.assertRaises(ValueError) as ctx:
                    validation_command_argv(command)
                self.assertIn("unsupported validation command", str(ctx.exception))

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

    def test_validation_cli_rejects_suffix_before_execution(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            commands_path = tmp_path / "validation-commands.txt"
            commands_path.write_text("cargo test --workspace --all-features; touch injected\n", encoding="utf-8")

            result = subprocess.run(
                [
                    sys.executable,
                    "-m",
                    "codex_review.cli",
                    "stage07-run-validation",
                    "--commands",
                    str(commands_path),
                    "--workspace",
                    str(tmp_path),
                ],
                cwd=ROOT,
                env={"PYTHONPATH": str(ROOT / "src")},
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )

            self.assertEqual("", result.stdout)
            self.assertEqual(1, result.returncode)
            self.assertIn("unsupported validation command", result.stderr)
            self.assertFalse((tmp_path / "injected").exists())

    def test_workspace_revalidation_rejects_validation_side_effect_file(self):
        with tempfile.TemporaryDirectory() as tmp:
            repo = Path(tmp)
            subprocess.run(["git", "init"], cwd=repo, check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            subprocess.run(["git", "config", "user.name", "test"], cwd=repo, check=True)
            subprocess.run(["git", "config", "user.email", "test@example.com"], cwd=repo, check=True)
            (repo / "allowed.txt").write_text("base\n", encoding="utf-8")
            subprocess.run(["git", "add", "allowed.txt"], cwd=repo, check=True)
            subprocess.run(["git", "commit", "-m", "base"], cwd=repo, check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)

            (repo / "allowed.txt").write_text("changed\n", encoding="utf-8")
            subprocess.run(["git", "add", "allowed.txt"], cwd=repo, check=True)
            (repo / "created-by-validation.txt").write_text("side effect\n", encoding="utf-8")

            with self.assertRaises(ValueError) as ctx:
                assert_workspace_changes_match(repo, ["allowed.txt"])

            self.assertIn("workspace changed files mismatch", str(ctx.exception))
            self.assertIn("created-by-validation.txt", str(ctx.exception))

    def test_workspace_revalidation_rejects_validation_side_effect_modified_file(self):
        with tempfile.TemporaryDirectory() as tmp:
            repo = Path(tmp)
            subprocess.run(["git", "init"], cwd=repo, check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            subprocess.run(["git", "config", "user.name", "test"], cwd=repo, check=True)
            subprocess.run(["git", "config", "user.email", "test@example.com"], cwd=repo, check=True)
            (repo / "allowed.txt").write_text("base\n", encoding="utf-8")
            (repo / "outside.txt").write_text("base\n", encoding="utf-8")
            subprocess.run(["git", "add", "allowed.txt", "outside.txt"], cwd=repo, check=True)
            subprocess.run(["git", "commit", "-m", "base"], cwd=repo, check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)

            (repo / "allowed.txt").write_text("changed\n", encoding="utf-8")
            subprocess.run(["git", "add", "allowed.txt"], cwd=repo, check=True)
            (repo / "outside.txt").write_text("side effect\n", encoding="utf-8")

            with self.assertRaises(ValueError) as ctx:
                assert_workspace_changes_match(repo, ["allowed.txt"])

            self.assertIn("workspace changed files mismatch", str(ctx.exception))
            self.assertIn("outside.txt", str(ctx.exception))

    def test_workspace_revalidation_rejects_validation_side_effect_on_allowed_file(self):
        with tempfile.TemporaryDirectory() as tmp:
            repo = Path(tmp)
            subprocess.run(["git", "init"], cwd=repo, check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            subprocess.run(["git", "config", "user.name", "test"], cwd=repo, check=True)
            subprocess.run(["git", "config", "user.email", "test@example.com"], cwd=repo, check=True)
            (repo / "allowed.txt").write_text("base\n", encoding="utf-8")
            subprocess.run(["git", "add", "allowed.txt"], cwd=repo, check=True)
            subprocess.run(["git", "commit", "-m", "base"], cwd=repo, check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)

            (repo / "allowed.txt").write_text("candidate patch\n", encoding="utf-8")
            subprocess.run(["git", "add", "allowed.txt"], cwd=repo, check=True)
            (repo / "allowed.txt").write_text("validation side effect\n", encoding="utf-8")

            with self.assertRaises(ValueError) as ctx:
                assert_workspace_changes_match(repo, ["allowed.txt"])

            self.assertIn("unstaged", str(ctx.exception))
            self.assertIn("allowed.txt", str(ctx.exception))

    def test_stage_workspace_files_stages_only_revalidated_files_and_handles_deletions(self):
        with tempfile.TemporaryDirectory() as tmp:
            repo = Path(tmp)
            subprocess.run(["git", "init"], cwd=repo, check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            subprocess.run(["git", "config", "user.name", "test"], cwd=repo, check=True)
            subprocess.run(["git", "config", "user.email", "test@example.com"], cwd=repo, check=True)
            (repo / "deleted.txt").write_text("base\n", encoding="utf-8")
            (repo / "untouched.txt").write_text("base\n", encoding="utf-8")
            subprocess.run(["git", "add", "deleted.txt", "untouched.txt"], cwd=repo, check=True)
            subprocess.run(["git", "commit", "-m", "base"], cwd=repo, check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)

            (repo / "deleted.txt").unlink()
            (repo / "untouched.txt").write_text("side effect\n", encoding="utf-8")
            stage_workspace_files(repo, ["deleted.txt"])
            cached = subprocess.run(
                ["git", "diff", "--cached", "--name-only"],
                cwd=repo,
                check=True,
                text=True,
                stdout=subprocess.PIPE,
            )

            self.assertEqual(["deleted.txt"], cached.stdout.splitlines())


if __name__ == "__main__":
    unittest.main()
