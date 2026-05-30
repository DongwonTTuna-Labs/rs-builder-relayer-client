import json
import re
import unittest
from pathlib import Path


WORKFLOW_DIR = Path(__file__).resolve().parents[2] / "workflows"
WORKFLOW_PATH = WORKFLOW_DIR / "codex-pr-review.yml"
RESOLVE_WORKFLOW_PATH = WORKFLOW_DIR / "resolve-checker.yml"
STAGES = [
    "stage00-resolve-gate",
    "stage01-review",
    "stage02-techlead",
    "stage03-design",
    "stage04-design-chief",
    "stage05-fix-dispatch",
    "stage06-fix-merge",
    "stage07-push",
    "stage08-reentry",
]


class CodexPrReviewWorkflowTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.workflow_text = WORKFLOW_PATH.read_text(encoding="utf-8")

    def test_uses_one_codex_orchestrator_workflow(self):
        self.assertTrue(WORKFLOW_PATH.exists())
        self.assertFalse(RESOLVE_WORKFLOW_PATH.exists(), "resolve-checker.yml must be folded into the v3 orchestrator")
        self.assertLess(len(self.workflow_text.splitlines()), 650)

    def test_orchestrator_calls_all_stage_cli_contracts(self):
        for stage in STAGES:
            with self.subTest(stage=stage):
                self.assertIn(f"python3 -m codex_review.cli {stage}", self.workflow_text)

    def test_legacy_embedded_review_flow_is_removed(self):
        forbidden = [
            ".github/scripts/post_review.py",
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
        self.assertEqual(8, self.workflow_text.count('"additionalProperties":false'))
        self.assertEqual(4, self.workflow_text.count('"schema_version":{"type":"string","enum":['))
        self.assertNotIn('"schema_version":{"const":', self.workflow_text)
        self.assertIn(
            '"required":["finding_id","axis","severity","title","body","root_cause_key","file","line"]',
            self.workflow_text,
        )
        schemas = [
            json.loads(line.strip())
            for line in self.workflow_text.splitlines()
            if line.strip().startswith('{"type":"object"')
        ]
        self.assertEqual(4, len(schemas))

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
        self.assertIn("python3 -m codex_review.cli stage07-push", trusted_push)
        self.assertNotIn("openai/codex-action", trusted_push)
        self.assertNotIn("id-token: write", trusted_push)
        self.assertNotIn("gh pr merge", trusted_push)

    def test_stage_artifacts_are_uploaded_between_jobs(self):
        artifact_names = [
            "codex-v3-context",
            "codex-v3-stage00",
            "codex-v3-stage01-model",
            "codex-v3-stage01",
            "codex-v3-stage02",
            "codex-v3-stage03-model",
            "codex-v3-stage03",
            "codex-v3-stage04-model",
            "codex-v3-stage04",
            "codex-v3-stage05",
            "codex-v3-fix-outputs",
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
        self.assertIn("python3 -m codex_review.cli stage00-context", stage00)
        self.assertNotIn('"threads": []', stage00)

    def test_stage05_prompt_allows_workflow_files_when_allowed(self):
        self.assertIn("Only modify files listed in allowed_files.", self.workflow_text)
        self.assertNotIn("touch workflow files", self.workflow_text)

    def test_stage05_prompt_requires_final_json_or_conflict(self):
        stage05 = self.workflow_text.split("stage05-fix-agent:", 1)[1].split("stage06-fix-merge:", 1)[0]

        self.assertIn("Return exactly one JSON object.", stage05)
        self.assertIn("No markdown, no prose, no code fences, no logs.", stage05)
        self.assertIn("If a valid JSON output would be too large or uncertain, emit conflict outputs", stage05)
        self.assertIn("Do not emit an empty outputs array.", stage05)

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

        self.assertIn("git -C workspace diff --name-only", trusted_push)
        self.assertIn("expected = set(data.get(\"touched_files\") or [])", trusted_push)


if __name__ == "__main__":
    unittest.main()
