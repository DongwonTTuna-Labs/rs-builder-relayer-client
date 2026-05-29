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
        self.assertEqual(result["trigger_class"], "human_trusted_synchronize")

    def test_trusted_pull_request_target_runs(self):
        result = self.resolve(pr_payload(), event_name="pull_request_target")
        self.assertEqual(result["should_run"], "true")
        self.assertEqual(result["trigger"], "pull_request_target:synchronize")
        self.assertEqual(result["trigger_class"], "human_trusted_synchronize")

    def test_codex_autofix_bot_synchronize_runs_for_trusted_commit(self):
        bot = post_review.TRUSTED_CODEX_REVIEW_AUTHORS[1]

        def fetch_commit(path):
            self.assertEqual(path, "/repos/DongwonTTuna-Labs/bioden/commits/head-sha")
            return {
                "commit": {"message": post_review.CODEX_AUTOFIX_COMMIT_SUBJECT + "\n"},
                "author": {"login": bot},
                "committer": {"login": bot},
            }

        def fetch_compare(path):
            self.assertEqual(path, "/repos/DongwonTTuna-Labs/bioden/compare/base-sha...head-sha")
            return {
                "commits": [
                    {"sha": "head-sha", "commit": {"message": post_review.CODEX_AUTOFIX_COMMIT_SUBJECT + "\n"}}
                ]
            }

        result = self.resolve(
            pr_payload(sender=bot),
            event_name="pull_request_target",
            actor=bot,
            triggering_actor=bot,
            fetch_commit=fetch_commit,
            fetch_compare=fetch_compare,
        )

        self.assertEqual(result["should_run"], "true")
        self.assertEqual(result["trigger_class"], "codex_app_autofix_synchronize")

    def test_codex_autofix_bot_synchronize_requires_trusted_commit_subject(self):
        bot = post_review.TRUSTED_CODEX_REVIEW_AUTHORS[1]

        result = self.resolve(
            pr_payload(sender=bot),
            event_name="pull_request_target",
            actor=bot,
            triggering_actor=bot,
            fetch_commit=lambda path: {
                "commit": {"message": "chore: unrelated"},
                "author": {"login": bot},
                "committer": {"login": bot},
            },
            fetch_compare=lambda path: self.fail(f"unexpected compare fetch: {path}"),
        )

        self.assertEqual(result["should_run"], "false")

    def test_codex_autofix_bot_synchronize_requires_author_and_committer_login(self):
        bot = post_review.TRUSTED_CODEX_REVIEW_AUTHORS[1]

        for missing_side in ("author", "committer"):
            with self.subTest(missing_side=missing_side):
                result = self.resolve(
                    pr_payload(sender=bot),
                    event_name="pull_request_target",
                    actor=bot,
                    triggering_actor=bot,
                    fetch_commit=lambda path, missing_side=missing_side: {
                        "commit": {"message": post_review.CODEX_AUTOFIX_COMMIT_SUBJECT + "\n"},
                        "author": None if missing_side == "author" else {"login": bot},
                        "committer": None if missing_side == "committer" else {"login": bot},
                    },
                    fetch_compare=lambda path: self.fail(f"unexpected compare fetch: {path}"),
                )

                self.assertEqual(result["should_run"], "false")

    def test_codex_autofix_bot_synchronize_enforces_commit_cap(self):
        bot = post_review.TRUSTED_CODEX_REVIEW_AUTHORS[1]
        too_many = [
            {
                "sha": f"sha-{index}",
                "commit": {"message": post_review.CODEX_AUTOFIX_COMMIT_SUBJECT + "\n"},
            }
            for index in range(post_review.AUTOFIX_MAX_COMMITS)
        ]
        too_many.append(
            {"sha": "head-sha", "commit": {"message": post_review.CODEX_AUTOFIX_COMMIT_SUBJECT + "\n"}}
        )

        result = self.resolve(
            pr_payload(sender=bot),
            event_name="pull_request_target",
            actor=bot,
            triggering_actor=bot,
            fetch_commit=lambda path: {
                "commit": {"message": post_review.CODEX_AUTOFIX_COMMIT_SUBJECT + "\n"},
                "author": {"login": bot},
                "committer": {"login": bot},
            },
            fetch_compare=lambda path: {"commits": too_many},
        )

        self.assertEqual(result["should_run"], "false")

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

    def test_trusted_workflow_dispatch_collects_requested_pr(self):
        def fetch_pr(path):
            self.assertEqual(path, "/repos/DongwonTTuna-Labs/bioden/pulls/54")
            return pr_payload()["pull_request"]

        result = self.resolve(
            {"inputs": {"pr_number": "54"}},
            event_name="workflow_dispatch",
            fetch_pr=fetch_pr,
        )

        self.assertEqual(result["should_collect"], "true")
        self.assertEqual(result["pr_number"], "54")
        self.assertEqual(result["head_sha"], "head-sha")

    def test_trusted_workflow_run_collects_after_codex_pr_review(self):
        def fetch_pr(path):
            self.assertEqual(path, "/repos/DongwonTTuna-Labs/bioden/pulls/54")
            return pr_payload()["pull_request"]

        result = self.resolve(
            {
                "workflow_run": {
                    "name": "Codex PR Review",
                    "event": "pull_request_target",
                    "conclusion": "success",
                    "pull_requests": [{"number": 54}],
                }
            },
            event_name="workflow_run",
            fetch_pr=fetch_pr,
        )

        self.assertEqual(result["should_collect"], "true")
        self.assertEqual(result["pr_number"], "54")

    def test_trusted_bot_workflow_run_collects_after_codex_pr_review(self):
        bot = post_review.TRUSTED_CODEX_REVIEW_AUTHORS[1]

        def fetch_pr(path):
            self.assertEqual(path, "/repos/DongwonTTuna-Labs/bioden/pulls/54")
            return pr_payload()["pull_request"]

        result = self.resolve(
            {
                "workflow_run": {
                    "name": "Codex PR Review",
                    "event": "pull_request_target",
                    "conclusion": "success",
                    "head_sha": "head-sha",
                    "pull_requests": [{"number": 54}],
                }
            },
            event_name="workflow_run",
            actor=bot,
            triggering_actor=bot,
            fetch_pr=fetch_pr,
        )

        self.assertEqual(result["should_collect"], "true")
        self.assertEqual(result["pr_number"], "54")

    def test_workflow_run_skips_when_completed_head_is_not_current_pr_head(self):
        result = self.resolve(
            {
                "workflow_run": {
                    "name": "Codex PR Review",
                    "event": "pull_request_target",
                    "conclusion": "success",
                    "head_sha": "stale-sha",
                    "pull_requests": [{"number": 54}],
                }
            },
            event_name="workflow_run",
            fetch_pr=lambda path: pr_payload()["pull_request"],
        )

        self.assertEqual(result["should_collect"], "false")

    def test_workflow_run_skips_non_review_or_missing_pr(self):
        cases = [
            {"workflow_run": {"name": "CI", "event": "pull_request_target", "pull_requests": [{"number": 54}]}},
            {"workflow_run": {"name": "Codex PR Review", "event": "issue_comment", "pull_requests": [{"number": 54}]}},
            {"workflow_run": {"name": "Codex PR Review", "event": "pull_request_target", "conclusion": "cancelled", "pull_requests": [{"number": 54}]}},
            {"workflow_run": {"name": "Codex PR Review", "event": "pull_request_target", "conclusion": "success", "pull_requests": []}},
        ]
        for event in cases:
            with self.subTest(event=event):
                result = self.resolve(
                    event,
                    event_name="workflow_run",
                    fetch_pr=lambda path: self.fail(f"unexpected fetch: {path}"),
                )
                self.assertEqual(result["should_collect"], "false")

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
    original_commit_oid="old-sha",
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
                    "originalCommit": {"oid": original_commit_oid} if original_commit_oid is not None else None,
                    "outdated": outdated,
                    "path": "src/lib.rs",
                    "line": comment_line,
                    "originalLine": original_line,
                    "url": "https://github.example/review-comment",
                }
            ]
        },
    }


def append_review_comment(thread, *, body, comment_id="3311706431", line=4):
    thread["comments"]["nodes"].append(
        {
            "id": f"comment-node-id-{comment_id}",
            "fullDatabaseId": comment_id,
            "body": body,
            "author": {"login": post_review.TRUSTED_CODEX_REVIEW_AUTHORS[0]},
            "commit": {"oid": "old-sha"},
            "originalCommit": {"oid": "old-sha"},
            "path": "src/lib.rs",
            "line": line,
            "originalLine": line,
            "url": f"https://github.example/review-comment-{comment_id}",
        }
    )


class CollectResolutionsTests(unittest.TestCase):
    def collect(
        self,
        threads,
        *,
        head_sha="head-sha",
        stale_batches=None,
    ):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            batch_dir = root / "batches"
            output_path = root / "github-output"
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
            args = argparse.Namespace(batch_dir=str(batch_dir))
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

    def test_collects_reanchored_old_comment_using_original_commit(self):
        outputs, batches = self.collect(
            [
                review_thread(
                    author=post_review.TRUSTED_CODEX_REVIEW_AUTHORS[0],
                    commit_oid="head-sha",
                    original_commit_oid="old-sha",
                )
            ]
        )

        self.assertIn("has_comments=true", outputs)
        comment = batches["resolve-batch-0.json"]["comments"][0]
        self.assertEqual("head-sha", comment["current_commit_oid"])
        self.assertEqual("old-sha", comment["original_commit_oid"])
        self.assertNotIn("code_snippet", comment)
        self.assertNotIn("search_context", comment)

    def test_ignores_human_authored_inline_marker_comments(self):
        outputs, batches = self.collect([review_thread(author="DongwonTTuna", commit_oid="old-sha")])

        self.assertIn("has_comments=false", outputs)
        self.assertEqual({}, batches)

    def test_outdated_comment_is_minimal_thread_pointer(self):
        body = "\n".join(
            [
                post_review.INLINE_MARKER,
                "<!-- codex-review-id: test-coverage-7 -->",
                "**[SUGGEST][test-coverage] redaction assertion misses lowercase leak**",
                "",
                "`contains(WALLET_OWNER)` misses lowercase address output.",
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
        )

        self.assertIn("has_comments=true", outputs)
        comment = batches["resolve-batch-0.json"]["comments"][0]
        self.assertEqual("test-coverage-7", comment["marker_key"])
        self.assertEqual("old-sha", comment["original_commit_oid"])
        self.assertNotIn("code_snippet", comment)
        self.assertNotIn("search_context", comment)

    def test_ignores_current_head_inline_comments(self):
        outputs, batches = self.collect(
            [
                review_thread(
                    author=post_review.TRUSTED_CODEX_REVIEW_AUTHORS[0],
                    commit_oid="head-sha",
                    original_commit_oid="head-sha",
                )
            ]
        )

        self.assertIn("has_comments=false", outputs)
        self.assertEqual({}, batches)

    def test_missing_original_commit_falls_back_to_current_commit(self):
        outputs, batches = self.collect(
            [
                review_thread(
                    author=post_review.TRUSTED_CODEX_REVIEW_AUTHORS[0],
                    commit_oid="head-sha",
                    original_commit_oid=None,
                )
            ]
        )

        self.assertIn("has_comments=false", outputs)
        self.assertEqual({}, batches)

    def test_missing_original_commit_collects_non_head_commit(self):
        outputs, batches = self.collect(
            [
                review_thread(
                    author=post_review.TRUSTED_CODEX_REVIEW_AUTHORS[0],
                    commit_oid="old-sha",
                    original_commit_oid=None,
                )
            ]
        )

        self.assertIn("has_comments=true", outputs)
        self.assertEqual(3311706429, batches["resolve-batch-0.json"]["comments"][0]["comment_id"])


class ThreadLifecycleV3Tests(unittest.TestCase):
    def test_trusted_author_check_rejects_generic_bot_logins(self):
        self.assertTrue(post_review.is_trusted_codex_review_author(post_review.TRUSTED_CODEX_REVIEW_AUTHORS[0]))
        self.assertFalse(post_review.is_trusted_codex_review_author("random-reviewer[bot]"))

    def test_normalize_finding_rejects_empty_root_cause_key(self):
        with self.assertRaises(SystemExit):
            post_review.normalize_finding(
                "correctness",
                {
                    "id": "correctness-1",
                    "type": "MUST",
                    "file": "src/lib.rs",
                    "line": 7,
                    "title": "title",
                    "reason": "reason",
                    "root_cause_key": "",
                },
            )

    def test_normalize_finding_rejects_finding_id_root_cause_key(self):
        with self.assertRaises(SystemExit):
            post_review.normalize_finding(
                "correctness",
                {
                    "id": "correctness-1",
                    "type": "MUST",
                    "file": "src/lib.rs",
                    "line": 7,
                    "title": "title",
                    "reason": "reason",
                    "root_cause_key": "correctness-1",
                },
            )

    def test_tech_lead_primary_root_cause_key_must_match_known_root_cause(self):
        findings = [{"id": "correctness-1", "root_cause_key": "deposit-wallet-submit-state"}]
        decisions = {
            "by_id": {
                "correctness-1": {
                    "action": "publish_and_fix_now",
                    "reason": "publish representative",
                    "primary_root_cause_key": "correctness-1",
                }
            }
        }

        with self.assertRaises(SystemExit):
            post_review.validate_decision_coverage(findings, decisions)

    def test_thread_inventory_skips_existing_terminal_lifecycle_marker(self):
        thread = review_thread(author=post_review.TRUSTED_CODEX_REVIEW_AUTHORS[0], commit_oid="old-sha")
        thread["comments"]["nodes"].append(
            {
                "id": "lifecycle-comment",
                "fullDatabaseId": "3311706430",
                "body": '<!-- codex-thread-lifecycle:v3 {"state":"defer_to_issue","issue_url":"https://github.example/issues/9","resolved":true,"resolved_at":"2026-05-29T00:00:00Z"} -->',
                "author": {"login": post_review.TRUSTED_CODEX_REVIEW_AUTHORS[0]},
                "commit": {"oid": "old-sha"},
                "originalCommit": {"oid": "old-sha"},
                "path": "src/lib.rs",
                "line": 2,
                "originalLine": 2,
                "url": "https://github.example/lifecycle",
            }
        )

        inventory = post_review.build_thread_lifecycle_inventory([thread], head_sha="head-sha")

        self.assertEqual([], inventory)

    def test_thread_inventory_does_not_skip_unresolved_or_untrusted_lifecycle_marker(self):
        for author, marker_payload in (
            (
                post_review.TRUSTED_CODEX_REVIEW_AUTHORS[0],
                '{"state":"false_positive","resolved":false}',
            ),
            (
                "random-reviewer[bot]",
                '{"state":"false_positive","resolved":true,"resolved_at":"2026-05-29T00:00:00Z"}',
            ),
        ):
            with self.subTest(author=author, marker_payload=marker_payload):
                thread = review_thread(author=post_review.TRUSTED_CODEX_REVIEW_AUTHORS[0], commit_oid="old-sha")
                thread["comments"]["nodes"].append(
                    {
                        "id": "lifecycle-comment",
                        "fullDatabaseId": "3311706430",
                        "body": f"<!-- codex-thread-lifecycle:v3 {marker_payload} -->",
                        "author": {"login": author},
                        "commit": {"oid": "old-sha"},
                        "originalCommit": {"oid": "old-sha"},
                        "path": "src/lib.rs",
                        "line": 2,
                        "originalLine": 2,
                        "url": "https://github.example/lifecycle",
                    }
                )

                inventory = post_review.build_thread_lifecycle_inventory([thread], head_sha="head-sha")

                self.assertEqual(1, len(inventory))

    def test_thread_inventory_groups_comments_by_thread(self):
        thread = review_thread(author=post_review.TRUSTED_CODEX_REVIEW_AUTHORS[0], commit_oid="old-sha")
        thread["comments"]["nodes"].append(
            {
                "id": "comment-node-id-2",
                "fullDatabaseId": "3311706431",
                "body": "\n".join(
                    [
                        post_review.INLINE_MARKER,
                        "<!-- codex-review-id: correctness-2 -->",
                        "second review body",
                    ]
                ),
                "author": {"login": post_review.TRUSTED_CODEX_REVIEW_AUTHORS[0]},
                "commit": {"oid": "old-sha"},
                "originalCommit": {"oid": "old-sha"},
                "path": "src/lib.rs",
                "line": 4,
                "originalLine": 4,
                "url": "https://github.example/review-comment-2",
            }
        )

        inventory = post_review.build_thread_lifecycle_inventory([thread], head_sha="head-sha")

        self.assertEqual(1, len(inventory))
        self.assertEqual("thread-node-id", inventory[0]["thread_id"])
        self.assertEqual([3311706429, 3311706431], [item["comment_id"] for item in inventory[0]["comments"]])

    def test_render_current_inline_persists_root_cause_metadata(self):
        body = post_review.render_current_inline(
            {
                "id": "correctness-1",
                "agent": "correctness",
                "type": "MUST",
                "file": "src/deposit_wallet/http/submit_flow.rs",
                "line": 7,
                "title": "submit state invariant",
                "reason": "state must stay observable",
                "root_cause_key": "deposit-wallet-submit-state",
            },
            {"primary_root_cause_key": "deposit-wallet-submit-state", "reason": "publish representative"},
        )

        self.assertIn("<!-- codex-review-id: correctness-1 -->", body)
        self.assertIn("<!-- codex-root-cause-key: deposit-wallet-submit-state -->", body)
        self.assertIn("<!-- codex-root-cause-area: deposit-wallet -->", body)
        self.assertIn("<!-- codex-root-cause-failure-kind: correctness -->", body)

    def test_render_current_inline_does_not_persist_finding_id_as_root_cause_key(self):
        body = post_review.render_current_inline(
            {
                "id": "correctness-1",
                "agent": "correctness",
                "type": "MUST",
                "file": "src/deposit_wallet/http/submit_flow.rs",
                "line": 7,
                "title": "submit state invariant",
                "reason": "state must stay observable",
                "root_cause_key": "deposit-wallet-submit-state",
            },
            {"primary_root_cause_key": "correctness-1", "reason": "invalid override"},
        )

        self.assertIn("<!-- codex-root-cause-key: deposit-wallet-submit-state -->", body)
        self.assertNotIn("<!-- codex-root-cause-key: correctness-1 -->", body)

    def test_thread_inventory_extracts_root_cause_key_not_finding_id(self):
        body = post_review.render_current_inline(
            {
                "id": "correctness-1",
                "agent": "correctness",
                "type": "MUST",
                "file": "src/deposit_wallet/http/submit_flow.rs",
                "line": 7,
                "title": "submit state invariant",
                "reason": "state must stay observable",
                "root_cause_key": "deposit-wallet-submit-state",
            },
            {"primary_root_cause_key": "deposit-wallet-submit-state", "reason": "publish representative"},
        )
        thread = review_thread(
            author=post_review.TRUSTED_CODEX_REVIEW_AUTHORS[0],
            body=body,
            commit_oid="old-sha",
        )

        inventory = post_review.build_thread_lifecycle_inventory([thread], head_sha="head-sha")

        self.assertEqual("deposit-wallet-submit-state", inventory[0]["root_cause_key"])
        self.assertEqual("codex-root-cause-key", inventory[0]["root_cause_key_source"])
        self.assertEqual("correctness-1", inventory[0]["comments"][0]["marker_key"])

    def test_deferred_issue_key_is_stable_for_same_root_cause_across_finding_ids(self):
        threads = []
        for finding_id in ("correctness-1", "correctness-7"):
            body = post_review.render_current_inline(
                {
                    "id": finding_id,
                    "agent": "correctness",
                    "type": "MUST",
                    "file": "src/deposit_wallet/http/submit_flow.rs",
                    "line": 7,
                    "title": "submit state invariant",
                    "reason": "state must stay observable",
                    "root_cause_key": "deposit-wallet-submit-state",
                },
                {"primary_root_cause_key": "deposit-wallet-submit-state", "reason": "publish representative"},
            )
            thread = review_thread(
                author=post_review.TRUSTED_CODEX_REVIEW_AUTHORS[0],
                body=body,
                commit_oid="old-sha",
            )
            thread["id"] = f"thread-{finding_id}"
            thread["comments"]["nodes"][0]["id"] = f"comment-{finding_id}"
            thread["comments"]["nodes"][0]["fullDatabaseId"] = "1" if finding_id.endswith("1") else "7"
            threads.append(thread)

        inventory = post_review.build_thread_lifecycle_inventory(threads, head_sha="head-sha")

        self.assertEqual(
            post_review.trusted_issue_key_for_thread("repo/name", inventory[0]),
            post_review.trusted_issue_key_for_thread("repo/name", inventory[1]),
        )

    def test_missing_root_cause_marker_forces_needs_human(self):
        thread = review_thread(author=post_review.TRUSTED_CODEX_REVIEW_AUTHORS[0], commit_oid="old-sha")

        inventory = post_review.build_thread_lifecycle_inventory([thread], head_sha="head-sha")

        self.assertEqual("correctness-1", inventory[0]["root_cause_key"])
        self.assertEqual("legacy-codex-review-id", inventory[0]["root_cause_key_source"])
        self.assertEqual("needs_human", inventory[0]["forced_state"])
        self.assertIn("root-cause metadata", inventory[0]["needs_human_hint"])

    def test_thread_inventory_forces_needs_human_when_root_marker_is_finding_id(self):
        body = "\n".join(
            [
                post_review.INLINE_MARKER,
                "<!-- codex-review-id: correctness-1 -->",
                "<!-- codex-root-cause-key: correctness-1 -->",
                "<!-- codex-root-cause-area: deposit-wallet -->",
                "<!-- codex-root-cause-failure-kind: correctness -->",
                "review body",
            ]
        )
        thread = review_thread(
            author=post_review.TRUSTED_CODEX_REVIEW_AUTHORS[0],
            body=body,
            commit_oid="old-sha",
        )

        inventory = post_review.build_thread_lifecycle_inventory([thread], head_sha="head-sha")

        self.assertEqual("correctness-1", inventory[0]["root_cause_key"])
        self.assertEqual("invalid-codex-root-cause-key", inventory[0]["root_cause_key_source"])
        self.assertEqual("needs_human", inventory[0]["forced_state"])
        self.assertIn("invalid root-cause metadata", inventory[0]["needs_human_hint"])

    def test_thread_inventory_forces_needs_human_when_root_marker_is_empty(self):
        body = "\n".join(
            [
                post_review.INLINE_MARKER,
                "<!-- codex-review-id: correctness-1 -->",
                "<!-- codex-root-cause-key: -->",
                "<!-- codex-root-cause-area: deposit-wallet -->",
                "<!-- codex-root-cause-failure-kind: correctness -->",
                "review body",
            ]
        )
        thread = review_thread(
            author=post_review.TRUSTED_CODEX_REVIEW_AUTHORS[0],
            body=body,
            commit_oid="old-sha",
        )

        inventory = post_review.build_thread_lifecycle_inventory([thread], head_sha="head-sha")

        self.assertEqual("needs_human", inventory[0]["forced_state"])
        self.assertIn("root-cause metadata", inventory[0]["needs_human_hint"])

    def test_thread_inventory_forces_needs_human_when_later_comment_missing_root_marker(self):
        valid_body = "\n".join(
            [
                post_review.INLINE_MARKER,
                "<!-- codex-review-id: correctness-1 -->",
                "<!-- codex-root-cause-key: submit-state -->",
                "<!-- codex-root-cause-area: workflow -->",
                "<!-- codex-root-cause-failure-kind: correctness -->",
                "review body",
            ]
        )
        legacy_body = "\n".join(
            [
                post_review.INLINE_MARKER,
                "<!-- codex-review-id: correctness-2 -->",
                "legacy review body",
            ]
        )
        thread = review_thread(
            author=post_review.TRUSTED_CODEX_REVIEW_AUTHORS[0],
            body=valid_body,
            commit_oid="old-sha",
        )
        append_review_comment(thread, body=legacy_body)

        inventory = post_review.build_thread_lifecycle_inventory([thread], head_sha="head-sha")

        self.assertEqual("needs_human", inventory[0]["forced_state"])
        self.assertIn("missing trusted root-cause metadata", inventory[0]["needs_human_hint"])

    def test_thread_inventory_forces_needs_human_when_later_comment_has_finding_id_root_marker(self):
        valid_body = "\n".join(
            [
                post_review.INLINE_MARKER,
                "<!-- codex-review-id: correctness-1 -->",
                "<!-- codex-root-cause-key: submit-state -->",
                "<!-- codex-root-cause-area: workflow -->",
                "<!-- codex-root-cause-failure-kind: correctness -->",
                "review body",
            ]
        )
        invalid_body = "\n".join(
            [
                post_review.INLINE_MARKER,
                "<!-- codex-review-id: correctness-2 -->",
                "<!-- codex-root-cause-key: correctness-2 -->",
                "<!-- codex-root-cause-area: workflow -->",
                "<!-- codex-root-cause-failure-kind: correctness -->",
                "legacy review body",
            ]
        )
        thread = review_thread(
            author=post_review.TRUSTED_CODEX_REVIEW_AUTHORS[0],
            body=valid_body,
            commit_oid="old-sha",
        )
        append_review_comment(thread, body=invalid_body)

        inventory = post_review.build_thread_lifecycle_inventory([thread], head_sha="head-sha")

        self.assertEqual("needs_human", inventory[0]["forced_state"])
        self.assertIn("invalid root-cause metadata", inventory[0]["needs_human_hint"])

    def test_thread_inventory_forces_needs_human_when_thread_has_multiple_root_cause_keys(self):
        first_body = "\n".join(
            [
                post_review.INLINE_MARKER,
                "<!-- codex-review-id: correctness-1 -->",
                "<!-- codex-root-cause-key: submit-state -->",
                "<!-- codex-root-cause-area: workflow -->",
                "<!-- codex-root-cause-failure-kind: correctness -->",
                "review body",
            ]
        )
        second_body = "\n".join(
            [
                post_review.INLINE_MARKER,
                "<!-- codex-review-id: correctness-2 -->",
                "<!-- codex-root-cause-key: nonce-state -->",
                "<!-- codex-root-cause-area: workflow -->",
                "<!-- codex-root-cause-failure-kind: correctness -->",
                "review body",
            ]
        )
        thread = review_thread(
            author=post_review.TRUSTED_CODEX_REVIEW_AUTHORS[0],
            body=first_body,
            commit_oid="old-sha",
        )
        append_review_comment(thread, body=second_body)

        inventory = post_review.build_thread_lifecycle_inventory([thread], head_sha="head-sha")

        self.assertEqual("needs_human", inventory[0]["forced_state"])
        self.assertIn("conflicting root-cause metadata", inventory[0]["needs_human_hint"])

    def test_thread_inventory_allows_handoff_when_all_comments_have_same_valid_root_metadata(self):
        first_body = "\n".join(
            [
                post_review.INLINE_MARKER,
                "<!-- codex-review-id: correctness-1 -->",
                "<!-- codex-root-cause-key: submit-state -->",
                "<!-- codex-root-cause-area: workflow -->",
                "<!-- codex-root-cause-failure-kind: correctness -->",
                "review body",
            ]
        )
        second_body = "\n".join(
            [
                post_review.INLINE_MARKER,
                "<!-- codex-review-id: correctness-2 -->",
                "<!-- codex-root-cause-key: submit-state -->",
                "<!-- codex-root-cause-area: workflow -->",
                "<!-- codex-root-cause-failure-kind: correctness -->",
                "review body",
            ]
        )
        thread = review_thread(
            author=post_review.TRUSTED_CODEX_REVIEW_AUTHORS[0],
            body=first_body,
            commit_oid="old-sha",
        )
        append_review_comment(thread, body=second_body)

        inventory = post_review.build_thread_lifecycle_inventory([thread], head_sha="head-sha")

        self.assertEqual("submit-state", inventory[0]["root_cause_key"])
        self.assertEqual("codex-root-cause-key", inventory[0]["root_cause_key_source"])
        self.assertNotIn("forced_state", inventory[0])

    def test_batch_planner_uses_thread_batches_not_three_comment_batches(self):
        threads = []
        for index in range(13):
            thread = review_thread(
                author=post_review.TRUSTED_CODEX_REVIEW_AUTHORS[0],
                commit_oid="old-sha",
                line=index + 1,
            )
            thread["id"] = f"thread-{index}"
            thread["comments"]["nodes"][0]["id"] = f"comment-{index}"
            thread["comments"]["nodes"][0]["fullDatabaseId"] = str(1000 + index)
            threads.append(thread)

        inventory = post_review.build_thread_lifecycle_inventory(threads, head_sha="head-sha")
        batches = post_review.plan_thread_lifecycle_batches(inventory)

        self.assertEqual([12, 1], [len(batch["threads"]) for batch in batches])
        self.assertEqual("codex.thread_lifecycle_batch.v3", batches[0]["schema_version"])

    def test_thread_inventory_forces_needs_human_for_large_thread_connection(self):
        thread = review_thread(author=post_review.TRUSTED_CODEX_REVIEW_AUTHORS[0], commit_oid="old-sha")
        thread["comments"]["totalCount"] = 51

        inventory = post_review.build_thread_lifecycle_inventory([thread], head_sha="head-sha")

        self.assertEqual("needs_human", inventory[0]["forced_state"])
        self.assertIn("more than 50 comments", inventory[0]["needs_human_hint"])

    def test_lifecycle_result_rejects_defer_without_issue_key(self):
        payload = {
            "schema_version": "codex.thread_lifecycle_result.v3",
            "threads": [
                {
                    "thread_id": "thread-a",
                    "state": "defer_to_issue",
                    "reason": "PR scope 밖이므로 이슈로 이관",
                    "evidence": "현재 PR 변경 범위와 직접 관련 없음",
                }
            ],
        }

        with self.assertRaises(SystemExit):
            post_review.normalize_lifecycle_outputs([payload], {"thread-a"})

    def test_trusted_deferred_issue_key_groups_by_root_cause_not_thread_id(self):
        base = {
            "root_cause_key": "deposit-wallet-submit-state",
            "area": "deposit-wallet",
            "file": "src/deposit_wallet/http/submit_flow.rs",
        }
        first = {**base, "thread_id": "thread-a"}
        second = {**base, "thread_id": "thread-b", "file": "src/deposit_wallet/http/status.rs"}

        self.assertEqual(
            post_review.trusted_issue_key_for_thread("repo/name", first),
            post_review.trusted_issue_key_for_thread("repo/name", second),
        )

    def test_trusted_deferred_issue_key_differs_by_failure_kind(self):
        base = {
            "root_cause_key": "deposit-wallet-submit-state",
            "area": "deposit-wallet",
            "file": "src/deposit_wallet/http/submit_flow.rs",
            "thread_id": "thread-a",
        }
        correctness = {**base, "root_cause_failure_kind": "correctness"}
        security = {**base, "root_cause_failure_kind": "security"}

        self.assertNotEqual(
            post_review.trusted_issue_key_for_thread("repo/name", correctness),
            post_review.trusted_issue_key_for_thread("repo/name", security),
        )

    def test_root_cause_key_uses_full_marker_id_not_axis_collapse(self):
        item = {"marker_key": "correctness-17", "file": "src/lib.rs"}

        self.assertEqual("correctness-17", post_review.root_cause_key_for_comment(item))

    def test_apply_lifecycle_replies_before_resolving_deferred_thread(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            batches = root / "batches"
            results = root / "results"
            batches.mkdir()
            results.mkdir()
            (batches / "resolve-batch-0.json").write_text(
                json.dumps(
                    {
                        "schema_version": "codex.thread_lifecycle_batch.v3",
                        "threads": [
                            {
                                "thread_id": "thread-a",
                                "root_cause_key": "deposit-wallet-submit-state",
                                "area": "deposit-wallet",
                                "file": "src/deposit_wallet/http/submit_flow.rs",
                                "comments": [
                                    {
                                        "comment_id": 1,
                                        "thread_id": "thread-a",
                                        "file": "src/deposit_wallet/http/submit_flow.rs",
                                        "line": 7,
                                        "url": "https://github.example/comment",
                                        "body_excerpt": "state invariant",
                                    }
                                ],
                            }
                        ],
                    },
                    ensure_ascii=False,
                ),
                encoding="utf-8",
            )
            (results / "lifecycle-0.json").write_text(
                json.dumps(
                    {
                        "schema_version": "codex.thread_lifecycle_result.v3",
                        "threads": [
                            {
                                "thread_id": "thread-a",
                                "state": "defer_to_issue",
                                "reason": "별도 PR에서 다룰 root cause",
                                "evidence": "현재 PR scope 밖",
                                "issue": {
                                    "key": "abc123",
                                    "title": "[codex][deferred][deposit-wallet] submit state invariant",
                                    "body": "## Summary\nsubmit state invariant",
                                    "labels": ["codex/deferred", "area/deposit-wallet"],
                                },
                            }
                        ],
                    },
                    ensure_ascii=False,
                ),
                encoding="utf-8",
            )
            calls = []

            def fake_issue(*, repo, request):
                calls.append(("issue", request["key"]))
                return {"html_url": "https://github.example/issues/9", "state": "open", "number": 9}

            def fake_reply(thread_id, body):
                calls.append(("reply", thread_id, body))

            def fake_resolve(thread_id):
                calls.append(("resolve", thread_id))

            def fake_reply(thread_id, body):
                calls.append(("reply", thread_id, body))

            def fake_upsert(**kwargs):
                calls.append(("upsert", kwargs["marker"], kwargs["body"]))
                return "updated"

            env = {"GITHUB_REPOSITORY": "repo/name", "PR_NUMBER": "21"}
            args = argparse.Namespace(batches=str(batches), results=str(results))
            with patch.dict(os.environ, env, clear=False), patch.object(
                post_review, "create_or_update_deferred_issue", side_effect=fake_issue
            ), patch.object(
                post_review, "reply_to_review_thread", side_effect=fake_reply
            ), patch.object(
                post_review, "resolve_thread", side_effect=fake_resolve
            ), patch.object(
                post_review, "apply_write_token_preflight", return_value=None
            ), patch.object(post_review, "upsert_marker_comment", side_effect=fake_upsert):
                post_review.command_apply_resolutions(args)

        self.assertEqual("issue", calls[0][0])
        self.assertNotEqual("abc123", calls[0][1])
        self.assertEqual("reply", calls[1][0])
        self.assertIn("https://github.example/issues/9", calls[1][2])
        self.assertIn('"resolved": false', calls[1][2])
        self.assertEqual(("resolve", "thread-a"), calls[2])

    def test_duplicate_of_issue_requires_codex_deferred_issue(self):
        calls = []

        def fake_issue(repo, issue_url):
            self.assertEqual("https://github.com/repo/name/issues/9", issue_url)
            return {"state": "open", "number": 9, "body": "plain issue", "labels": []}

        def fake_upsert(**kwargs):
            calls.append(("upsert", kwargs["marker"], kwargs["body"]))
            return "updated"

        with patch.object(
            post_review, "get_same_repo_issue_by_url", side_effect=fake_issue
        ), patch.object(
            post_review, "reply_to_review_thread"
        ) as reply, patch.object(
            post_review, "resolve_thread"
        ) as resolve, patch.object(post_review, "upsert_marker_comment", side_effect=fake_upsert):
            post_review.apply_lifecycle_resolutions(
                repo="repo/name",
                pr_number="21",
                threads={"thread-a": {"thread_id": "thread-a", "file": "src/lib.rs"}},
                decisions={
                    "thread-a": {
                        "thread_id": "thread-a",
                        "state": "duplicate_of_issue",
                        "reason": "이미 이슈가 있음",
                        "evidence": "모델 판단",
                        "issue_url": "https://github.com/repo/name/issues/9",
                    }
                },
            )

        reply.assert_not_called()
        resolve.assert_not_called()
        self.assertIn("must point to an open Codex deferred issue", calls[0][2])

    def test_duplicate_of_issue_rejects_pull_request_issue_object(self):
        issue = {
            "state": "open",
            "pull_request": {"url": "https://api.github.com/repos/repo/name/pulls/9"},
            "labels": [{"name": "codex/deferred"}],
            "body": "<!-- codex-issue-key: key -->",
        }

        self.assertFalse(post_review.is_codex_deferred_issue(issue))

    def test_duplicate_of_issue_accepts_codex_deferred_label_or_marker(self):
        self.assertTrue(
            post_review.is_codex_deferred_issue(
                {"state": "open", "labels": [{"name": "codex/deferred"}], "body": ""}
            )
        )
        self.assertTrue(
            post_review.is_codex_deferred_issue(
                {"state": "open", "labels": [], "body": "<!-- codex-issue-key: abc -->"}
            )
        )

    def test_deferred_issue_update_preserves_existing_source_threads(self):
        key = "submit-state-abc"
        existing_body = post_review.issue_body_with_marker(
            {
                "key": key,
                "body": "existing",
                "root_cause": {"key": "submit-state"},
                "source_threads": ["thread-a"],
            }
        )
        calls = []

        def fake_find(repo, issue_key):
            self.assertEqual("repo/name", repo)
            self.assertEqual(key, issue_key)
            return {"number": 9, "state": "open", "body": existing_body}

        def fake_api(path, *, method="GET", payload=None):
            calls.append((path, method, payload))
            return {"number": 9, "state": "open", "body": payload["body"]}

        with patch.object(post_review, "find_issue_by_key", side_effect=fake_find), patch.object(
            post_review, "github_api", side_effect=fake_api
        ):
            post_review.create_or_update_deferred_issue(
                repo="repo/name",
                request={
                    "key": key,
                    "title": "submit state",
                    "body": "updated",
                    "root_cause": {"key": "submit-state"},
                    "source_threads": ["thread-b"],
                    "labels": ["codex/deferred"],
                },
            )

        updated_body = calls[0][2]["body"]
        self.assertIn('"thread-a"', updated_body)
        self.assertIn('"thread-b"', updated_body)

    def test_apply_lifecycle_forced_needs_human_overrides_model_terminal_state(self):
        calls = []

        def fake_reply(thread_id, body):
            calls.append(("reply", thread_id, body))

        def fake_resolve(thread_id):
            calls.append(("resolve", thread_id))

        def fake_upsert(**kwargs):
            calls.append(("upsert", kwargs["marker"], kwargs["body"]))
            return "updated"

        with patch.object(
            post_review, "reply_to_review_thread", side_effect=fake_reply
        ), patch.object(
            post_review, "resolve_thread", side_effect=fake_resolve
        ), patch.object(post_review, "upsert_marker_comment", side_effect=fake_upsert):
            post_review.apply_lifecycle_resolutions(
                repo="repo/name",
                pr_number="21",
                threads={
                    "thread-a": {
                        "thread_id": "thread-a",
                        "file": "src/lib.rs",
                        "forced_state": "needs_human",
                        "needs_human_hint": "thread has more than 50 comments",
                    }
                },
                decisions={
                    "thread-a": {
                        "thread_id": "thread-a",
                        "state": "false_positive",
                        "reason": "모델은 terminal이라고 판단",
                        "evidence": "하지만 collector context가 불완전",
                    }
                },
            )

        self.assertNotIn(("resolve", "thread-a"), calls)
        self.assertFalse(any(call[0] == "reply" for call in calls))
        self.assertIn("more than 50 comments", calls[0][2])


class ReviewContextTests(unittest.TestCase):
    def test_build_review_context_includes_authoritative_and_advisory_sections(self):
        pr = {"title": "Deposit wallet state", "body": "Current PR state is authoritative."}
        issue_comments = [
            {
                "id": 1,
                "body": post_review.DESIGN_MARKER + "\nold design",
                "updated_at": "2026-05-27T00:00:00Z",
                "user": {"login": "codex-reviewer-for-dongwonttuna"},
            },
            {
                "id": 2,
                "body": post_review.REVIEW_SUMMARY_MARKER + "\nsticky review summary",
                "updated_at": "2026-05-27T02:00:00Z",
                "user": {"login": "codex-reviewer-for-dongwonttuna"},
            },
            {
                "id": 3,
                "body": post_review.RESOLVE_MARKER + "\nsticky resolve summary",
                "updated_at": "2026-05-27T03:00:00Z",
                "user": {"login": "codex-reviewer-for-dongwonttuna"},
            }
        ]
        reviews = [
            {
                "body": post_review.REVIEW_MARKER + "\nreview summary",
                "submitted_at": "2026-05-27T01:00:00Z",
                "state": "CHANGES_REQUESTED",
                "user": {"login": "codex-reviewer-for-dongwonttuna"},
            }
        ]
        threads = [
            review_thread(
                author=post_review.TRUSTED_CODEX_REVIEW_AUTHORS[0],
                body="\n".join(
                    [
                        post_review.INLINE_MARKER,
                        "<!-- codex-review-id: domain-1 -->",
                        "nonce invariant is not preserved",
                    ]
                ),
            )
        ]

        context = post_review.build_review_context_markdown(
            repo="DongwonTTuna-Labs/rs-builder-relayer-client",
            pr_number="21",
            head_sha="head-sha",
            pr=pr,
            issue_comments=issue_comments,
            reviews=reviews,
            threads=threads,
        )

        self.assertIn("Current PR State (authoritative)", context)
        self.assertIn("Current PR state is authoritative.", context)
        self.assertIn("Latest Sticky Design Plan (advisory)", context)
        self.assertIn("old design", context)
        self.assertIn("Recent Codex Review And Resolve Summaries", context)
        self.assertIn("sticky review summary", context)
        self.assertIn("sticky resolve summary", context)
        self.assertIn("review summary", context)
        self.assertIn("Current Unresolved Inline Threads", context)
        self.assertIn("domain", context)
        self.assertIn("현재 코드", context)

    def test_build_review_context_truncates_large_sections(self):
        old_limit = post_review.REVIEW_CONTEXT_MAX_CHARS
        try:
            post_review.REVIEW_CONTEXT_MAX_CHARS = 2000
            context = post_review.build_review_context_markdown(
                repo="repo/name",
                pr_number="1",
                head_sha="head",
                pr={"title": "title", "body": "x" * 5000},
                issue_comments=[],
                reviews=[],
                threads=[],
            )
        finally:
            post_review.REVIEW_CONTEXT_MAX_CHARS = old_limit

        self.assertLessEqual(len(context), 2015)
        self.assertIn("...[truncated]", context)


class StickySummaryTests(unittest.TestCase):
    def test_upsert_marker_comment_updates_latest_match(self):
        calls = []

        def list_comments(path):
            self.assertEqual(path, "/repos/repo/name/issues/21/comments?per_page=100")
            return [
                {"id": 1, "body": post_review.REVIEW_SUMMARY_MARKER, "updated_at": "2026-05-26T00:00:00Z"},
                {"id": 2, "body": post_review.REVIEW_SUMMARY_MARKER, "updated_at": "2026-05-27T00:00:00Z"},
            ]

        def api(path, *, method="GET", payload=None):
            calls.append((path, method, payload))

        result = post_review.upsert_marker_comment(
            repo="repo/name",
            pr_number="21",
            marker=post_review.REVIEW_SUMMARY_MARKER,
            body=post_review.REVIEW_SUMMARY_MARKER + "\nnew",
            list_comments=list_comments,
            api=api,
        )

        self.assertEqual("updated", result)
        self.assertEqual(
            ("/repos/repo/name/issues/comments/2", "PATCH", {"body": post_review.REVIEW_SUMMARY_MARKER + "\nnew"}),
            calls[0],
        )

    def test_render_current_body_uses_sticky_review_summary_marker(self):
        body = post_review.render_current_body(
            event="COMMENT",
            allowed=[],
            denied_count=0,
            unplaced=[],
            decisions={"judgment": {}, "merge_notes": []},
        )

        self.assertIn(post_review.REVIEW_SUMMARY_MARKER, body)
        self.assertNotIn(post_review.REVIEW_MARKER, body)

    def test_current_review_inline_redacts_secret_like_values(self):
        token = "github_pat_" + ("A" * 24)
        body = post_review.render_current_inline(
            {
                "id": "correctness-1",
                "type": "MUST",
                "agent": "correctness",
                "title": f"leaks {token}",
                "reason": f"reason includes {token}",
                "root_cause_key": "secret-redaction",
            },
            {"reason": f"decision includes {token}"},
        )

        self.assertIn("[redacted]", body)
        self.assertNotIn(token, body)

    def test_current_review_body_redacts_secret_like_values(self):
        token = "sk-proj-" + ("A" * 24)
        body = post_review.render_current_body(
            event="COMMENT",
            allowed=[],
            denied_count=0,
            unplaced=[
                (
                    {
                        "id": "correctness-1",
                        "type": "MUST",
                        "agent": "correctness",
                        "file": "src/lib.rs",
                        "line": 7,
                        "title": f"title {token}",
                        "reason": f"reason {token}",
                    },
                    {"reason": f"decision {token}"},
                )
            ],
            decisions={
                "judgment": {"status": "NEEDS_WORK", "headline": f"headline {token}"},
                "merge_notes": [{"primary_id": "correctness-1", "merged_ids": [], "reason": f"merge {token}"}],
            },
        )

        self.assertIn("[redacted]", body)
        self.assertNotIn(token, body)

    def test_post_current_without_inline_comments_only_upserts_sticky_summary(self):
        calls = []

        def fake_upsert(**kwargs):
            calls.append(("upsert", kwargs["marker"], kwargs["body"]))
            return "updated"

        env = {
            "GITHUB_REPOSITORY": "repo/name",
            "PR_NUMBER": "21",
            "HEAD_SHA": "head-sha",
        }
        args = argparse.Namespace(artifacts="artifacts", decisions="decisions.json")
        with patch.dict(os.environ, env, clear=False), patch.object(
            post_review, "load_current_findings", return_value=[]
        ), patch.object(
            post_review,
            "load_decisions",
            return_value={"by_id": {}, "judgment": {"status": "LGTM"}, "merge_notes": []},
        ), patch.object(
            post_review, "build_changed_line_map", return_value={}
        ), patch.object(
            post_review, "upsert_marker_comment", side_effect=fake_upsert
        ), patch.object(
            post_review, "github_api"
        ) as api:
            post_review.command_post_current(args)

        api.assert_not_called()
        self.assertEqual(1, len(calls))
        self.assertEqual(post_review.REVIEW_SUMMARY_MARKER, calls[0][1])
        self.assertIn("Codex 리뷰가 완료되었습니다", calls[0][2])

    def test_apply_resolutions_resolves_threads_and_upserts_summary(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            batches = root / "batches"
            results = root / "results"
            batches.mkdir()
            results.mkdir()
            (batches / "resolve-batch-0.json").write_text(
                json.dumps(
                    {
                        "comments": [
                            {
                                "comment_id": 1,
                                "thread_id": "thread-a",
                                "file": "src/lib.rs",
                                "line": 7,
                                "url": "https://github.example/comment",
                            },
                            {
                                "comment_id": 2,
                                "thread_id": "thread-b",
                                "file": "src/main.rs",
                                "line": 9,
                            },
                        ]
                    },
                    ensure_ascii=False,
                ),
                encoding="utf-8",
            )
            (results / "resolutions-0.json").write_text(
                json.dumps(
                    {
                        "resolutions": [
                            {"comment_id": 1, "resolved": True, "reason": "현재 head에서 해결됨"},
                            {"comment_id": 2, "resolved": False, "reason": "아직 실제 결함"},
                        ]
                    },
                    ensure_ascii=False,
                ),
                encoding="utf-8",
            )
            calls = []

            def fake_resolve(thread_id):
                calls.append(("resolve", thread_id))

            def fake_reply(thread_id, body):
                calls.append(("reply", thread_id, body))

            def fake_upsert(**kwargs):
                calls.append(("upsert", kwargs["marker"], kwargs["body"]))
                return "updated"

            env = {"GITHUB_REPOSITORY": "repo/name", "PR_NUMBER": "21"}
            args = argparse.Namespace(batches=str(batches), results=str(results))
            with patch.dict(os.environ, env, clear=False), patch.object(
                post_review, "resolve_thread", side_effect=fake_resolve
            ), patch.object(
                post_review, "reply_to_review_thread", side_effect=fake_reply
            ), patch.object(
                post_review, "apply_write_token_preflight", return_value=None
            ), patch.object(post_review, "upsert_marker_comment", side_effect=fake_upsert):
                post_review.command_apply_resolutions(args)

        self.assertIn("codex-thread-lifecycle:v3", calls[0][2])
        self.assertIn(("resolve", "thread-a"), calls)
        upserts = [call for call in calls if call[0] == "upsert"]
        self.assertEqual(1, len(upserts))
        self.assertEqual(post_review.RESOLVE_MARKER, upserts[0][1])
        self.assertIn("아직 미해결", upserts[0][2])

    def test_apply_resolutions_preflight_failure_only_posts_summary(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            batches = root / "batches"
            results = root / "results"
            batches.mkdir()
            results.mkdir()
            calls = []

            def fake_upsert(**kwargs):
                calls.append(("upsert", kwargs["marker"], kwargs["body"]))
                return "updated"

            env = {"GITHUB_REPOSITORY": "repo/name", "PR_NUMBER": "21"}
            args = argparse.Namespace(batches=str(batches), results=str(results))
            with patch.dict(os.environ, env, clear=False), patch.object(
                post_review, "apply_write_token_preflight", return_value="viewerPermission=READ"
            ), patch.object(
                post_review, "resolve_thread"
            ) as resolve, patch.object(
                post_review, "reply_to_review_thread"
            ) as reply, patch.object(
                post_review, "upsert_marker_comment", side_effect=fake_upsert
            ):
                post_review.command_apply_resolutions(args)

        resolve.assert_not_called()
        reply.assert_not_called()
        self.assertEqual(1, len(calls))
        self.assertEqual(post_review.RESOLVE_MARKER, calls[0][1])
        self.assertIn("token preflight 실패", calls[0][2])


class BoundedAutofixTests(unittest.TestCase):
    def finding(self, finding_id, **overrides):
        item = {
            "id": finding_id,
            "agent": "correctness",
            "type": "MUST",
            "file": "src/lib.rs",
            "line": 7,
            "title": "counter edge case",
            "reason": "The current branch misses a local edge case.",
            "rule_ref": None,
            "cross_cutting": False,
            "root_cause_key": "counter-edge",
            "scope": "current_pr",
            "public_api_risk": False,
            "autofix_eligible_hint": True,
        }
        item.update(overrides)
        return item

    def test_autofix_manifest_selects_one_safe_representative(self):
        findings = [
            self.finding("correctness-1"),
            self.finding("correctness-2", line=9),
            self.finding(
                "correctness-3",
                root_cause_key="public-api",
                public_api_risk=True,
                title="public API shape changes",
            ),
        ]
        decisions = {
            "by_id": {
                "correctness-1": {"action": "publish_and_fix_now", "reason": "safe local fix"},
                "correctness-2": {"action": "publish_and_fix_now", "reason": "duplicate root"},
                "correctness-3": {"action": "publish_and_fix_now", "reason": "needs guard"},
            },
            "judgment": {},
            "merge_notes": [],
        }

        manifest = post_review.build_autofix_manifest(findings, decisions)

        self.assertEqual(post_review.AUTOFIX_MANIFEST_SCHEMA, manifest["schema_version"])
        self.assertEqual(["correctness-1"], [item["id"] for item in manifest["eligible"]])
        self.assertEqual(["correctness-2", "correctness-3"], [item["id"] for item in manifest["blocked"]])

    def test_validate_autofix_patch_blocks_public_api_surface(self):
        manifest = {
            "schema_version": post_review.AUTOFIX_MANIFEST_SCHEMA,
            "eligible": [{"id": "correctness-1", "file": "src/lib.rs"}],
        }
        patch = """diff --git a/src/lib.rs b/src/lib.rs
index 0000000..1111111 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,2 +1,3 @@
+pub fn new_exported_api() {}
 fn existing() {}
"""

        with self.assertRaises(SystemExit):
            post_review.validate_autofix_patch_text(patch, manifest)

    def test_validate_autofix_patch_blocks_file_outside_manifest(self):
        manifest = {
            "schema_version": post_review.AUTOFIX_MANIFEST_SCHEMA,
            "eligible": [{"id": "correctness-1", "file": "src/lib.rs"}],
        }
        patch = """diff --git a/src/other.rs b/src/other.rs
index 0000000..1111111 100644
--- a/src/other.rs
+++ b/src/other.rs
@@ -1,2 +1,2 @@
-fn existing() {}
+fn changed() {}
"""

        with self.assertRaises(SystemExit):
            post_review.validate_autofix_patch_text(patch, manifest)

    def test_validate_autofix_patch_blocks_binary_patch(self):
        manifest = {
            "schema_version": post_review.AUTOFIX_MANIFEST_SCHEMA,
            "eligible": [{"id": "correctness-1", "file": "src/lib.rs"}],
        }
        patch = """diff --git a/src/lib.rs b/src/lib.rs
index 0000000..1111111 100644
GIT binary patch
literal 0
"""

        with self.assertRaises(SystemExit):
            post_review.validate_autofix_patch_text(patch, manifest)

    def test_validate_autofix_patch_blocks_secret_like_material(self):
        manifest = {
            "schema_version": post_review.AUTOFIX_MANIFEST_SCHEMA,
            "eligible": [{"id": "correctness-1", "file": "src/lib.rs"}],
        }
        token = "github_pat_" + ("A" * 24)
        patch = f"""diff --git a/src/lib.rs b/src/lib.rs
index 0000000..1111111 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,2 +1,2 @@
-fn existing() {{}}
+fn existing() {{ let _token = "{token}"; }}
"""

        with self.assertRaises(SystemExit):
            post_review.validate_autofix_patch_text(patch, manifest)

    def test_validate_autofix_patch_blocks_secret_like_context_line(self):
        manifest = {
            "schema_version": post_review.AUTOFIX_MANIFEST_SCHEMA,
            "eligible": [{"id": "correctness-1", "file": "src/lib.rs"}],
        }
        token = "github_pat_" + ("A" * 24)
        patch = f"""diff --git a/src/lib.rs b/src/lib.rs
index 0000000..1111111 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,3 +1,3 @@
 fn existing() {{ let _token = "{token}"; }}
-fn old_name() {{}}
+fn new_name() {{}}
 fn tail() {{}}
"""

        with self.assertRaises(SystemExit):
            post_review.validate_autofix_patch_text(patch, manifest)

    def test_extract_issue_source_threads_reads_machine_readable_block_only(self):
        body = """Intro with model-authored JSON:

```json
{"source_threads": ["wrong-thread"]}
```

## Machine-readable
```json
{"source_threads": ["thread-a", "thread-b"]}
```
"""

        self.assertEqual(["thread-a", "thread-b"], post_review.extract_issue_source_threads(body))

    def test_extract_issue_source_threads_uses_last_machine_readable_block(self):
        body = """Model-authored section:

## Machine-readable
```json
{"source_threads": ["wrong-thread"]}
```

Trusted appended section:

## Machine-readable
```json
{"source_threads": ["thread-a", "thread-b"]}
```
"""

        self.assertEqual(["thread-a", "thread-b"], post_review.extract_issue_source_threads(body))


class DesignPlanTests(unittest.TestCase):
    def sample_plan(self):
        return {
            "version": 1,
            "summary": "전체 invariant 중심으로 수정한다.",
            "root_cause": "댓글 단위 대응으로 상태 계약이 분산됐다.",
            "invariants": ["PR body가 현재 spec이다."],
            "retired_approaches": ["코멘트 하나마다 별도 상태를 추가하지 않는다."],
            "intended_architecture": ["상태 전이를 한 곳에서 관리한다."],
            "edit_sequence": ["상태 타입을 먼저 정리한다."],
            "tests": ["상태 전이 회귀 테스트를 추가한다."],
            "acceptance_criteria": ["MUST finding이 같은 root cause로 재발하지 않는다."],
            "open_questions": [],
        }

    def test_render_design_plan_body_contains_marker_and_machine_json(self):
        body = post_review.render_design_plan_body(self.sample_plan())

        self.assertIn(post_review.DESIGN_MARKER, body)
        self.assertIn("Machine Readable JSON", body)
        self.assertIn('"version": 1', body)
        self.assertIn("전체 invariant 중심", body)

    def test_upsert_design_comment_creates_when_missing(self):
        calls = []

        def list_comments(path):
            self.assertEqual(path, "/repos/repo/name/issues/21/comments?per_page=100")
            return []

        def api(path, *, method="GET", payload=None):
            calls.append((path, method, payload))

        result = post_review.upsert_design_comment(
            repo="repo/name",
            pr_number="21",
            body=post_review.DESIGN_MARKER + "\nbody",
            list_comments=list_comments,
            api=api,
        )

        self.assertEqual("created", result)
        self.assertEqual(("/repos/repo/name/issues/21/comments", "POST", {"body": post_review.DESIGN_MARKER + "\nbody"}), calls[0])

    def test_upsert_design_comment_updates_latest_marker_comment(self):
        calls = []

        def list_comments(path):
            return [
                {"id": 1, "body": post_review.DESIGN_MARKER, "updated_at": "2026-05-26T00:00:00Z"},
                {"id": 2, "body": post_review.DESIGN_MARKER, "updated_at": "2026-05-27T00:00:00Z"},
            ]

        def api(path, *, method="GET", payload=None):
            calls.append((path, method, payload))

        result = post_review.upsert_design_comment(
            repo="repo/name",
            pr_number="21",
            body=post_review.DESIGN_MARKER + "\nnew",
            list_comments=list_comments,
            api=api,
        )

        self.assertEqual("updated", result)
        self.assertEqual(("/repos/repo/name/issues/comments/2", "PATCH", {"body": post_review.DESIGN_MARKER + "\nnew"}), calls[0])


class DesignNeedTests(unittest.TestCase):
    def test_needs_design_for_needs_work_judgment(self):
        needs_design, blocking_count = post_review.should_run_design(
            [],
            {"by_id": {}, "judgment": {"status": "NEEDS_WORK"}, "merge_notes": []},
        )

        self.assertTrue(needs_design)
        self.assertEqual(0, blocking_count)

    def test_needs_design_for_publish_and_fix_now_finding(self):
        findings = [
            {
                "id": "correctness-1",
                "agent": "correctness",
                "type": "MUST",
                "rule_ref": None,
            }
        ]
        decisions = {
            "by_id": {"correctness-1": {"action": "publish_and_fix_now", "reason": "real blocker"}},
            "judgment": {"status": "LGTM"},
            "merge_notes": [],
        }

        needs_design, blocking_count = post_review.should_run_design(findings, decisions)

        self.assertTrue(needs_design)
        self.assertEqual(1, blocking_count)

    def test_skips_design_for_lgtm_without_blockers(self):
        findings = [
            {
                "id": "performance-1",
                "agent": "performance",
                "type": "NITS",
                "rule_ref": None,
            }
        ]
        decisions = {
            "by_id": {"performance-1": {"action": "deny_false_positive", "reason": "minor"}},
            "judgment": {"status": "LGTM"},
            "merge_notes": [],
        }

        needs_design, blocking_count = post_review.should_run_design(findings, decisions)

        self.assertFalse(needs_design)
        self.assertEqual(0, blocking_count)


if __name__ == "__main__":
    unittest.main()
