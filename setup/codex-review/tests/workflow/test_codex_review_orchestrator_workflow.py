import json
import re
import subprocess
import tempfile
import unittest
from pathlib import Path


WORKFLOW_DIR = Path(__file__).resolve().parents[4] / ".github" / "workflows"
WORKFLOW_PATH = WORKFLOW_DIR / "codex-review-orchestrator.yml"
RESOLVE_WORKFLOW_PATH = WORKFLOW_DIR / "resolve-checker.yml"
SETUP_ROOT = Path(__file__).resolve().parents[2]
PROMPTS_DIR = SETUP_ROOT / "prompts"
SCHEMAS_DIR = SETUP_ROOT / "schemas"
INLINE_SCHEMA_MARKER = "output-schema:" + " |"
STAGES = [
    "stage00-resolve-gate",
    "stage01-review",
    "stage02-techlead",
    "stage03-design",
    "stage04-design-chief",
    "stage05-fix-dispatch",
    "stage06-fix-merge",
    "stage06-run-deferred-validation",
    "stage06-finalize-validation",
    "stage07-push",
    "stage08-reentry",
]
MODEL_JOB_RANGES = {
    "stage01-review-model": "stage01-stage02",
    "stage03-design-model": "stage03-design",
    "stage04-design-chief-model": "stage04-stage05",
    "stage05-fix-agent": "stage06-fix-merge",
}


class CodexPrReviewWorkflowTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.workflow_text = WORKFLOW_PATH.read_text(encoding="utf-8")

    def test_uses_one_codex_orchestrator_workflow(self):
        self.assertTrue(WORKFLOW_PATH.exists())
        self.assertFalse(RESOLVE_WORKFLOW_PATH.exists(), "resolve-checker.yml must be folded into the v3 orchestrator")
        self.assertLess(len(self.workflow_text.splitlines()), 900)

    def test_orchestrator_calls_all_stage_cli_contracts(self):
        for stage in STAGES:
            with self.subTest(stage=stage):
                self.assertIn(f"setup/codex-review/bin/codex-review {stage}", self.workflow_text)

    def test_legacy_embedded_review_flow_is_removed(self):
        forbidden = [
            ".github" + "/scripts/post_review.py",
            "combined-findings.json",
            "design-normalize",
            "design-cluster",
            "design-analyze",
            "autofix-plan",
            "resolveReviewThread",
            "security",
        ]
        for token in forbidden:
            with self.subTest(token=token):
                self.assertNotIn(token, self.workflow_text)

    def test_oidc_relay_is_only_used_by_codex_model_jobs(self):
        relay_uses = self.workflow_text.count(
            "uses: DongwonTTuna-Labs/home-server-infra/.github/actions/setup-codex-relay@main"
        )
        codex_uses = self.workflow_text.count("uses: openai/codex-action@")
        id_token_grants = self.workflow_text.count("id-token: write")

        self.assertEqual(4, relay_uses)
        self.assertEqual(relay_uses, codex_uses)
        self.assertEqual(relay_uses, id_token_grants)
        self.assertIn("trusted-actors: DongwonTTuna,codex-reviewer-for-dongwonttuna[bot]", self.workflow_text)

    def test_codex_args_are_normalized_before_action(self):
        self.assertEqual(4, self.workflow_text.count("id: codex-args"))
        self.assertEqual(4, self.workflow_text.count("normalize-codex-args --raw \"$CODEX_ARGS\""))
        self.assertEqual(4, self.workflow_text.count("codex-args: ${{ steps.codex-args.outputs.codex_args }}"))
        self.assertNotIn("codex-args: ${{ steps.relay-token.outputs.codex_args }}", self.workflow_text)

    def test_codex_model_jobs_disable_bwrap_sandbox_on_self_hosted_runner(self):
        self.assertEqual(4, self.workflow_text.count("sandbox: danger-full-access"))

    def test_codex_model_jobs_share_relay_codex_home(self):
        for stage in ("stage01", "stage03", "stage04", "stage05"):
            with self.subTest(stage=stage):
                home = "codex-home: ${{ runner.temp }}/codex-home-" + stage
                self.assertEqual(2, self.workflow_text.count(home))

    def test_codex_output_schemas_are_strict(self):
        self.assertNotIn(INLINE_SCHEMA_MARKER, self.workflow_text)
        schema_files = [
            "stage01-model-review.v1.schema.json",
            "stage03-model-design.v1.schema.json",
            "stage04-model-design-chief.v1.schema.json",
            "stage06-fix-outputs.v1.schema.json",
        ]
        schemas = []
        for schema_file in schema_files:
            with self.subTest(schema=schema_file):
                self.assertIn(f"schema --name {schema_file}", self.workflow_text)
                schemas.append(json.loads((SCHEMAS_DIR / schema_file).read_text(encoding="utf-8")))
        self.assertEqual(4, len(schemas))
        self.assertIn(
            "root_cause_key",
            json.dumps(schemas[0], sort_keys=True),
        )

        def assert_enum_types(node, path="schema"):
            if isinstance(node, dict):
                if "enum" in node:
                    self.assertIn("type", node, path)
                for key, value in node.items():
                    assert_enum_types(value, f"{path}.{key}")
            elif isinstance(node, list):
                for index, value in enumerate(node):
                    assert_enum_types(value, f"{path}[{index}]")

        for schema in schemas:
            assert_enum_types(schema)

    def test_trusted_push_job_has_the_only_contents_write_permission(self):
        write_permissions = re.findall(r"^\s{6}contents: write$", self.workflow_text, flags=re.MULTILINE)

        self.assertEqual(1, len(write_permissions))
        self.assertIn("stage07-trusted-push:", self.workflow_text)
        trusted_push = self.workflow_text.split("stage07-trusted-push:", 1)[1].split("stage08-reentry:", 1)[0]
        self.assertIn("contents: write", trusted_push)
        self.assertIn("setup/codex-review/bin/codex-review stage07-push", trusted_push)
        self.assertNotIn("openai/codex-action", trusted_push)
        self.assertNotIn("id-token: write", trusted_push)
        self.assertNotIn("gh pr merge", trusted_push)

    def test_stage_artifacts_are_uploaded_between_jobs(self):
        artifact_names = [
            "codex-v3-context",
            "codex-v3-stage00",
            "codex-v3-stage00-lifecycle",
            "codex-v3-stage01-model",
            "codex-v3-stage01",
            "codex-v3-stage02",
            "codex-v3-stage03-model",
            "codex-v3-stage03",
            "codex-v3-stage04-model",
            "codex-v3-stage04",
            "codex-v3-stage05",
            "codex-v3-fix-outputs",
            "codex-v3-stage06-initial",
            "codex-v3-stage06-deferred-validation",
            "codex-v3-stage06",
            "codex-v3-stage07",
            "codex-v3-stage08",
        ]
        for name in artifact_names:
            with self.subTest(name=name):
                self.assertIn(f"name: {name}", self.workflow_text)

    def test_pr_metadata_uses_supported_gh_fields(self):
        self.assertNotIn("baseRefOid", self.workflow_text)
        self.assertIn("--json baseRefName,headRefName,headRefOid,headRepository,files", self.workflow_text)
        self.assertIn('gh api "repos/${GITHUB_REPOSITORY}/git/ref/heads/${BASE_REF}"', self.workflow_text)

    def test_trusted_stage_checkouts_use_workflow_ref_for_dispatch_validation(self):
        self.assertNotIn("ref: ${{ steps.meta.outputs.base_sha }}", self.workflow_text)
        self.assertNotIn("ref: ${{ needs.stage00-resolve-gate.outputs.base_sha }}", self.workflow_text)
        self.assertGreaterEqual(self.workflow_text.count("ref: ${{ github.sha }}"), 6)

    def test_stage00_checkout_precedes_artifact_generation(self):
        stage00 = self.workflow_text.split("stage00-resolve-gate:", 1)[1].split("stage01-review-model:", 1)[0]
        self.assertLess(stage00.index("uses: actions/checkout@v6"), stage00.index("id: meta"))

    def test_stage00_collects_review_threads_instead_of_empty_inventory(self):
        stage00 = self.workflow_text.split("stage00-resolve-gate:", 1)[1].split("stage01-review-model:", 1)[0]

        self.assertIn("reviewThreads(first: 100)", stage00)
        self.assertIn("setup/codex-review/bin/codex-review stage00-context", stage00)
        self.assertNotIn('"threads": []', stage00)

    def test_stage00_collects_authoritative_pr_diff_artifact(self):
        stage00 = self.workflow_text.split("stage00-resolve-gate:", 1)[1].split("stage01-review-model:", 1)[0]

        self.assertIn('gh pr diff "$PR_NUMBER" --repo "$GITHUB_REPOSITORY" --patch > artifacts/pr-diff.patch', stage00)
        self.assertIn("artifacts/pr-diff.patch", stage00)

    def test_stage00_lifecycle_consumes_unresolved_thread_gate(self):
        stage00 = self.workflow_text.split("stage00-resolve-gate:", 1)[1].split("stage01-review-model:", 1)[0]

        self.assertIn("setup/codex-review/bin/codex-review stage00-lifecycle", stage00)
        self.assertIn("artifacts/stage00-lifecycle.json", stage00)
        self.assertIn("data = json.load(open(\"artifacts/stage00-lifecycle.json\"", stage00)

    def test_stage01_prompt_consumes_lifecycle_context(self):
        stage01 = self.workflow_text.split("stage01-review-model:", 1)[1].split("stage01-stage02:", 1)[0]
        prompt = (PROMPTS_DIR / "stage01_review" / "prompt.md").read_text(encoding="utf-8")

        self.assertIn("name: codex-v3-stage00-lifecycle", stage01)
        self.assertIn("--stage stage01_review", stage01)
        self.assertIn("--append artifacts/stage00-lifecycle.json", stage01)
        self.assertIn("--append artifacts/thread-inventory.json", stage01)
        self.assertIn("--append artifacts/pr-diff.patch", stage01)
        self.assertIn("--append artifacts/review-request.json", stage01)
        self.assertIn("Use stage00 lifecycle, thread inventory, and artifacts/pr-diff.patch", prompt)
        self.assertIn("Stage07 pushes use checkout GITHUB_TOKEN credentials", prompt)
        self.assertIn("stage08 reentry is produced in the same run", prompt)

    def assert_model_job_uses_trusted_helpers_and_pr_workspace(self, job_name, next_job_name):
        job = self.workflow_text.split(f"{job_name}:", 1)[1].split(f"{next_job_name}:", 1)[0]

        self.assertLess(job.index("ref: ${{ github.sha }}"), job.index("path: workspace"))
        self.assertIn("persist-credentials: false", job.split("path: workspace", 1)[0])
        self.assertIn("repository: ${{ needs.stage00-resolve-gate.outputs.head_repo }}", job)
        self.assertIn("ref: ${{ needs.stage00-resolve-gate.outputs.head_sha }}", job)
        self.assertIn("path: workspace", job)
        self.assertIn("setup/codex-review/bin/codex-review normalize-codex-args", job)
        self.assertIn("workspace", job)
        self.assertNotIn("workspace/.github" + "/scripts/codex-review/src", job)

    def test_codex_model_jobs_use_trusted_helpers_and_pr_workspace(self):
        self.assertIn(
            "CODEX_PYTHONPATH: ${{ github.workspace }}/setup/codex-review/src",
            self.workflow_text,
        )
        self.assertIn("CODEX_REVIEW_ROOT: setup/codex-review", self.workflow_text)
        for job_name, next_job_name in MODEL_JOB_RANGES.items():
            with self.subTest(job=job_name):
                self.assert_model_job_uses_trusted_helpers_and_pr_workspace(job_name, next_job_name)

    def test_stage02_posts_sticky_review_summary_to_pr(self):
        stage02 = self.workflow_text.split("stage01-stage02:", 1)[1].split("stage03-design-model:", 1)[0]

        self.assertIn("issues: write", stage02)
        self.assertIn("pull-requests: write", stage02)
        self.assertIn("setup/codex-review/bin/codex-review stage02-comment", stage02)
        self.assertIn("codex-review-v3-stage02", stage02)
        self.assertIn("issues/${PR_NUMBER}/comments", stage02)
        self.assertIn("issues/comments/$comment_id", stage02)
        self.assertNotIn("openai/codex-action", stage02)

    def test_stage05_prompt_allows_workflow_files_when_allowed(self):
        prompt = (PROMPTS_DIR / "stage05_fix_dispatch" / "prompt.md").read_text(encoding="utf-8")
        self.assertIn("Only modify files listed in allowed_files.", prompt)
        self.assertNotIn("touch workflow files", self.workflow_text)

    def test_stage03_prompt_requires_exact_test_plan_commands(self):
        stage03 = self.workflow_text.split("stage03-design-model:", 1)[1].split("stage03-design:", 1)[0]
        prompt = (PROMPTS_DIR / "stage03_design" / "prompt.md").read_text(encoding="utf-8")

        self.assertIn("--stage stage03_design", stage03)
        self.assertIn("--append artifacts/stage01-review.json", stage03)
        self.assertIn("--append artifacts/stage02-techlead.json", stage03)
        self.assertIn("test_plan entries must be exact commands", prompt)
        self.assertIn("one command per entry", prompt)
        self.assertIn("no markdown, no backticks, no prose", prompt)
        self.assertIn("cargo test --workspace --all-features", prompt)

    def test_stage04_prompt_uses_stage06_stage07_validation_split(self):
        stage04 = self.workflow_text.split("stage04-design-chief-model:", 1)[1].split("stage04-stage05:", 1)[0]
        prompt = (PROMPTS_DIR / "stage04_design_chief" / "prompt.md").read_text(encoding="utf-8")

        self.assertIn("--stage stage04_design_chief", stage04)
        self.assertIn("--append artifacts/stage03-design.json", stage04)
        self.assertIn("Approve exact test_plan commands", prompt)
        self.assertIn("Stage06 separates Stage07 push-safe validation_commands", prompt)
        self.assertIn("deferred_validation_commands", prompt)

    def test_stage05_prompt_requires_final_json_or_conflict(self):
        stage05 = self.workflow_text.split("stage05-fix-agent:", 1)[1].split("stage06-fix-merge:", 1)[0]
        prompt = (PROMPTS_DIR / "stage05_fix_dispatch" / "prompt.md").read_text(encoding="utf-8")

        self.assertIn("--stage stage05_fix_dispatch", stage05)
        self.assertIn("--append artifacts/stage05-fix-dispatch.json", stage05)
        self.assertIn("Return exactly one JSON object.", prompt)
        self.assertIn("No markdown, no prose, no code fences, no logs.", prompt)
        self.assertIn("If a valid JSON output would be too large or uncertain, emit conflict outputs", prompt)
        self.assertIn("Do not emit an empty outputs array.", prompt)

    def test_stage05_action_failure_is_normalized_to_conflict_outputs(self):
        stage05 = self.workflow_text.split("stage05-fix-agent:", 1)[1].split("stage06-fix-merge:", 1)[0]

        self.assertIn("id: fix-agent", stage05)
        self.assertIn("continue-on-error: true", stage05)
        self.assertIn("steps.fix-agent.outcome", stage05)
        self.assertIn("! -s artifacts/fix-outputs.json", stage05)
        self.assertIn("stage05-fallback-fix-outputs", stage05)
        self.assertLess(stage05.index("continue-on-error: true"), stage05.index("name: codex-v3-fix-outputs"))

    def test_trusted_push_revalidates_applied_patch_files(self):
        trusted_push = self.workflow_text.split("stage07-trusted-push:", 1)[1].split("stage08-reentry:", 1)[0]

        self.assertIn("git -C workspace apply --index", trusted_push)
        self.assertIn("git -C workspace diff --cached --check", trusted_push)
        self.assertIn("assert_workspace_changes_match(Path(\"workspace\"), data.get(\"touched_files\") or [])", trusted_push)
        self.assertIn("artifacts/applied-files.txt", trusted_push)

    def test_trusted_push_revalidates_after_validation_before_commit(self):
        trusted_push = self.workflow_text.split("stage07-trusted-push:", 1)[1].split("stage08-reentry:", 1)[0]

        self.assertIn("artifacts/revalidated-files.txt", trusted_push)
        self.assertIn("stage_workspace_files(Path(\"workspace\"), files)", trusted_push)
        self.assertLess(trusted_push.index("stage07-run-validation"), trusted_push.index("artifacts/revalidated-files.txt"))
        self.assertLess(trusted_push.index("artifacts/revalidated-files.txt"), trusted_push.index("git -C workspace commit"))
        self.assertNotIn("git -C workspace add --all\n", trusted_push)

    def test_stage07_requires_stage06_ready_status(self):
        stage06_initial = self.workflow_text.split("stage06-fix-merge:", 1)[1].split("stage06-deferred-validation:", 1)[0]
        stage06_final = self.workflow_text.split("stage06-finalize:", 1)[1].split("stage07-trusted-push:", 1)[0]
        trusted_push = self.workflow_text.split("stage07-trusted-push:", 1)[1].split("stage08-reentry:", 1)[0]

        self.assertIn("status: ${{ steps.result.outputs.status }}", stage06_initial)
        self.assertIn("status: ${{ steps.result.outputs.status }}", stage06_final)
        self.assertIn("print(f\"status={data['status']}\", file=output)", stage06_final)
        self.assertIn("needs: [stage00-resolve-gate, stage06-finalize]", trusted_push)
        self.assertIn("needs.stage06-finalize.outputs.status == 'ready'", trusted_push)

    def test_deferred_validation_job_is_non_write_and_uses_head_sha_workspace(self):
        stage06_deferred = self.workflow_text.split("stage06-deferred-validation:", 1)[1].split("stage06-finalize:", 1)[0]

        self.assertIn("needs: [stage00-resolve-gate, stage06-fix-merge]", stage06_deferred)
        self.assertIn("needs.stage06-fix-merge.outputs.status == 'needs_validation'", stage06_deferred)
        self.assertIn("contents: read", stage06_deferred)
        self.assertNotIn("contents: write", stage06_deferred)
        self.assertLess(stage06_deferred.index("ref: ${{ github.sha }}"), stage06_deferred.index("path: workspace"))
        self.assertIn("persist-credentials: false", stage06_deferred.split("path: workspace", 1)[0])
        self.assertIn("repository: ${{ needs.stage00-resolve-gate.outputs.head_repo }}", stage06_deferred)
        self.assertIn("ref: ${{ needs.stage00-resolve-gate.outputs.head_sha }}", stage06_deferred)
        self.assertIn("path: workspace", stage06_deferred)
        self.assertNotIn("persist-credentials: true", stage06_deferred)
        self.assertIn("name: codex-v3-stage06-initial", stage06_deferred)
        self.assertIn("stage06-run-deferred-validation", stage06_deferred)
        self.assertIn("name: codex-v3-stage06-deferred-validation", stage06_deferred)

    def test_stage06_finalize_consumes_deferred_validation_before_stage07(self):
        stage06_initial = self.workflow_text.split("stage06-fix-merge:", 1)[1].split("stage06-deferred-validation:", 1)[0]
        stage06_final = self.workflow_text.split("stage06-finalize:", 1)[1].split("stage07-trusted-push:", 1)[0]
        trusted_push = self.workflow_text.split("stage07-trusted-push:", 1)[1].split("stage08-reentry:", 1)[0]

        self.assertIn("name: codex-v3-stage06-initial", stage06_initial)
        self.assertIn("needs: [stage00-resolve-gate, stage06-fix-merge, stage06-deferred-validation]", stage06_final)
        self.assertIn("name: codex-v3-stage06-initial", stage06_final)
        self.assertIn("name: codex-v3-stage06-deferred-validation", stage06_final)
        self.assertIn("stage06-finalize-validation", stage06_final)
        self.assertIn("name: codex-v3-stage06", stage06_final)
        self.assertIn("name: codex-v3-stage06", trusted_push)

    def test_trusted_push_uses_trusted_root_scripts_and_workspace_pr_checkout(self):
        trusted_push = self.workflow_text.split("stage07-trusted-push:", 1)[1].split("stage08-reentry:", 1)[0]

        self.assertLess(trusted_push.index("ref: ${{ github.sha }}"), trusted_push.index("path: workspace"))
        self.assertIn("persist-credentials: false", trusted_push.split("path: workspace", 1)[0])
        self.assertIn("ref: ${{ needs.stage00-resolve-gate.outputs.head_ref }}", trusted_push)
        self.assertIn("persist-credentials: true", trusted_push.split("path: workspace", 1)[1].split("uses: actions/download-artifact@v8", 1)[0])
        self.assertIn("setup/codex-review/bin/codex-review stage07-run-validation", trusted_push)
        self.assertNotIn("workspace/.github" + "/scripts/codex-review/src", trusted_push)

    def test_stage07_uses_github_token_push_and_stage08_same_run_artifact(self):
        trusted_push = self.workflow_text.split("stage07-trusted-push:", 1)[1].split("stage08-reentry:", 1)[0]
        stage08 = self.workflow_text.split("stage08-reentry:", 1)[1]

        self.assertIn('git -C workspace push origin "HEAD:${HEAD_REF}"', trusted_push)
        self.assertNotIn("GH_TOKEN:", trusted_push)
        self.assertNotIn("token:", trusted_push)
        self.assertNotIn("secrets.", trusted_push)
        self.assertIn("needs: [stage00-resolve-gate, stage07-trusted-push]", stage08)
        self.assertIn("name: codex-v3-stage08", stage08)
        self.assertIn("if-no-files-found: error", stage08)

    def test_stage07_cached_diff_includes_added_files(self):
        with tempfile.TemporaryDirectory() as tmp:
            repo = Path(tmp)
            subprocess.run(["git", "init"], cwd=repo, check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            subprocess.run(["git", "config", "user.name", "test"], cwd=repo, check=True)
            subprocess.run(["git", "config", "user.email", "test@example.com"], cwd=repo, check=True)
            (repo / "README.md").write_text("base\n", encoding="utf-8")
            subprocess.run(["git", "add", "README.md"], cwd=repo, check=True)
            subprocess.run(["git", "commit", "-m", "base"], cwd=repo, check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            patch = repo / "add.patch"
            patch.write_text(
                "diff --git a/new.txt b/new.txt\n"
                "new file mode 100644\n"
                "index 0000000..ce01362\n"
                "--- /dev/null\n"
                "+++ b/new.txt\n"
                "@@ -0,0 +1 @@\n"
                "+hello\n",
                encoding="utf-8",
            )

            subprocess.run(["git", "apply", "--index", str(patch)], cwd=repo, check=True)
            diff = subprocess.run(
                ["git", "diff", "--cached", "--name-only"],
                cwd=repo,
                check=True,
                text=True,
                stdout=subprocess.PIPE,
            )

        self.assertEqual(["new.txt"], diff.stdout.splitlines())

    def test_trusted_push_runs_validation_commands_before_commit(self):
        trusted_push = self.workflow_text.split("stage07-trusted-push:", 1)[1].split("stage08-reentry:", 1)[0]

        self.assertIn("validation-commands.txt", trusted_push)
        self.assertIn("stage07-run-validation", trusted_push)
        self.assertNotIn("bash -lc \"$command\"", trusted_push)
        self.assertLess(trusted_push.index("validation-commands.txt"), trusted_push.index("git -C workspace commit"))


if __name__ == "__main__":
    unittest.main()
