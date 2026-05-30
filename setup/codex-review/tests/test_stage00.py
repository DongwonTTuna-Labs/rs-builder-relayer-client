import json
import subprocess
import sys
import tempfile
import unittest
import hashlib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "src"))

from codex_review.stage00 import build_context_artifacts, build_resolve_gate_result


def inventory(threads):
    return {
        "schema_version": "codex.stage00.thread_inventory.v1",
        "repository": "DongwonTTuna-Labs/rs-builder-relayer-client",
        "pr_number": "36",
        "base_ref": "main",
        "base_sha": "a" * 40,
        "head_sha": "b" * 40,
        "threads": threads,
    }


def thread(thread_id="thread-1", *, forced_state="", needs_human_hint=""):
    return {
        "thread_id": thread_id,
        "file": "src/lib.rs",
        "line": 10,
        "root_cause_key": "root-key",
        "forced_state": forced_state,
        "needs_human_hint": needs_human_hint,
        "comments": [
            {
                "comment_node_id": "comment-1",
                "comment_id": 1,
                "body_sha256": "c" * 64,
            }
        ],
    }


def pr_payload():
    return {
        "baseRefName": "main",
        "headRefName": "feature/codex-review-v3",
        "headRefOid": "b" * 40,
        "headRepository": {"nameWithOwner": "DongwonTTuna-Labs/rs-builder-relayer-client"},
        "files": [{"path": "src/lib.rs", "changeType": "MODIFIED"}],
    }


def review_threads_payload():
    return {
        "data": {
            "repository": {
                "pullRequest": {
                    "reviewThreads": {
                        "pageInfo": {"hasNextPage": False},
                        "nodes": [
                            {
                                "id": "resolved-thread",
                                "isResolved": True,
                                "path": "src/lib.rs",
                                "line": 12,
                                "originalLine": 12,
                                "comments": {
                                    "pageInfo": {"hasNextPage": False},
                                    "nodes": [{"id": "comment-resolved", "databaseId": 1, "body": "resolved"}],
                                },
                            },
                            {
                                "id": "open-thread",
                                "isResolved": False,
                                "path": "src/lib.rs",
                                "line": None,
                                "originalLine": 9,
                                "comments": {
                                    "pageInfo": {"hasNextPage": False},
                                    "nodes": [{"id": "comment-open", "databaseId": 2, "body": "fix this"}],
                                },
                            },
                        ],
                    }
                }
            }
        }
    }


class Stage00Tests(unittest.TestCase):
    def test_build_context_artifacts_hashes_unresolved_review_threads(self):
        result = build_context_artifacts(
            pr_payload(),
            review_threads_payload(),
            repository="DongwonTTuna-Labs/rs-builder-relayer-client",
            pr_number="36",
            base_sha="a" * 40,
            run_id="12345",
            event_name="workflow_dispatch",
        )

        self.assertEqual("main", result["outputs"]["base_ref"])
        self.assertEqual(["src/lib.rs"], [item["path"] for item in result["review_request"]["changed_files"]])
        self.assertEqual("codex.stage00.thread_inventory.v1", result["thread_inventory"]["schema_version"])
        self.assertEqual(1, len(result["thread_inventory"]["threads"]))
        open_thread = result["thread_inventory"]["threads"][0]
        self.assertEqual("open-thread", open_thread["thread_id"])
        self.assertEqual(9, open_thread["line"])
        self.assertEqual(
            hashlib.sha256(b"fix this").hexdigest(),
            open_thread["comments"][0]["body_sha256"],
        )
        self.assertNotIn("body", open_thread["comments"][0])
        self.assertEqual(0, result["run_state"]["loop_count"])

    def test_empty_inventory_is_clear(self):
        result = build_resolve_gate_result(inventory([]))

        self.assertEqual("codex.stage00.resolve_gate.v1", result["schema_version"])
        self.assertEqual("stage00-resolve-gate", result["stage"])
        self.assertEqual("clear", result["status"])
        self.assertTrue(result["can_continue"])
        self.assertEqual([], result["thread_ids"])

    def test_unresolved_threads_require_lifecycle_review(self):
        result = build_resolve_gate_result(inventory([thread("thread-1")]))

        self.assertEqual("needs_lifecycle_review", result["status"])
        self.assertFalse(result["can_continue"])
        self.assertEqual(["thread-1"], result["thread_ids"])
        self.assertEqual([], result["needs_human_thread_ids"])

    def test_forced_needs_human_stops_pipeline(self):
        result = build_resolve_gate_result(
            inventory([thread("thread-1", forced_state="needs_human", needs_human_hint="missing root metadata")])
        )

        self.assertEqual("needs_human", result["status"])
        self.assertFalse(result["can_continue"])
        self.assertEqual(["thread-1"], result["needs_human_thread_ids"])
        self.assertIn("missing root metadata", result["stop_reasons"][0])

    def test_duplicate_thread_ids_fail_closed(self):
        with self.assertRaises(ValueError) as ctx:
            build_resolve_gate_result(inventory([thread("thread-1"), thread("thread-1")]))

        self.assertIn("duplicate thread_id", str(ctx.exception))

    def test_forced_needs_human_requires_hint(self):
        with self.assertRaises(ValueError) as ctx:
            build_resolve_gate_result(inventory([thread("thread-1", forced_state="needs_human")]))

        self.assertIn("needs_human_hint", str(ctx.exception))

    def test_unknown_forced_state_fails_closed(self):
        with self.assertRaises(ValueError) as ctx:
            build_resolve_gate_result(inventory([thread("thread-1", forced_state="resolved")]))

        self.assertIn("unknown forced_state", str(ctx.exception))

    def test_cli_reads_inventory_and_writes_gate_result(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            inventory_path = tmp_path / "inventory.json"
            out_path = tmp_path / "gate.json"
            inventory_path.write_text(json.dumps(inventory([thread("thread-1")])), encoding="utf-8")

            result = subprocess.run(
                [
                    sys.executable,
                    "-m",
                    "codex_review.cli",
                    "stage00-resolve-gate",
                    "--inventory",
                    str(inventory_path),
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
            self.assertEqual("needs_lifecycle_review", payload["status"])

    def test_cli_writes_context_artifacts_and_github_outputs(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            pr_path = tmp_path / "pr.json"
            threads_path = tmp_path / "review-threads.json"
            out_dir = tmp_path / "artifacts"
            github_output = tmp_path / "github-output.txt"
            pr_path.write_text(json.dumps(pr_payload()), encoding="utf-8")
            threads_path.write_text(json.dumps(review_threads_payload()), encoding="utf-8")

            result = subprocess.run(
                [
                    sys.executable,
                    "-m",
                    "codex_review.cli",
                    "stage00-context",
                    "--pr-json",
                    str(pr_path),
                    "--review-threads",
                    str(threads_path),
                    "--repository",
                    "DongwonTTuna-Labs/rs-builder-relayer-client",
                    "--pr-number",
                    "36",
                    "--base-sha",
                    "a" * 40,
                    "--run-id",
                    "12345",
                    "--event-name",
                    "workflow_dispatch",
                    "--out-dir",
                    str(out_dir),
                    "--github-output",
                    str(github_output),
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
            self.assertTrue((out_dir / "thread-inventory.json").exists())
            self.assertTrue((out_dir / "review-request.json").exists())
            self.assertIn("head_ref=feature/codex-review-v3", github_output.read_text(encoding="utf-8"))


if __name__ == "__main__":
    unittest.main()
