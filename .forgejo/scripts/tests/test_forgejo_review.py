import json
import os
import tempfile
import unittest
from pathlib import Path

import sys

SCRIPT_DIR = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPT_DIR))

from forgejo_api import redact_secret, split_repo  # noqa: E402
from post_review_comments import (  # noqa: E402
    extract_marker,
    finding_key,
    marker_for,
    render_inline_body,
)
from prepare_pr_context import parse_changed_right_lines  # noqa: E402
from resolve_pr_metadata import authorize, requested_pr_number  # noqa: E402


class ForgejoApiTests(unittest.TestCase):
    def test_redaction_removes_token_from_errors(self) -> None:
        self.assertEqual(
            redact_secret("token abc123 leaked", "abc123"),
            "token <redacted> leaked",
        )

    def test_split_repo_rejects_invalid_shape(self) -> None:
        self.assertEqual(split_repo("DongwonTTuna-Labs/demo"), ("DongwonTTuna-Labs", "demo"))
        with self.assertRaises(SystemExit):
            split_repo("demo")


class DiffParserTests(unittest.TestCase):
    def test_changed_right_lines_from_unified_diff(self) -> None:
        diff = """diff --git a/app.py b/app.py
--- a/app.py
+++ b/app.py
@@ -1,3 +1,4 @@
 context
-old
+new
+added
 same
"""
        self.assertEqual(parse_changed_right_lines(diff), {"app.py": {2, 3}})


class ReviewCommentTests(unittest.TestCase):
    def test_inline_payload_contains_marker_and_no_login_default(self) -> None:
        finding = {
            "agent": "security",
            "id": "security-1",
            "type": "MUST",
            "file": "src/lib.rs",
            "line": 12,
            "title": "토큰 노출 위험",
            "reason": "로그에 인증 헤더가 출력될 수 있습니다.",
        }
        key = finding_key(finding)
        body = render_inline_body(finding, key)
        self.assertIn(marker_for(key), body)
        self.assertIn("Codex Reviewer for DongwonTTuna", body)
        legacy_name = "codex-review" + "-" + "bot"
        self.assertNotIn(legacy_name, body)
        parsed_key, status = extract_marker(body)
        self.assertEqual((parsed_key, status), (key, "active"))


class EventParserTests(unittest.TestCase):
    def test_issue_comment_command_resolves_pr_number(self) -> None:
        os.environ["GITHUB_EVENT_NAME"] = "issue_comment"
        payload = {
            "comment": {"body": "/codex-review"},
            "issue": {"number": 7, "pull_request": {"url": "x"}},
        }
        self.assertEqual(requested_pr_number(payload), ("7", "issue_comment:/codex-review"))

    def test_authorize_rejects_bot_and_non_main(self) -> None:
        os.environ["GITHUB_ACTOR"] = "DongwonTTuna[bot]"
        pr = {
            "draft": False,
            "base": {"ref": "main"},
            "head": {"repo": {"full_name": "DongwonTTuna-Labs/demo"}},
            "user": {"login": "DongwonTTuna"},
        }
        allowed, reason = authorize({"sender": {"login": "DongwonTTuna[bot]"}}, pr, "DongwonTTuna-Labs/demo")
        self.assertFalse(allowed)
        self.assertIn("bot", reason)
        os.environ["GITHUB_ACTOR"] = "DongwonTTuna"
        pr["base"]["ref"] = "develop"
        allowed, reason = authorize({"sender": {"login": "DongwonTTuna"}}, pr, "DongwonTTuna-Labs/demo")
        self.assertFalse(allowed)
        self.assertIn("base ref", reason)


if __name__ == "__main__":
    unittest.main()
