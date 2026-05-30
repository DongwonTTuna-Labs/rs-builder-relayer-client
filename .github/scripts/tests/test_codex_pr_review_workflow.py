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
        self.assertIn('expected="c36946ed34d86ecd40b4805e1427c031b34c8a2b5f3018085b45a048e07bcf09"', self.workflow_text)

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


if __name__ == "__main__":
    unittest.main()
