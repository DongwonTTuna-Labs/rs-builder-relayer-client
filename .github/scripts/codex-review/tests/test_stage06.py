import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "src"))

from codex_review.stage06 import build_conflict_fix_outputs, build_fix_merge_result


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
                "test_plan": ["cargo fmt --all --check"],
            }
        ],
    }


def fix_outputs(status="completed", touched_files=None, patch=None):
    if touched_files is None:
        touched_files = [] if status == "conflict" else ["src/lib.rs"]
    if patch is None:
        patch = (
            ""
            if status == "conflict"
            else "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-old\n+new\n"
        )
    return {
        "schema_version": "codex.stage06.fix_outputs.v1",
        "outputs": [
            {
                "task_id": "FIX-DES-001",
                "status": status,
                "patch": patch,
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
        self.assertEqual(
            ["cargo fmt --all --check", "cargo test --workspace --all-features"],
            result["validation_commands"],
        )

    def test_completed_outputs_touching_same_file_conflict(self):
        payload = dispatch()
        payload["task_count"] = 2
        payload["tasks"] = [
            {
                "task_id": "FIX-DES-001",
                "allowed_files": ["src/lib.rs"],
                "test_plan": [],
            },
            {
                "task_id": "FIX-DES-002",
                "allowed_files": ["src/lib.rs"],
                "test_plan": [],
            },
        ]
        outputs = {
            "schema_version": "codex.stage06.fix_outputs.v1",
            "outputs": [
                {
                    "task_id": "FIX-DES-001",
                    "status": "completed",
                    "patch": (
                        "diff --git a/src/lib.rs b/src/lib.rs\n"
                        "--- a/src/lib.rs\n"
                        "+++ b/src/lib.rs\n"
                        "@@ -1 +1 @@\n"
                        "-old\n"
                        "+new\n"
                    ),
                    "touched_files": ["src/lib.rs"],
                    "tests": [],
                    "conflict_reason": "",
                },
                {
                    "task_id": "FIX-DES-002",
                    "status": "completed",
                    "patch": (
                        "diff --git a/src/lib.rs b/src/lib.rs\n"
                        "--- a/src/lib.rs\n"
                        "+++ b/src/lib.rs\n"
                        "@@ -1 +1 @@\n"
                        "-old\n"
                        "+other\n"
                    ),
                    "touched_files": ["src/lib.rs"],
                    "tests": [],
                    "conflict_reason": "",
                },
            ],
        }

        result = build_fix_merge_result(payload, outputs)

        self.assertEqual("conflict", result["status"])
        self.assertFalse(result["can_continue"])
        self.assertEqual("", result["candidate_patch"])
        self.assertEqual(["src/lib.rs"], result["touched_files"])
        self.assertEqual(
            ["multiple completed outputs touch the same file: src/lib.rs (FIX-DES-001, FIX-DES-002)"],
            result["conflicts"],
        )

    def test_validation_commands_are_deduped_across_dispatch_and_outputs(self):
        payload = dispatch()
        payload["tasks"][0]["test_plan"] = [
            "python3 -m unittest discover -s .github/scripts/codex-review/tests",
            "git diff --check",
        ]
        outputs = fix_outputs()
        outputs["outputs"][0]["tests"] = [
            "git diff --check",
            "actionlint .github/workflows/codex-pr-review.yml",
        ]

        result = build_fix_merge_result(payload, outputs)

        self.assertEqual(
            [
                "python3 -m unittest discover -s .github/scripts/codex-review/tests",
                "git diff --check",
                "actionlint .github/workflows/codex-pr-review.yml",
            ],
            result["validation_commands"],
        )

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

    def test_workflow_file_changes_are_allowed_when_task_allows_them(self):
        payload = dispatch()
        payload["tasks"][0]["allowed_files"].append(".github/workflows/codex-pr-review.yml")
        outputs = fix_outputs(touched_files=[".github/workflows/codex-pr-review.yml"])
        outputs["outputs"][0]["patch"] = (
            "diff --git a/.github/workflows/codex-pr-review.yml b/.github/workflows/codex-pr-review.yml\n"
            "--- a/.github/workflows/codex-pr-review.yml\n"
            "+++ b/.github/workflows/codex-pr-review.yml\n"
            "@@ -1 +1 @@\n-old\n+new\n"
        )

        result = build_fix_merge_result(payload, outputs)

        self.assertEqual([".github/workflows/codex-pr-review.yml"], result["touched_files"])

    def test_patch_files_must_match_touched_files(self):
        with self.assertRaises(ValueError) as ctx:
            build_fix_merge_result(dispatch(), fix_outputs(touched_files=["tests/lib.rs"]))

        self.assertIn("patch files must match touched_files", str(ctx.exception))

    def test_patch_files_must_be_allowed_even_when_touched_files_claim_allowed(self):
        payload = fix_outputs()
        payload["outputs"][0]["patch"] = (
            "diff --git a/.github/workflows/codex-pr-review.yml b/.github/workflows/codex-pr-review.yml\n"
            "--- a/.github/workflows/codex-pr-review.yml\n"
            "+++ b/.github/workflows/codex-pr-review.yml\n"
            "@@ -1 +1 @@\n-old\n+new\n"
        )

        with self.assertRaises(ValueError) as ctx:
            build_fix_merge_result(dispatch(), payload)

        self.assertIn("touched file is not allowed", str(ctx.exception))

    def test_candidate_patch_preserves_trailing_blank_context_lines(self):
        payload = dispatch()
        payload["task_count"] = 2
        payload["tasks"] = [
            {
                "task_id": "FIX-DES-001",
                "allowed_files": ["a.txt"],
            },
            {
                "task_id": "FIX-DES-002",
                "allowed_files": ["b.txt"],
            },
        ]
        outputs = {
            "schema_version": "codex.stage06.fix_outputs.v1",
            "outputs": [
                {
                    "task_id": "FIX-DES-001",
                    "status": "completed",
                    "patch": (
                        "diff --git a/a.txt b/a.txt\n"
                        "--- a/a.txt\n"
                        "+++ b/a.txt\n"
                        "@@ -1,3 +1,3 @@\n"
                        " one\n"
                        "-two\n"
                        "+TWO\n"
                        " \n"
                    ),
                    "touched_files": ["a.txt"],
                    "tests": [],
                    "conflict_reason": "",
                },
                {
                    "task_id": "FIX-DES-002",
                    "status": "completed",
                    "patch": (
                        "diff --git a/b.txt b/b.txt\n"
                        "--- a/b.txt\n"
                        "+++ b/b.txt\n"
                        "@@ -1 +1 @@\n"
                        "-alpha\n"
                        "+BETA\n"
                    ),
                    "touched_files": ["b.txt"],
                    "tests": [],
                    "conflict_reason": "",
                },
            ],
        }
        result = build_fix_merge_result(payload, outputs)

        self.assertIn("+TWO\n \ndiff --git a/b.txt", result["candidate_patch"])
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            (tmp_path / "a.txt").write_text("one\ntwo\n\n", encoding="utf-8")
            (tmp_path / "b.txt").write_text("alpha\n", encoding="utf-8")
            patch_path = tmp_path / "candidate.patch"
            patch_path.write_text(result["candidate_patch"], encoding="utf-8")
            check = subprocess.run(
                ["git", "apply", "--check", str(patch_path)],
                cwd=tmp_path,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )

        self.assertEqual("", check.stderr)
        self.assertEqual(0, check.returncode)

    def test_conflict_output_stops_pipeline_with_conflict_report(self):
        result = build_fix_merge_result(dispatch(), fix_outputs(status="conflict"))

        self.assertEqual("conflict", result["status"])
        self.assertFalse(result["can_continue"])
        self.assertEqual(["FIX-DES-001: manual merge required"], result["conflicts"])
        self.assertEqual("", result["candidate_patch"])

    def test_conflict_output_may_have_no_touched_files(self):
        result = build_fix_merge_result(dispatch(), fix_outputs(status="conflict", touched_files=[]))

        self.assertEqual("conflict", result["status"])
        self.assertFalse(result["can_continue"])
        self.assertEqual([], result["touched_files"])
        self.assertEqual(["FIX-DES-001: manual merge required"], result["conflicts"])

    def test_build_conflict_fix_outputs_covers_every_dispatch_task(self):
        payload = dispatch()
        payload["task_count"] = 2
        payload["tasks"].append(
            {
                "task_id": "FIX-DES-002",
                "allowed_files": ["README.md"],
            }
        )

        outputs = build_conflict_fix_outputs(payload, "codex action did not produce fix-outputs.json")

        self.assertEqual("codex.stage06.fix_outputs.v1", outputs["schema_version"])
        self.assertEqual(["FIX-DES-001", "FIX-DES-002"], [output["task_id"] for output in outputs["outputs"]])
        for output in outputs["outputs"]:
            self.assertEqual("conflict", output["status"])
            self.assertEqual("", output["patch"])
            self.assertEqual([], output["touched_files"])
            self.assertEqual([], output["tests"])
            self.assertIn("codex action did not produce fix-outputs.json", output["conflict_reason"])

        result = build_fix_merge_result(payload, outputs)
        self.assertEqual("conflict", result["status"])
        self.assertFalse(result["can_continue"])

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

    def test_cli_writes_conflict_fix_outputs_fallback_artifact(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            dispatch_path = tmp_path / "dispatch.json"
            out_path = tmp_path / "fix-outputs.json"
            dispatch_path.write_text(json.dumps(dispatch()), encoding="utf-8")

            result = subprocess.run(
                [
                    sys.executable,
                    "-m",
                    "codex_review.cli",
                    "stage05-fallback-fix-outputs",
                    "--dispatch",
                    str(dispatch_path),
                    "--reason",
                    "codex action did not produce fix-outputs.json",
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
            self.assertEqual("codex.stage06.fix_outputs.v1", payload["schema_version"])
            self.assertEqual("conflict", payload["outputs"][0]["status"])
            self.assertIn("codex action did not produce fix-outputs.json", payload["outputs"][0]["conflict_reason"])


if __name__ == "__main__":
    unittest.main()
