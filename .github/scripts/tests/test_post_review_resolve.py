import argparse
import importlib.util
import json
import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch


MODULE_PATH = Path(__file__).resolve().parents[1] / "post_review.py"
SPEC = importlib.util.spec_from_file_location("post_review", MODULE_PATH)
post_review = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(post_review)


def pr_payload(
    *,
    draft=False,
    base_ref="main",
    head_repo="DongwonTTuna-Labs/bioden",
    author="DongwonTTuna",
    sender="DongwonTTuna",
):
    return {
        "action": "synchronize",
        "sender": {"login": sender},
        "pull_request": {
            "number": 54,
            "draft": draft,
            "base": {"ref": base_ref, "sha": "base-sha"},
            "head": {"sha": "head-sha", "repo": {"full_name": head_repo}},
            "user": {"login": author},
        },
    }


class ResolveCurrentReviewEventTests(unittest.TestCase):
    def resolve(self, event, **overrides):
        args = {
            "event_name": "pull_request",
            "event": event,
            "repo": "DongwonTTuna-Labs/bioden",
            "actor": "DongwonTTuna",
            "triggering_actor": "DongwonTTuna",
        }
        args.update(overrides)
        return post_review.resolve_current_review_event(**args)

    def test_trusted_pull_request_runs(self):
        result = self.resolve(pr_payload())
        self.assertEqual(result["should_run"], "true")
        self.assertEqual(result["pr_number"], "54")
        self.assertEqual(result["head_sha"], "head-sha")
        self.assertEqual(result["base_sha"], "base-sha")
        self.assertEqual(result["trigger"], "pull_request:synchronize")

    def test_trusted_pull_request_target_runs(self):
        result = self.resolve(pr_payload(), event_name="pull_request_target")
        self.assertEqual(result["should_run"], "true")
        self.assertEqual(result["trigger"], "pull_request_target:synchronize")

    def test_skips_untrusted_triggering_actor(self):
        result = self.resolve(pr_payload(), triggering_actor="somebody-else")
        self.assertEqual(result["should_run"], "false")

    def test_skips_draft_fork_and_non_main(self):
        cases = [
            pr_payload(draft=True),
            pr_payload(base_ref="develop"),
            pr_payload(head_repo="somebody/fork"),
            pr_payload(author="somebody-else"),
            pr_payload(sender="somebody-else"),
        ]
        for event in cases:
            with self.subTest(event=event):
                self.assertEqual(self.resolve(event)["should_run"], "false")

    def test_trusted_issue_comment_runs(self):
        event = {
            "issue": {"number": 54, "pull_request": {}},
            "comment": {"body": "/codex-review", "user": {"login": "DongwonTTuna"}},
        }

        def fetch_pr(path):
            self.assertEqual(path, "/repos/DongwonTTuna-Labs/bioden/pulls/54")
            return pr_payload()["pull_request"]

        result = post_review.resolve_current_review_event(
            event_name="issue_comment",
            event=event,
            repo="DongwonTTuna-Labs/bioden",
            actor="DongwonTTuna",
            triggering_actor="DongwonTTuna",
            fetch_pr=fetch_pr,
        )
        self.assertEqual(result["should_run"], "true")
        self.assertEqual(result["trigger"], "issue_comment:/codex-review")

    def test_skips_non_codex_review_comment(self):
        event = {
            "issue": {"number": 54, "pull_request": {}},
            "comment": {"body": "hello", "user": {"login": "DongwonTTuna"}},
        }
        result = post_review.resolve_current_review_event(
            event_name="issue_comment",
            event=event,
            repo="DongwonTTuna-Labs/bioden",
            actor="DongwonTTuna",
            triggering_actor="DongwonTTuna",
            fetch_pr=lambda path: self.fail(f"unexpected fetch: {path}"),
        )
        self.assertEqual(result["should_run"], "false")


class ResolvePreviousReviewEventTests(unittest.TestCase):
    def resolve(self, event, **overrides):
        args = {
            "event_name": "pull_request",
            "event": event,
            "repo": "DongwonTTuna-Labs/bioden",
            "actor": "DongwonTTuna",
            "triggering_actor": "DongwonTTuna",
        }
        args.update(overrides)
        return post_review.resolve_previous_review_event(**args)

    def test_trusted_pull_request_collects(self):
        result = self.resolve(pr_payload())
        self.assertEqual(result["should_collect"], "true")
        self.assertEqual(result["pr_number"], "54")
        self.assertEqual(result["base_sha"], "base-sha")

    def test_trusted_pull_request_target_collects(self):
        result = self.resolve(pr_payload(), event_name="pull_request_target")
        self.assertEqual(result["should_collect"], "true")
        self.assertEqual(result["pr_number"], "54")
        self.assertEqual(result["base_sha"], "base-sha")

    def test_skips_untrusted_draft_fork_and_non_main(self):
        cases = [
            (pr_payload(), {"triggering_actor": "somebody-else"}),
            (pr_payload(draft=True), {}),
            (pr_payload(base_ref="develop"), {}),
            (pr_payload(head_repo="somebody/fork"), {}),
            (pr_payload(author="somebody-else"), {}),
        ]
        for event, overrides in cases:
            with self.subTest(event=event, overrides=overrides):
                self.assertEqual(self.resolve(event, **overrides)["should_collect"], "false")


def review_thread(
    *,
    author,
    body=None,
    commit_oid="old-sha",
    line=2,
    original_line=None,
    outdated=False,
    resolved=False,
):
    if original_line is None:
        original_line = line
    comment_line = None if outdated else line
    body = body or "\n".join(
        [
            post_review.INLINE_MARKER,
            "<!-- codex-review-id: correctness-1 -->",
            "review body",
        ]
    )
    return {
        "id": "thread-node-id",
        "isResolved": resolved,
        "isOutdated": outdated,
        "path": "src/lib.rs",
        "line": comment_line,
        "originalLine": original_line,
        "comments": {
            "nodes": [
                {
                    "id": "comment-node-id",
                    "fullDatabaseId": "3311706429",
                    "body": body,
                    "author": {"login": author},
                    "commit": {"oid": commit_oid},
                    "originalCommit": {"oid": "old-sha"},
                    "outdated": outdated,
                    "path": "src/lib.rs",
                    "line": comment_line,
                    "originalLine": original_line,
                    "url": "https://github.example/review-comment",
                }
            ]
        },
    }


class CollectResolutionsTests(unittest.TestCase):
    def collect(
        self,
        threads,
        *,
        head_sha="head-sha",
        workspace_text="fn one() {}\nfn two() {}\n",
        stale_batches=None,
    ):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            workspace = root / "workspace"
            batch_dir = root / "batches"
            output_path = root / "github-output"
            (workspace / "src").mkdir(parents=True)
            (workspace / "src" / "lib.rs").write_text(workspace_text, encoding="utf-8")
            if stale_batches:
                batch_dir.mkdir(parents=True)
                for name, payload in stale_batches.items():
                    (batch_dir / name).write_text(
                        json.dumps(payload, ensure_ascii=False, indent=2) + "\n",
                        encoding="utf-8",
                    )
            env = {
                "GITHUB_REPOSITORY": "DongwonTTuna-Labs/rs-builder-relayer-client",
                "PR_NUMBER": "12",
                "HEAD_SHA": head_sha,
                "GITHUB_OUTPUT": str(output_path),
            }
            args = argparse.Namespace(workspace=str(workspace), batch_dir=str(batch_dir))
            with patch.dict(os.environ, env, clear=False), patch.object(
                post_review,
                "collect_review_threads",
                return_value=threads,
            ):
                post_review.command_collect_resolutions(args)
            outputs = output_path.read_text(encoding="utf-8")
            batches = {
                path.name: json.loads(path.read_text(encoding="utf-8"))
                for path in sorted(batch_dir.glob("*.json"))
            }
            return outputs, batches

    def test_collects_previous_inline_comment_from_codex_app_author(self):
        outputs, batches = self.collect(
            [
                review_thread(
                    author=post_review.TRUSTED_CODEX_REVIEW_AUTHORS[0],
                    commit_oid="old-sha",
                )
            ]
        )

        self.assertIn("has_comments=true", outputs)
        self.assertIn("batch_indexes=[0]", outputs)
        self.assertEqual(["resolve-batch-0.json"], list(batches))
        self.assertEqual(3311706429, batches["resolve-batch-0.json"]["comments"][0]["comment_id"])

    def test_collect_removes_stale_resolve_batches(self):
        outputs, batches = self.collect(
            [
                review_thread(
                    author=post_review.TRUSTED_CODEX_REVIEW_AUTHORS[0],
                    commit_oid="old-sha",
                )
            ],
            stale_batches={
                "resolve-batch-1.json": {
                    "comments": [
                        {
                            "comment_id": 999,
                            "thread_id": "stale-thread",
                        }
                    ]
                },
            },
        )

        self.assertIn("has_comments=true", outputs)
        self.assertEqual(["resolve-batch-0.json"], list(batches))
        self.assertEqual(3311706429, batches["resolve-batch-0.json"]["comments"][0]["comment_id"])

    def test_ignores_human_authored_inline_marker_comments(self):
        outputs, batches = self.collect([review_thread(author="DongwonTTuna", commit_oid="old-sha")])

        self.assertIn("has_comments=false", outputs)
        self.assertEqual({}, batches)

    def test_outdated_comment_includes_current_search_context(self):
        body = "\n".join(
            [
                post_review.INLINE_MARKER,
                "<!-- codex-review-id: test-coverage-7 -->",
                "**[SUGGEST][test-coverage] redaction assertion misses lowercase leak**",
                "",
                "`contains(WALLET_OWNER)` misses lowercase address output.",
            ]
        )
        workspace_text = "\n".join(
            [
                "fn unrelated_one() {}",
                "fn stale_original_line() {}",
                "fn unrelated_three() {}",
                "fn unrelated_four() {}",
                "fn redaction_test() {",
                "    let owner_checksum = to_checksum(&address(WALLET_OWNER), None);",
                "    assert!(!debug.contains(&owner_checksum));",
                "    assert!(!debug.contains(&owner_checksum.to_ascii_lowercase()));",
                "}",
            ]
        )

        outputs, batches = self.collect(
            [
                review_thread(
                    author=post_review.TRUSTED_CODEX_REVIEW_AUTHORS[0],
                    body=body,
                    commit_oid="old-sha",
                    original_line=2,
                    outdated=True,
                )
            ],
            workspace_text=workspace_text,
        )

        self.assertIn("has_comments=true", outputs)
        comment = batches["resolve-batch-0.json"]["comments"][0]
        self.assertIn("stale_original_line", comment["code_snippet"])
        self.assertTrue(comment["search_context"])
        search_text = "\n".join(context["snippet"] for context in comment["search_context"])
        self.assertIn("owner_checksum.to_ascii_lowercase", search_text)

    def test_ignores_current_head_inline_comments(self):
        outputs, batches = self.collect(
            [
                review_thread(
                    author=post_review.TRUSTED_CODEX_REVIEW_AUTHORS[0],
                    commit_oid="head-sha",
                )
            ]
        )

        self.assertIn("has_comments=false", outputs)
        self.assertEqual({}, batches)


if __name__ == "__main__":
    unittest.main()
