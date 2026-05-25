import json
import os
import sys
import unittest
import urllib.error
from pathlib import Path
from unittest import mock

import yaml

SCRIPT_DIR = Path(__file__).resolve().parents[1]
REPO_ROOT = SCRIPT_DIR.parents[1]
sys.path.insert(0, str(SCRIPT_DIR))

from forgejo_api import ForgejoApiError, ForgejoClient, redact_secret, sanitize_error_detail, split_repo  # noqa: E402
from post_review_comments import finding_key, marker_for, render_sticky  # noqa: E402
from prepare_pr_context import is_bot_comment, parse_changed_right_lines  # noqa: E402
from resolve_pr_metadata import authorize, requested_pr_number  # noqa: E402


class FakeResponse:
    def __init__(self, chunks: list[bytes], content_type: str = "application/json") -> None:
        self.chunks = list(chunks)
        self.headers = {"Content-Type": content_type}

    def __enter__(self) -> "FakeResponse":
        return self

    def __exit__(self, *args) -> None:
        return None

    def read(self, size: int | None = None) -> bytes:
        if not self.chunks:
            return b""
        return self.chunks.pop(0)


class ForgejoApiTests(unittest.TestCase):
    def test_redaction_removes_token_from_errors(self) -> None:
        self.assertEqual(redact_secret("token abc123 leaked", "abc123"), "token <redacted> leaked")

    def test_sanitize_error_detail_masks_url_credentials_and_query_tokens(self) -> None:
        text = "failed https://user:pass@git.example/api?access_token=url-secret token secret-token"
        sanitized = sanitize_error_detail(text, "secret-token")
        self.assertIn("https://<redacted>@git.example/api?access_token=<redacted>", sanitized)
        self.assertNotIn("user:pass", sanitized)
        self.assertNotIn("url-secret", sanitized)
        self.assertNotIn("secret-token", sanitized)

    def test_split_repo_rejects_invalid_shape(self) -> None:
        self.assertEqual(split_repo("DongwonTTuna-Labs/demo"), ("DongwonTTuna-Labs", "demo"))
        with self.assertRaises(SystemExit):
            split_repo("demo")

    def test_request_text_prefix_streams_lines_without_storing_full_response(self) -> None:
        client = ForgejoClient("https://git.example/api/v1", "owner/repo", "secret")
        lines: list[str] = []
        with mock.patch(
            "urllib.request.urlopen",
            return_value=FakeResponse([b"line-1\nline-", b"2\nline-3"], "text/plain"),
        ):
            prefix, truncated = client.request_text_prefix("diff", 12, on_line=lines.append, chunk_size=6)
        self.assertEqual(prefix, "line-1\nline-")
        self.assertTrue(truncated)
        self.assertEqual(lines, ["line-1", "line-2", "line-3"])

    def test_request_redacts_token_on_http_error(self) -> None:
        client = ForgejoClient("https://git.example/api/v1", "owner/repo", "secret-token")
        response = mock.Mock()
        response.read.return_value = b"secret-token failed"
        error = urllib.error.HTTPError("https://git.example/api/v1/items", 500, "server error", {}, response)
        with mock.patch("urllib.request.urlopen", side_effect=error):
            with self.assertRaises(ForgejoApiError) as raised:
                client.request("POST", "items", {"x": 1})
        self.assertIn("<redacted>", str(raised.exception))
        self.assertNotIn("secret-token", str(raised.exception))


class ReviewContextTests(unittest.TestCase):
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

    def test_bot_comment_detection_uses_forgejo_reviewer_identity(self) -> None:
        with mock.patch.dict(os.environ, {"FORGEJO_BOT_LOGIN": "codex-reviewer-for-dongwonttuna"}):
            self.assertTrue(is_bot_comment({"user": {"login": "codex-reviewer-for-dongwonttuna"}}))
            self.assertFalse(is_bot_comment({"user": {"login": "DongwonTTuna"}}))

    def test_review_comment_marker_is_stable(self) -> None:
        finding = {"agent": "security", "type": "MUST", "file": "src/lib.rs", "line": 12}
        key = finding_key(finding)
        self.assertEqual(finding_key({**finding, "title": "changed"}), key)
        self.assertIn(key, marker_for(key))

    def test_sticky_summary_renders_lgtm_without_findings(self) -> None:
        body = render_sticky(
            "2",
            [],
            posted=0,
            skipped_existing=0,
            resolved=0,
            skipped_inline=0,
            axes_status={"missing": []},
            judgment={"status": "LGTM", "headline": "No findings."},
        )
        self.assertIn("Status: **LGTM**", body)
        self.assertIn("Missing axes: none", body)
        self.assertIn("재리뷰는 `/codex-review` 코멘트로 요청하세요.", body)


class ResolveMetadataTests(unittest.TestCase):
    def setUp(self) -> None:
        self.env_patch = mock.patch.dict(os.environ, {}, clear=True)
        self.env_patch.start()

    def tearDown(self) -> None:
        self.env_patch.stop()

    def test_issue_comment_command_resolves_pr_number(self) -> None:
        os.environ["GITHUB_EVENT_NAME"] = "issue_comment"
        payload = {
            "action": "created",
            "comment": {"body": "/codex-review"},
            "issue": {"number": 7, "pull_request": {"url": "x"}},
        }
        self.assertEqual(requested_pr_number(payload), ("7", "issue_comment:/codex-review"))

    def test_issue_comment_edits_do_not_trigger_review(self) -> None:
        os.environ["GITHUB_EVENT_NAME"] = "issue_comment"
        payload = {
            "action": "edited",
            "comment": {"body": "/codex-review"},
            "issue": {"number": 7, "pull_request": {"url": "x"}},
        }
        self.assertEqual(requested_pr_number(payload), (None, "issue_comment:edited"))

    def test_authorize_allows_non_default_base_when_surface_check_is_deferred(self) -> None:
        os.environ["GITHUB_ACTOR"] = "DongwonTTuna"
        pr = {
            "draft": False,
            "title": "ready",
            "base": {"ref": "release-candidate"},
            "head": {"repo": {"full_name": "DongwonTTuna-Labs/demo"}, "sha": "1" * 40},
            "user": {"login": "DongwonTTuna"},
        }
        allowed, reason = authorize({"sender": {"login": "DongwonTTuna"}}, pr, "DongwonTTuna-Labs/demo")
        self.assertTrue(allowed, reason)

    def test_authorize_rejects_bot_actor_draft_fork_and_wip_title(self) -> None:
        base_pr = {
            "draft": False,
            "title": "ready",
            "head": {"repo": {"full_name": "DongwonTTuna-Labs/demo"}},
            "user": {"login": "DongwonTTuna"},
        }
        os.environ["GITHUB_ACTOR"] = "DongwonTTuna[bot]"
        allowed, reason = authorize({"sender": {"login": "DongwonTTuna[bot]"}}, base_pr, "DongwonTTuna-Labs/demo")
        self.assertFalse(allowed)
        self.assertIn("bot", reason)

        os.environ["GITHUB_ACTOR"] = "DongwonTTuna"
        for pr, expected in [
            ({**base_pr, "draft": True}, "draft"),
            ({**base_pr, "head": {"repo": {"full_name": "someone/demo"}}}, "fork"),
            ({**base_pr, "title": "WIP: not ready"}, "WIP"),
        ]:
            allowed, reason = authorize({"sender": {"login": "DongwonTTuna"}}, pr, "DongwonTTuna-Labs/demo")
            self.assertFalse(allowed)
            self.assertIn(expected, reason)


class WorkflowParityTests(unittest.TestCase):
    def forgejo_text(self) -> str:
        chunks: list[str] = []
        for path in (REPO_ROOT / ".forgejo").rglob("*"):
            if "tests" in path.parts:
                continue
            if path.suffix in {".yml", ".yaml", ".py", ".sh", ".json"}:
                chunks.append(path.read_text(encoding="utf-8"))
        return "\n".join(chunks)

    def test_workflow_yaml_parses(self) -> None:
        for path in sorted((REPO_ROOT / ".forgejo" / "workflows").glob("*.yml")):
            with self.subTest(path=path):
                with path.open(encoding="utf-8") as handle:
                    self.assertIsInstance(yaml.safe_load(handle), dict)

    def test_workflow_set_matches_forgejo_only_stack(self) -> None:
        expected_required = {"codex-pr-review.yml", "codex-pr-review-pipeline.yml"}
        expected_optional = {"rust-ci.yml", "metadata.yml"}
        actual = {path.name for path in (REPO_ROOT / ".forgejo" / "workflows").glob("*.yml")}
        self.assertTrue(expected_required.issubset(actual))
        self.assertLessEqual(actual, expected_required | expected_optional)

    def test_no_legacy_action_metadata_remains_in_forgejo_or_agents(self) -> None:
        legacy_dir = "." + "github"
        forbidden = [
            legacy_dir,
            "api." + "github.com",
            "actions/create-" + "github-app-token",
            "github-" + "actions",
        ]
        text = self.forgejo_text()
        text += "\n".join(path.read_text(encoding="utf-8") for path in (REPO_ROOT / ".codex" / "agents").glob("*.md"))
        for value in forbidden:
            self.assertNotIn(value, text)

    def test_codex_review_uses_default_branch_trusted_scripts(self) -> None:
        workflow = (REPO_ROOT / ".forgejo" / "workflows" / "codex-pr-review.yml").read_text(encoding="utf-8")
        self.assertIn("pull_request_target:", workflow)
        self.assertIn("types: [opened, synchronize, reopened]", workflow)
        self.assertNotIn("types: [opened, synchronize, reopened, edited]", workflow)
        self.assertNotIn("ready_for_review", workflow)
        self.assertIn("issue_comment:", workflow)
        self.assertIn("workflow_dispatch:", workflow)
        self.assertIn("github.event_name != 'issue_comment' || github.event.action == 'created'", workflow)
        self.assertIn("git_fetch ls-remote --symref origin HEAD", workflow)
        self.assertIn('git_fetch fetch --depth=1 origin "refs/heads/$default_branch"', workflow)
        self.assertIn("CODEX_DEFAULT_BRANCH=$default_branch", workflow)
        self.assertIn("CODEX_DEFAULT_SHA=$(git rev-parse HEAD)", workflow)
        self.assertNotIn("branches: [main]", workflow)

    def test_pipeline_uses_forgejo_scripts_and_shared_codex_auth(self) -> None:
        pipeline = (REPO_ROOT / ".forgejo" / "workflows" / "codex-pr-review-pipeline.yml").read_text(encoding="utf-8")
        self.assertIn("python3 pipeline/.forgejo/scripts/build_prompt.py", pipeline)
        self.assertIn("bash pipeline/.forgejo/scripts/codex_exec.sh", pipeline)
        self.assertIn("/codex-runner-home:/home/runner/.codex", pipeline)
        self.assertIn("/codex-runner-locks:/var/lib/codex-runner/auth-runs-root", pipeline)
        self.assertIn('auth_lock_file="$auth_lock_dir/forgejo-shared.lock"', pipeline)
        self.assertIn("codex login status", pipeline)
        self.assertIn("cleanup-codex-auth:", pipeline)
        for axis in ["correctness", "security", "performance", "test-coverage", "domain"]:
            self.assertIn(f"review-{axis}:", pipeline)
            self.assertIn(f"review-{axis}/auth.json", pipeline)

    def test_pipeline_jobs_skip_when_resolver_denies_review(self) -> None:
        workflow = yaml.safe_load(
            (REPO_ROOT / ".forgejo" / "workflows" / "codex-pr-review-pipeline.yml").read_text(encoding="utf-8")
        )
        required_guard = "inputs.head_sha != '' && inputs.base_sha != '' && inputs.scripts_ref != ''"
        expected_if = {
            "prepare-context": required_guard,
            "prepare-codex-auth": required_guard,
            "review-correctness": required_guard,
            "review-security": required_guard,
            "review-performance": required_guard,
            "review-test-coverage": required_guard,
            "review-domain": required_guard,
            "tech-lead": f"{required_guard} && always() && !cancelled()",
            "post": required_guard,
            "cleanup-codex-auth": f"{required_guard} && always()",
        }
        for job_name, expected in expected_if.items():
            with self.subTest(job=job_name):
                self.assertEqual(workflow["jobs"][job_name].get("if"), expected)

    def test_forgejo_script_tests_remain(self) -> None:
        self.assertTrue((REPO_ROOT / ".forgejo" / "scripts" / "tests" / "test_forgejo_review.py").exists())

    def test_runner_label_and_toolchain_assumptions(self) -> None:
        text = self.forgejo_text()
        self.assertIn("runs-on: dongwontuna-labs-runner", text)
        self.assertNotIn("runs-on: ubuntu-latest", text)
        self.assertNotIn("setup-node", text)
        self.assertNotIn("setup-rust", text)

    def test_rust_ci_is_preserved_when_present(self) -> None:
        rust_ci = REPO_ROOT / ".forgejo" / "workflows" / "rust-ci.yml"
        if not rust_ci.exists():
            self.skipTest("repository does not have a Rust CI workflow")
        workflow = rust_ci.read_text(encoding="utf-8")
        self.assertIn("cargo fmt --all --check", workflow)
        self.assertIn("cargo clippy --workspace --all-targets --all-features -- -D warnings", workflow)
        self.assertIn("cargo test --workspace --all-features", workflow)
        self.assertIn("cargo build --workspace --all-targets --all-features", workflow)
        self.assertIn("https://code.forgejo.org/actions/cache@v4", workflow)
        self.assertIn("github.event.pull_request.head.repo.full_name == github.repository", workflow)

        parsed = yaml.safe_load(workflow)
        for job_name in ["clippy", "test", "build"]:
            with self.subTest(job=job_name):
                cache_steps = [
                    step
                    for step in parsed["jobs"][job_name]["steps"]
                    if step.get("uses") == "https://code.forgejo.org/actions/cache@v4"
                ]
                self.assertEqual(len(cache_steps), 1)
                cache_step = cache_steps[0]
                self.assertEqual(cache_step["name"], "Restore Cargo dependency cache")
                cache_paths = cache_step["with"]["path"].splitlines()
                self.assertEqual(cache_paths, ["~/.cargo/registry", "~/.cargo/git"])
                self.assertIn("-cargo-deps-", cache_step["with"]["key"])
                self.assertIn("-cargo-deps-", cache_step["with"]["restore-keys"])

    def test_metadata_workflow_checks_forgejo_layout_when_present(self) -> None:
        metadata = REPO_ROOT / ".forgejo" / "workflows" / "metadata.yml"
        if not metadata.exists():
            self.skipTest("repository does not have a metadata workflow")
        workflow = metadata.read_text(encoding="utf-8")
        legacy_dir = "." + "github"
        self.assertIn("test -d .forgejo/scripts", workflow)
        self.assertIn("test -d .codex/agents", workflow)
        self.assertNotIn(legacy_dir, workflow)

    def test_self_repository_urls_do_not_point_to_legacy_remote(self) -> None:
        repo_name = REPO_ROOT.name
        legacy_owner_url = "github.com/DongwonTTuna/" + repo_name
        legacy_org_url = "github.com/DongwonTTuna-Labs/" + repo_name
        paths = [REPO_ROOT / "README.md", REPO_ROOT / "Cargo.toml", REPO_ROOT / "docs" / "PUBLISHING_DISABLED.md"]
        combined = "\n".join(path.read_text(encoding="utf-8") for path in paths if path.exists())
        self.assertNotIn(legacy_owner_url, combined)
        self.assertNotIn(legacy_org_url, combined)


if __name__ == "__main__":
    unittest.main()
