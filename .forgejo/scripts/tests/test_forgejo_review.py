import json
import os
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import sys

SCRIPT_DIR = Path(__file__).resolve().parents[1]
REPO_ROOT = SCRIPT_DIR.parents[1]
sys.path.insert(0, str(SCRIPT_DIR))

from forgejo_api import ForgejoClient, ForgejoApiError, redact_secret, split_repo  # noqa: E402
from post_review_comments import (  # noqa: E402
    existing_by_key,
    extract_marker,
    finding_key,
    marker_for,
    post_inline_comments,
    render_inline_body,
    render_sticky,
)
from prepare_pr_context import is_bot_comment, parse_changed_right_lines  # noqa: E402
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

    def test_paginated_stops_on_short_page(self) -> None:
        client = ForgejoClient("https://git.example/api/v1", "owner/repo", "secret")
        with mock.patch.object(
            client,
            "request",
            side_effect=[[{"id": 1}], [{"id": 2}]],
        ) as request:
            self.assertEqual(client.paginated("items", limit=2), [{"id": 1}])
            request.assert_called_once()

    def test_request_redacts_token_on_http_error(self) -> None:
        client = ForgejoClient("https://git.example/api/v1", "owner/repo", "secret-token")
        response = mock.Mock()
        response.read.return_value = b"secret-token failed"
        error = __import__("urllib.error").error.HTTPError(
            "https://git.example/api/v1/items",
            500,
            "server error",
            {},
            response,
        )
        with mock.patch("urllib.request.urlopen", side_effect=error):
            with self.assertRaises(ForgejoApiError) as raised:
                client.request("POST", "items", {"x": 1})
        self.assertIn("<redacted>", str(raised.exception))
        self.assertNotIn("secret-token", str(raised.exception))


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

    def test_no_newline_marker_does_not_advance_line(self) -> None:
        diff = """diff --git a/app.py b/app.py
--- a/app.py
+++ b/app.py
@@ -1 +1,2 @@
+new
\\ No newline at end of file
+next
"""
        self.assertEqual(parse_changed_right_lines(diff), {"app.py": {1, 2}})


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
        self.assertFalse(body.startswith(" "))
        self.assertNotIn("\n        **[", body)
        self.assertNotIn("\n        _Codex Reviewer", body)
        parsed_key, status = extract_marker(body)
        self.assertEqual((parsed_key, status), (key, "active"))

    def test_finding_key_is_stable_when_title_or_reason_changes(self) -> None:
        base = {
            "agent": "security",
            "type": "MUST",
            "file": "src/lib.rs",
            "line": 12,
            "title": "old",
            "reason": "old reason",
        }
        changed = {**base, "title": "new", "reason": "new reason"}
        self.assertEqual(finding_key(base), finding_key(changed))

    def test_existing_marker_ignores_non_bot_comments(self) -> None:
        finding = {"agent": "security", "type": "MUST", "file": "src/lib.rs", "line": 12}
        key = finding_key(finding)
        body = marker_for(key)
        comments = [
            {"id": 1, "body": body, "user": {"login": "attacker"}},
            {"id": 2, "body": body, "user": {"login": "codex-reviewer"}},
        ]
        self.assertEqual(existing_by_key(comments)[key]["id"], 2)
        self.assertFalse(is_bot_comment(comments[0]))
        self.assertTrue(is_bot_comment(comments[1]))

    def test_changed_existing_inline_comment_is_deleted_and_reposted(self) -> None:
        class FakeClient:
            def __init__(self) -> None:
                self.calls = []

            def repo_path(self, path: str) -> str:
                return f"repos/owner/repo/{path}"

            def request(self, method: str, path: str, data=None, **kwargs):
                self.calls.append((method, path, data))
                return {}

        finding = {
            "agent": "test-coverage",
            "id": "test-coverage-4",
            "type": "MUST",
            "file": ".forgejo/scripts/forgejo_api.py",
            "line": 81,
            "title": "Forgejo API request 실패 경로 테스트가 없습니다",
            "reason": "urllib.request.urlopen mock coverage를 추가해야 합니다.",
        }
        key = finding_key(finding)
        stale_body = "\n".join(
            [
                marker_for(key),
                "        **[MUST] Forgejo API request 실패 경로 테스트가 없습니다**",
                "",
                "urllib.request.urlopen mock coverage를 추가해야 합니다.",
                "",
                "        _Codex Reviewer for DongwonTTuna: `test-coverage` / `test-coverage-4`_",
            ]
        )
        client = FakeClient()

        posted, skipped = post_inline_comments(
            client,
            "4",
            "abc123",
            [finding],
            {
                key: {
                    "id": 61,
                    "pull_request_review_id": 1,
                    "body": stale_body,
                    "user": {"login": "codex-reviewer"},
                }
            },
        )

        self.assertEqual((posted, skipped), (1, 0))
        self.assertEqual(client.calls[0][0], "DELETE")
        self.assertIn("pulls/4/reviews/1/comments/61", client.calls[0][1])
        self.assertEqual(client.calls[1][0], "POST")
        self.assertIn("pulls/4/reviews", client.calls[1][1])
        self.assertEqual(client.calls[1][2]["comments"][0]["body"], render_inline_body(finding, key))

    def test_sticky_includes_cross_cutting_findings(self) -> None:
        body = render_sticky(
            "2",
            [{"type": "MUST", "title": "workflow gate is unsafe", "agent": "security", "cross_cutting": True}],
            posted=0,
            skipped_existing=0,
            resolved=0,
            axes_status={"missing": []},
            judgment=None,
        )
        self.assertIn("Cross-cutting findings", body)
        self.assertIn("workflow gate is unsafe", body)


class EventParserTests(unittest.TestCase):
    def test_issue_comment_command_resolves_pr_number(self) -> None:
        os.environ["GITHUB_EVENT_NAME"] = "issue_comment"
        payload = {
            "comment": {"body": "/codex-review"},
            "issue": {"number": 7, "pull_request": {"url": "x"}},
        }
        self.assertEqual(requested_pr_number(payload), ("7", "issue_comment:/codex-review"))

    def test_pull_request_target_resolves_pr_number(self) -> None:
        os.environ["GITHUB_EVENT_NAME"] = "pull_request_target"
        payload = {"action": "synchronize", "pull_request": {"number": 8}}
        self.assertEqual(requested_pr_number(payload), ("8", "pull_request_target:synchronize"))

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

    def test_authorize_rejects_missing_head_repo(self) -> None:
        os.environ["GITHUB_ACTOR"] = "DongwonTTuna"
        pr = {
            "draft": False,
            "base": {"ref": "main"},
            "head": {"repo": {}},
            "user": {"login": "DongwonTTuna"},
        }
        allowed, reason = authorize({"sender": {"login": "DongwonTTuna"}}, pr, "DongwonTTuna-Labs/demo")
        self.assertFalse(allowed)
        self.assertIn("fork PR", reason)


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

    def test_workflows_use_unified_runner_label(self) -> None:
        text = self.forgejo_text()
        self.assertIn("runs-on: dongwontuna-labs-runner", text)
        self.assertNotIn("runs-on: codex", text)
        self.assertNotIn("runs-on: rust", text)
        self.assertNotIn("runs-on: ubuntu-latest", text)

    def test_workflows_use_non_reserved_secret_name(self) -> None:
        text = self.forgejo_text()
        self.assertIn("secrets.CODEX_REVIEW_BOT_TOKEN", text)
        self.assertNotIn("secrets.FORGEJO_BOT_TOKEN", text)

    def test_manual_dispatch_can_use_bootstrap_scripts_ref(self) -> None:
        text = self.forgejo_text()
        self.assertIn("scripts_ref:", text)
        self.assertIn("SCRIPTS_REF", text)
        self.assertIn('scripts_ref="main"', text)
        self.assertIn('scripts_ref="${GITHUB_SHA}"', text)
        self.assertIn("CODEX_BOOTSTRAP_SCRIPTS_REF: ${{ github.sha }}", text)
        self.assertIn("Bootstrap exception", text)

    def test_post_job_uses_default_needs_success_gate(self) -> None:
        text = self.forgejo_text()
        self.assertIn("needs: tech-lead", text)
        self.assertNotIn("needs.tech-lead.result", text)
        self.assertNotIn("Publish Forgejo review comments\n        if: always()", text)

    def test_bot_token_is_scoped_to_api_steps(self) -> None:
        pipeline = (REPO_ROOT / ".forgejo" / "workflows" / "codex-pr-review-pipeline.yml").read_text(encoding="utf-8")
        self.assertEqual(pipeline.count("FORGEJO_BOT_TOKEN: ${{ secrets.CODEX_REVIEW_BOT_TOKEN }}"), 2)
        self.assertIn("- name: Prepare Forgejo PR review context\n        env:\n          FORGEJO_BOT_TOKEN", pipeline)
        self.assertIn("- name: Publish Forgejo review comments\n        env:\n          FORGEJO_BOT_TOKEN", pipeline)

    def test_codex_exec_unsets_ci_tokens(self) -> None:
        script = (REPO_ROOT / ".github" / "scripts" / "codex_exec.sh").read_text(encoding="utf-8")
        for name in ["GIT_AUTH_TOKEN", "GITHUB_TOKEN", "FORGEJO_BOT_TOKEN", "ACTIONS_RUNTIME_TOKEN"]:
            self.assertIn(f"unset {name}", script)

    def test_auto_review_uses_resolver_authorization(self) -> None:
        workflow = (REPO_ROOT / ".forgejo" / "workflows" / "codex-pr-review.yml").read_text(encoding="utf-8")
        self.assertIn("CODEX_ALLOWED_LOGIN: ${{ vars.CODEX_ALLOWED_LOGIN || 'DongwonTTuna' }}", workflow)
        self.assertIn("run: python3 .forgejo/scripts/resolve_pr_metadata.py", workflow)
        self.assertNotIn("github.event.pull_request.user.login == 'DongwonTTuna'", workflow)

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
