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

from forgejo_api import ForgejoApiError, ForgejoClient, redact_secret, sanitized_url_error, split_repo  # noqa: E402
from post_review_comments import (  # noqa: E402
    changed_line_map,
    existing_by_key,
    extract_marker,
    finding_key,
    marker_for,
    post_inline_comments,
    require_json_list,
    render_inline_body,
    render_resolved_body,
    render_sticky,
    reply_resolved,
    sanitize_model_text,
)
import prepare_pr_context  # noqa: E402
import resolve_pr_metadata  # noqa: E402
from prepare_pr_context import configure_bot_login, fetch_review_comments, is_bot_comment, parse_changed_right_lines  # noqa: E402
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

    def test_paginated_default_reads_until_empty_page(self) -> None:
        client = ForgejoClient("https://git.example/api/v1", "owner/repo", "secret")
        pages = [[{"id": page}] for page in range(1, 22)] + [[]]
        with mock.patch.object(client, "request", side_effect=pages) as request:
            self.assertEqual(len(client.paginated("items", limit=1)), 21)
            self.assertEqual(request.call_count, 22)

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

    def test_request_builds_json_auth_payload_and_parses_json(self) -> None:
        client = ForgejoClient("https://git.example/api/v1", "owner/repo", "secret-token")
        response = mock.Mock()
        response.__enter__ = mock.Mock(return_value=response)
        response.__exit__ = mock.Mock(return_value=None)
        response.read.return_value = b'{"ok": true}'
        response.headers = {"Content-Type": "application/json; charset=utf-8"}
        with mock.patch("urllib.request.urlopen", return_value=response) as urlopen:
            self.assertEqual(client.request("POST", "items", {"x": 1}), {"ok": True})
        req = urlopen.call_args.args[0]
        self.assertEqual(req.get_method(), "POST")
        self.assertEqual(req.get_header("Authorization"), "token secret-token")
        self.assertEqual(json.loads(req.data.decode("utf-8")), {"x": 1})

    def test_get_retries_transient_http_error(self) -> None:
        client = ForgejoClient("https://git.example/api/v1", "owner/repo", "secret-token")
        error = __import__("urllib.error").error.HTTPError(
            "https://git.example/api/v1/items",
            503,
            "temporary",
            {},
            mock.Mock(read=lambda: b"try again"),
        )
        response = mock.Mock()
        response.__enter__ = mock.Mock(return_value=response)
        response.__exit__ = mock.Mock(return_value=None)
        response.read.return_value = b"[]"
        response.headers = {"Content-Type": "application/json"}
        with mock.patch("time.sleep"), mock.patch("urllib.request.urlopen", side_effect=[error, response]) as urlopen:
            self.assertEqual(client.request("GET", "items"), [])
        self.assertEqual(urlopen.call_count, 2)

    def test_url_error_reason_is_sanitized(self) -> None:
        client = ForgejoClient("https://git.example/api/v1", "owner/repo", "secret-token")
        error = __import__("urllib.error").error.URLError("proxy leaked secret-token")
        with mock.patch("urllib.request.urlopen", side_effect=error):
            with self.assertRaises(ForgejoApiError) as raised:
                client.request("POST", "items", {"x": 1})
        self.assertNotIn("secret-token", str(raised.exception))
        self.assertEqual(sanitized_url_error(error.reason), "network error")

    def test_authenticated_login_comes_from_token_owner(self) -> None:
        client = ForgejoClient("https://git.example/api/v1", "owner/repo", "secret-token")
        with mock.patch.dict(os.environ, {}, clear=True), mock.patch.object(
            client,
            "request",
            return_value={"login": "codex-reviewer-for-dongwonttuna"},
        ):
            self.assertEqual(configure_bot_login(client), "codex-reviewer-for-dongwonttuna")

    def test_configured_bot_login_avoids_extra_user_scope(self) -> None:
        client = ForgejoClient("https://git.example/api/v1", "owner/repo", "secret-token")
        with mock.patch.dict(
            os.environ,
            {"FORGEJO_BOT_LOGIN": "codex-reviewer-for-dongwonttuna"},
            clear=True,
        ), mock.patch.object(client, "request") as request:
            self.assertEqual(configure_bot_login(client), "codex-reviewer-for-dongwonttuna")
            request.assert_not_called()


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

    def test_multiple_files_new_file_delete_file_and_literal_header_lines(self) -> None:
        diff = """diff --git a/a.py b/a.py
--- a/a.py
+++ b/a.py
@@ -1 +1,3 @@
 keep
+++literal
+added
diff --git a/new.py b/new.py
--- /dev/null
+++ b/new.py
@@ -0,0 +1,2 @@
+n1
+n2
diff --git a/deleted.py b/deleted.py
--- a/deleted.py
+++ /dev/null
@@ -1 +0,0 @@
-gone
"""
        self.assertEqual(parse_changed_right_lines(diff), {"a.py": {2, 3}, "new.py": {1, 2}})


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

    def test_model_markdown_is_escaped_before_posting(self) -> None:
        finding = {
            "agent": "security",
            "id": "security-1",
            "type": "MUST",
            "file": "src/lib.rs",
            "line": 12,
            "title": "@all <script>",
            "reason": "![x](http://example.test) @here",
        }
        body = render_inline_body(finding, finding_key(finding))
        self.assertIn("@\u200ball &lt;script&gt;", body)
        self.assertIn("@\u200bhere", body)
        self.assertNotIn("<script>", body)

    def test_require_json_list_rejects_wrong_shape(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "allowed.json"
            path.write_text(json.dumps({"findings": []}), encoding="utf-8")
            with self.assertRaises(SystemExit):
                require_json_list(path)

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
            {"id": 2, "body": body, "user": {"login": "codex-reviewer-for-dongwonttuna"}},
        ]
        self.assertEqual(existing_by_key(comments), {})
        self.assertFalse(is_bot_comment(comments[0]))
        self.assertFalse(is_bot_comment(comments[1]))
        with mock.patch.dict(os.environ, {"FORGEJO_BOT_LOGIN": "codex-reviewer-for-dongwonttuna"}, clear=False):
            self.assertEqual(existing_by_key(comments)[key]["id"], 2)
            self.assertFalse(is_bot_comment(comments[0]))
            self.assertTrue(is_bot_comment(comments[1]))
            self.assertFalse(is_bot_comment({"user": {"login": "codex-reviewer"}}))

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
            {".forgejo/scripts/forgejo_api.py": {81}},
        )

        self.assertEqual((posted, skipped), (1, 0))
        self.assertEqual(client.calls[0][0], "DELETE")
        self.assertIn("pulls/4/reviews/1/comments/61", client.calls[0][1])
        self.assertEqual(client.calls[1][0], "POST")
        self.assertIn("pulls/4/reviews", client.calls[1][1])
        self.assertEqual(client.calls[1][2]["comments"][0]["body"], render_inline_body(finding, key))

    def test_inline_post_skips_finding_outside_changed_lines(self) -> None:
        class FakeClient:
            def repo_path(self, path: str) -> str:
                return f"repos/owner/repo/{path}"

            def request(self, method: str, path: str, data=None, **kwargs):
                raise AssertionError("HTTP request should not be made")

        finding = {
            "agent": "security",
            "type": "MUST",
            "file": "src/lib.rs",
            "line": 99,
            "title": "x",
            "reason": "y",
        }
        self.assertEqual(
            post_inline_comments(FakeClient(), "4", "abc123", [finding], {}, {"src/lib.rs": {10}}),
            (0, 0),
        )

    def test_stale_inline_comment_is_marked_resolved_without_delete(self) -> None:
        class FakeClient:
            def __init__(self) -> None:
                self.calls = []

            def repo_path(self, path: str) -> str:
                return f"repos/owner/repo/{path}"

            def request(self, method: str, path: str, data=None, **kwargs):
                self.calls.append((method, path, data))
                return {}

        key = "0123456789abcdef"
        client = FakeClient()
        comments = [
                {
                    "id": 61,
                    "pull_request_review_id": 1,
                    "body": marker_for(key),
                    "user": {"login": "codex-reviewer-for-dongwonttuna"},
                }
            ]
        with mock.patch.dict(os.environ, {"FORGEJO_BOT_LOGIN": "codex-reviewer-for-dongwonttuna"}, clear=False):
            resolved = reply_resolved(
                client,
                "4",
                comments,
                current_keys=set(),
            )

            resolved_again = reply_resolved(
                client,
                "4",
                [{**comments[0], "body": render_resolved_body(key)}],
                current_keys=set(),
            )

        self.assertEqual(resolved_again, 0)
        self.assertEqual(len(client.calls), 1)

        self.assertEqual(resolved, 1)
        self.assertEqual(client.calls, [("PATCH", "repos/owner/repo/issues/comments/61", {"body": render_resolved_body(key)})])
        self.assertNotIn("DELETE", [call[0] for call in client.calls])

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

    def test_changed_line_map_ignores_bad_context_rows(self) -> None:
        context = {
            "changed_files": [
                {"filename": "src/lib.rs", "changed_right_lines": [1, "2", "bad"]},
                {"filename": "", "changed_right_lines": [99]},
            ]
        }
        self.assertEqual(changed_line_map(context), {"src/lib.rs": {1, 2}})


class EventParserTests(unittest.TestCase):
    def setUp(self) -> None:
        self.env_patch = mock.patch.dict(os.environ, {}, clear=True)
        self.env_patch.start()

    def tearDown(self) -> None:
        self.env_patch.stop()

    def test_issue_comment_command_resolves_pr_number(self) -> None:
        os.environ["GITHUB_EVENT_NAME"] = "issue_comment"
        payload = {
            "comment": {"body": "/codex-review"},
            "issue": {"number": 7, "pull_request": {"url": "x"}},
        }
        self.assertEqual(requested_pr_number(payload), ("7", "issue_comment:/codex-review"))

    def test_issue_comment_edited_command_resolves_pr_number(self) -> None:
        os.environ["GITHUB_EVENT_NAME"] = "issue_comment"
        payload = {
            "action": "edited",
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

    def test_resolver_main_outputs_snapshot_and_scripts_ref(self) -> None:
        class FakeClient:
            repo = "DongwonTTuna-Labs/demo"

            def repo_path(self, path: str) -> str:
                return f"repos/DongwonTTuna-Labs/demo/{path}"

            def request(self, method: str, path: str, **_kwargs):
                self.seen = (method, path)
                return {
                    "number": 7,
                    "draft": False,
                    "user": {"login": "DongwonTTuna"},
                    "head": {
                        "sha": "1" * 40,
                        "repo": {"full_name": "DongwonTTuna-Labs/demo"},
                    },
                    "base": {"ref": "main", "sha": "2" * 40},
                }

        with tempfile.TemporaryDirectory() as tmp:
            event = Path(tmp) / "event.json"
            output = Path(tmp) / "outputs"
            event.write_text(json.dumps({"inputs": {"pr_number": "7"}, "sender": {"login": "DongwonTTuna"}}))
            env = {
                "GITHUB_EVENT_NAME": "workflow_dispatch",
                "GITHUB_EVENT_PATH": str(event),
                "GITHUB_OUTPUT": str(output),
                "GITHUB_ACTOR": "DongwonTTuna",
            }
            with mock.patch.dict(os.environ, env, clear=True), mock.patch.object(
                ForgejoClient,
                "from_env",
                return_value=FakeClient(),
            ):
                self.assertEqual(resolve_pr_metadata.main(), 0)
            lines = output.read_text(encoding="utf-8").splitlines()
        self.assertIn("allowed=true", lines)
        self.assertIn("head_sha=" + "1" * 40, lines)
        self.assertIn("base_sha=" + "2" * 40, lines)
        self.assertIn("scripts_ref=" + "2" * 40, lines)


class PrepareContextMainTests(unittest.TestCase):
    def test_fetch_review_comments_caps_review_comment_api_calls(self) -> None:
        class FakeClient:
            def __init__(self) -> None:
                self.comment_calls = 0

            def repo_path(self, path: str) -> str:
                return f"repos/owner/repo/{path}"

            def paginated(self, path: str, query=None, limit: int = 100, max_pages=None):
                if path.endswith("pulls/7/reviews"):
                    return [{"id": i} for i in range(100)]
                if "/reviews/" in path and path.endswith("/comments"):
                    self.comment_calls += 1
                    return [{"id": self.comment_calls}]
                return []

        client = FakeClient()
        comments = fetch_review_comments(client, "7")
        self.assertEqual(len(comments), prepare_pr_context.MAX_REVIEW_COMMENT_REVIEW_SCAN)
        self.assertEqual(client.comment_calls, prepare_pr_context.MAX_REVIEW_COMMENT_REVIEW_SCAN)

    def test_main_writes_context_with_token_owner_and_full_diff_line_scan(self) -> None:
        class FakeClient:
            repo = "DongwonTTuna-Labs/demo"

            def repo_path(self, path: str) -> str:
                return f"repos/DongwonTTuna-Labs/demo/{path}"

            def authenticated_login(self) -> str:
                return "codex-reviewer-for-dongwonttuna"

            def request(self, method: str, path: str, **_kwargs):
                self.requested = (method, path)
                return {
                    "title": "PR",
                    "body": "",
                    "head": {"sha": "1" * 40, "ref": "feature"},
                    "base": {"sha": "2" * 40, "ref": "main"},
                }

            def request_text_limited(self, path: str, limit: int, accept: str = "text/plain"):
                return (
                    "diff --git a/a.py b/a.py\n"
                    "--- a/a.py\n"
                    "+++ b/a.py\n"
                    "@@ -1 +1,2 @@\n"
                    " keep\n"
                    "+added\n",
                    False,
                )

            def paginated(self, path: str, query=None, limit: int = 100, max_pages=None):
                if path.endswith("/files"):
                    return [{"filename": "a.py", "status": "modified"}]
                if path.endswith("/comments"):
                    return []
                if path.endswith("/reviews"):
                    return []
                return []

        with tempfile.TemporaryDirectory() as tmp:
            env = {
                "PR_NUMBER": "7",
                "RUNNER_TEMP": tmp,
                "HEAD_SHA": "1" * 40,
                "BASE_SHA": "2" * 40,
            }
            with mock.patch.dict(os.environ, env, clear=True), mock.patch.object(
                ForgejoClient,
                "from_env",
                return_value=FakeClient(),
            ):
                self.assertEqual(prepare_pr_context.main(), 0)
            context = json.loads((Path(tmp) / "pr-context.json").read_text(encoding="utf-8"))
        self.assertEqual(context["changed_files"][0]["changed_right_lines"], [2])
        self.assertEqual(os.environ.get("FORGEJO_BOT_LOGIN"), None)


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

    def test_rust_ci_prewarms_action_cache_before_parallel_jobs(self) -> None:
        workflow = (REPO_ROOT / ".forgejo" / "workflows" / "rust-ci.yml").read_text(encoding="utf-8")
        self.assertIn("prepare-actions:", workflow)
        self.assertIn("name: Prepare action cache", workflow)
        self.assertIn("lookup-only: true", workflow)
        self.assertEqual(workflow.count("needs: prepare-actions"), 4)

    def test_workflows_use_non_reserved_secret_name(self) -> None:
        text = self.forgejo_text()
        self.assertIn("secrets.CODEX_REVIEW_BOT_TOKEN", text)
        self.assertNotIn("secrets.FORGEJO_BOT_TOKEN", text)

    def test_pipeline_uses_configured_bot_login_without_requiring_user_scope(self) -> None:
        pipeline = (REPO_ROOT / ".forgejo" / "workflows" / "codex-pr-review-pipeline.yml").read_text(
            encoding="utf-8"
        )
        self.assertIn("FORGEJO_BOT_LOGIN:", pipeline)
        self.assertIn("vars.FORGEJO_BOT_LOGIN", pipeline)
        self.assertIn("codex-reviewer-for-dongwontuna", pipeline)

    def test_manual_dispatch_can_use_bootstrap_scripts_ref(self) -> None:
        text = self.forgejo_text()
        self.assertIn("scripts_ref:", text)
        self.assertIn("SCRIPTS_REF", text)
        self.assertIn("SCRIPTS_REF: main", text)
        self.assertNotIn('scripts_ref="${GITHUB_SHA}"', text)
        self.assertNotIn("CODEX_BOOTSTRAP_SCRIPTS_REF", text)
        self.assertNotIn("Bootstrap exception", text)

    def test_trusted_script_checkout_fetches_branch_before_sha_checkout(self) -> None:
        text = self.forgejo_text()
        self.assertNotIn('git_fetch fetch --depth=1 origin "$SCRIPTS_REF"', text)
        self.assertNotIn('git_fetch fetch --depth=1 origin "$scripts_ref"', text)
        self.assertIn('git_fetch fetch --depth=256 origin "refs/heads/$trusted_ref"', text)
        self.assertIn('git cat-file -e "$SCRIPTS_REF^{commit}"', text)
        self.assertIn('git_fetch fetch --deepen=256 origin "refs/heads/$trusted_ref"', text)
        self.assertIn('git checkout --detach "$SCRIPTS_REF"', text)

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
        for workflow in ("codex-pr-review.yml", "codex-pr-review-on-comment.yml"):
            text = (REPO_ROOT / ".forgejo" / "workflows" / workflow).read_text(encoding="utf-8")
            self.assertNotIn("secrets: inherit", text)
            self.assertIn("CODEX_REVIEW_BOT_TOKEN: ${{ secrets.CODEX_REVIEW_BOT_TOKEN }}", text)

    def test_codex_exec_unsets_ci_tokens(self) -> None:
        script = (REPO_ROOT / ".github" / "scripts" / "codex_exec.sh").read_text(encoding="utf-8")
        for name in [
            "GIT_AUTH_TOKEN",
            "GITHUB_TOKEN",
            "FORGEJO_BOT_TOKEN",
            "ACTIONS_RUNTIME_TOKEN",
            "ACTIONS_CACHE_URL",
            "ACTIONS_RESULTS_URL",
            "ACTIONS_RUNTIME_URL",
        ]:
            self.assertIn(f"unset {name}", script)

    def test_comment_review_trigger_uses_issue_comment_created_and_edited(self) -> None:
        workflow = (REPO_ROOT / ".forgejo" / "workflows" / "codex-pr-review-on-comment.yml").read_text(
            encoding="utf-8"
        )
        self.assertIn("issue_comment:", workflow)
        self.assertIn("types: [created, edited]", workflow)
        self.assertNotIn("issues:\n    types: [edited]", workflow)
        self.assertIn("cancel-in-progress: false", workflow)

    def test_workflows_unset_fetch_token_before_checkout(self) -> None:
        for path in (REPO_ROOT / ".forgejo" / "workflows").glob("*.yml"):
            text = path.read_text(encoding="utf-8")
            self.assertNotRegex(text, r"git_fetch fetch --depth=1 origin [^\n]+\n\s+git checkout --detach FETCH_HEAD")

    def test_codex_scripts_tests_watch_github_scripts(self) -> None:
        workflow = (REPO_ROOT / ".forgejo" / "workflows" / "codex-scripts-tests.yml").read_text(
            encoding="utf-8"
        )
        self.assertGreaterEqual(workflow.count('".github/scripts/**"'), 2)

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
