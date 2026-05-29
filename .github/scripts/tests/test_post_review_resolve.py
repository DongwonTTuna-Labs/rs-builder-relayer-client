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

            def fake_upsert(**kwargs):
                calls.append(("upsert", kwargs["marker"], kwargs["body"]))
                return "updated"

            env = {"GITHUB_REPOSITORY": "repo/name", "PR_NUMBER": "21"}
            args = argparse.Namespace(batches=str(batches), results=str(results))
            with patch.dict(os.environ, env, clear=False), patch.object(
                post_review, "resolve_thread", side_effect=fake_resolve
            ), patch.object(post_review, "upsert_marker_comment", side_effect=fake_upsert):
                post_review.command_apply_resolutions(args)

        self.assertIn(("resolve", "thread-a"), calls)
        upserts = [call for call in calls if call[0] == "upsert"]
        self.assertEqual(1, len(upserts))
        self.assertEqual(post_review.RESOLVE_MARKER, upserts[0][1])
        self.assertIn("아직 미해결", upserts[0][2])


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

    def test_needs_design_for_allowed_must_finding(self):
        findings = [
            {
                "id": "correctness-1",
                "agent": "correctness",
                "type": "MUST",
                "rule_ref": None,
            }
        ]
        decisions = {
            "by_id": {"correctness-1": {"allow": True, "reason": "real blocker"}},
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
            "by_id": {"performance-1": {"allow": True, "reason": "minor"}},
            "judgment": {"status": "LGTM"},
            "merge_notes": [],
        }

        needs_design, blocking_count = post_review.should_run_design(findings, decisions)

        self.assertFalse(needs_design)
        self.assertEqual(0, blocking_count)


if __name__ == "__main__":
    unittest.main()
