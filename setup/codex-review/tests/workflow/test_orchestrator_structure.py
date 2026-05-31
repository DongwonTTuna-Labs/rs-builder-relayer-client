import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[4]
WORKFLOW_PATH = REPO_ROOT / ".github" / "workflows" / "codex-review-orchestrator.yml"
LEGACY_WORKFLOW_NAME = "codex-" + "pr-review.yml"
LEGACY_WORKFLOW_PATH = REPO_ROOT / ".github" / "workflows" / LEGACY_WORKFLOW_NAME
LEGACY_HELPER_ROOT = ".github" + "/scripts/codex-review"
INLINE_SCHEMA_MARKER = "output-schema:" + " |"
SETUP_ROOT = REPO_ROOT / "setup" / "codex-review"


class CodexReviewOrchestratorStructureTests(unittest.TestCase):
    def test_hard_cutover_uses_new_workflow_and_setup_root(self):
        self.assertTrue(WORKFLOW_PATH.exists(), "codex-review-orchestrator.yml must be the only review workflow")
        self.assertFalse(LEGACY_WORKFLOW_PATH.exists(), f"legacy {LEGACY_WORKFLOW_NAME} must be removed")
        workflow = WORKFLOW_PATH.read_text(encoding="utf-8")

        self.assertIn("CODEX_REVIEW_ROOT: setup/codex-review", workflow)
        self.assertIn("CODEX_PYTHONPATH: ${{ github.workspace }}/setup/codex-review/src", workflow)
        self.assertIn("setup/codex-review/bin/codex-review", workflow)
        self.assertNotIn(LEGACY_HELPER_ROOT, workflow)
        self.assertNotIn(LEGACY_WORKFLOW_NAME, workflow)
        self.assertNotIn(INLINE_SCHEMA_MARKER, workflow)

    def test_expected_setup_tree_exists(self):
        required_paths = [
            "README.md",
            "IMPLEMENTATION_ORDER.md",
            "config.yml",
            "bin/codex-review",
            "prompts/common",
            "prompts/stage00_resolve_gate",
            "prompts/stage01_review",
            "prompts/stage02_techlead",
            "prompts/stage03_design",
            "prompts/stage04_design_chief",
            "prompts/stage05_fix_dispatch",
            "prompts/stage06_fix_merge",
            "prompts/stage07_push",
            "schemas/stage01-model-review.v1.schema.json",
            "schemas/stage03-model-design.v1.schema.json",
            "schemas/stage04-model-design-chief.v1.schema.json",
            "schemas/stage06-fix-outputs.v1.schema.json",
            "src/codex_review/cli.py",
            "src/codex_review/config.py",
            "src/codex_review/constants.py",
            "src/codex_review/env.py",
            "src/codex_review/paths.py",
            "src/codex_review/artifacts.py",
            "src/codex_review/schema.py",
            "src/codex_review/github_output.py",
            "src/codex_review/errors.py",
            "src/codex_review/logging.py",
            "src/codex_review/github",
            "src/codex_review/security",
            "src/codex_review/context",
            "src/codex_review/loop",
            "src/codex_review/stages/stage00_resolve_gate",
            "src/codex_review/stages/stage01_review",
            "src/codex_review/stages/stage02_techlead",
            "src/codex_review/stages/stage03_design",
            "src/codex_review/stages/stage04_design_chief",
            "src/codex_review/stages/stage05_fix_dispatch",
            "src/codex_review/stages/stage06_fix_merge",
            "src/codex_review/stages/stage07_push",
            "src/codex_review/stages/stage08_reentry",
            "tests/unit",
            "tests/workflow",
            "tests/fixtures",
        ]
        for relative in required_paths:
            with self.subTest(path=relative):
                self.assertTrue((SETUP_ROOT / relative).exists())


if __name__ == "__main__":
    unittest.main()
