import importlib.util
import unittest
from pathlib import Path


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


if __name__ == "__main__":
    unittest.main()
