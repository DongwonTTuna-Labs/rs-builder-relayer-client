import json
import os
import tempfile
import unittest
from pathlib import Path

import sys

SCRIPT_DIR = Path(__file__).resolve().parents[1]
REPO_ROOT = SCRIPT_DIR.parents[1]
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


class WorkflowParityTests(unittest.TestCase):
    def forgejo_text(self) -> str:
        chunks = []
        for path in (REPO_ROOT / ".forgejo").rglob("*"):
            if "tests" in path.parts:
                continue
            if path.suffix not in {".yml", ".yaml", ".py", ".sh"}:
                continue
            chunks.append(path.read_text(encoding="utf-8"))
        return "\n".join(chunks)

    def test_workflows_do_not_keep_legacy_github_review_integrations(self) -> None:
        text = self.forgejo_text()
        forbidden = [
            "api.github.com",
            "gh api",
            "actions/create-github-app-token",
            "CODEX_APP_",
            "codex-review-bot",
            "data.forgejo.org",
        ]
        for value in forbidden:
            self.assertNotIn(value, text)

    def test_actions_use_canonical_forgejo_sources(self) -> None:
        text = self.forgejo_text()
        self.assertIn("https://code.forgejo.org/forgejo/upload-artifact@v4", text)
        self.assertIn("https://code.forgejo.org/forgejo/download-artifact@v4", text)
        self.assertNotIn("https://data.forgejo.org/actions/cache", text)
        self.assertNotIn("https://data.forgejo.org/actions/upload-artifact", text)
        self.assertNotIn("https://data.forgejo.org/actions/download-artifact", text)
        if "actions/cache@v4" in text:
            self.assertIn("https://code.forgejo.org/actions/cache@v4", text)

    def test_workflows_do_not_bootstrap_runner_toolchains(self) -> None:
        text = self.forgejo_text()
        self.assertNotIn("setup-node", text)
        self.assertNotIn("setup-rust", text)
        self.assertNotIn("install_node_action_runtime", text)
        self.assertNotIn("NODE_MAJOR", text)
        self.assertNotIn("NODE_VERSION", text)
        self.assertNotIn("node-v$NODE_VERSION", text)

    def test_manual_checkout_commands_end_before_next_step(self) -> None:
        for path in (REPO_ROOT / ".forgejo" / "workflows").glob("*.yml"):
            for line in path.read_text(encoding="utf-8").splitlines():
                if "git checkout --detach FETCH_HEAD" in line:
                    self.assertEqual(
                        line.strip(),
                        "git checkout --detach FETCH_HEAD",
                        f"malformed checkout command in {path}",
                    )


if __name__ == "__main__":
    unittest.main()
