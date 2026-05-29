import unittest
from pathlib import Path

import yaml


WORKFLOW_PATH = Path(__file__).resolve().parents[2] / "workflows" / "codex-pr-review.yml"
RESOLVE_WORKFLOW_PATH = Path(__file__).resolve().parents[2] / "workflows" / "resolve-checker.yml"


class CodexPrReviewWorkflowTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.workflow_text = WORKFLOW_PATH.read_text(encoding="utf-8")
        cls.workflow = yaml.safe_load(cls.workflow_text)
        cls.resolve_workflow_text = RESOLVE_WORKFLOW_PATH.read_text(encoding="utf-8")
        cls.resolve_workflow = yaml.safe_load(cls.resolve_workflow_text)

    def job(self, name):
        return self.workflow["jobs"][name]

    def resolve_job(self, name):
        return self.resolve_workflow["jobs"][name]

    def step(self, job_name, step_name):
        for step in self.job(job_name)["steps"]:
            if step.get("name") == step_name:
                return step
        self.fail(f"missing step {step_name!r} in job {job_name!r}")

    def step_index(self, job_name, step_name):
        for index, step in enumerate(self.job(job_name)["steps"]):
            if step.get("name") == step_name:
                return index
        self.fail(f"missing step {step_name!r} in job {job_name!r}")

    def test_non_command_issue_comments_do_not_cancel_review_runs(self):
        concurrency = self.workflow["concurrency"]

        self.assertEqual(
            "codex-pr-review-${{ github.event.pull_request.number || github.event.issue.number || github.run_id }}",
            concurrency["group"],
        )
        self.assertEqual(
            "${{ github.event_name != 'issue_comment' || (github.actor == 'DongwonTTuna' && github.triggering_actor == 'DongwonTTuna' && contains(github.event.comment.body, '/codex-review')) }}",
            concurrency["cancel-in-progress"],
        )

    def test_design_coordinate_keeps_write_permissions_out_of_model_job(self):
        permissions = self.job("design-coordinate")["permissions"]

        self.assertEqual("read", permissions["contents"])
        self.assertEqual("read", permissions["pull-requests"])
        self.assertEqual("write", permissions["id-token"])
        self.assertNotIn("issues", permissions)

    def test_design_coordinate_uploads_plan_artifact(self):
        upload = self.step("design-coordinate", "Upload design plan")
        artifact_paths = upload["with"]["path"]

        self.assertLess(
            self.step_index("design-coordinate", "Render design plan"),
            self.step_index("design-coordinate", "Upload design plan"),
        )
        self.assertEqual("actions/upload-artifact@v7", upload["uses"])
        self.assertEqual("codex-design-plan", upload["with"]["name"])
        self.assertIn("artifacts/design-plan.json", artifact_paths)
        self.assertIn("artifacts/design-plan.md", artifact_paths)
        self.assertNotIn(
            "Post sticky design plan",
            [step.get("name") for step in self.job("design-coordinate")["steps"]],
        )

    def test_design_post_uses_downloaded_artifact_and_write_permissions(self):
        job = self.job("design-post")
        permissions = job["permissions"]
        step_names = [step.get("name") for step in job["steps"]]
        checkouts = [step for step in job["steps"] if step.get("uses") == "actions/checkout@v6"]
        checkout = self.step("design-post", "Checkout trusted PR scripts")
        download = self.step("design-post", "Download design plan")
        app_token = self.step("design-post", "Generate App installation token")
        post = self.step("design-post", "Post sticky design plan")

        self.assertEqual(["resolve", "tech-lead", "design-coordinate"], job["needs"])
        self.assertEqual(
            "needs.resolve.outputs.should_run == 'true' && needs.tech-lead.outputs.needs_design == 'true' && needs.design-coordinate.result == 'success' && github.triggering_actor == 'DongwonTTuna'",
            job["if"],
        )
        self.assertEqual("write", permissions["pull-requests"])
        self.assertEqual("read", permissions["contents"])
        self.assertEqual("write", permissions["issues"])
        self.assertNotIn("id-token", permissions)
        self.assertNotIn("Checkout PR head", step_names)
        self.assertEqual([checkout], checkouts)
        self.assertEqual("actions/checkout@v6", checkout["uses"])
        self.assertEqual("${{ needs.resolve.outputs.base_sha }}", checkout["with"]["ref"])
        self.assertEqual("trusted", checkout["with"]["path"])
        self.assertIs(False, checkout["with"]["persist-credentials"])
        self.assertLess(
            self.step_index("design-post", "Download design plan"),
            self.step_index("design-post", "Post sticky design plan"),
        )
        self.assertEqual("actions/download-artifact@v8", download["uses"])
        self.assertEqual("codex-design-plan", download["with"]["name"])
        self.assertEqual("artifacts", download["with"]["path"])
        self.assertLess(
            self.step_index("design-post", "Generate App installation token"),
            self.step_index("design-post", "Post sticky design plan"),
        )
        self.assertEqual("actions/create-github-app-token@v3", app_token["uses"])
        self.assertEqual("${{ secrets.CODEX_APP_ID }}", app_token["with"]["app-id"])
        self.assertEqual("${{ secrets.CODEX_APP_PRIVATE_KEY }}", app_token["with"]["private-key"])
        self.assertEqual("write", app_token["with"]["permission-pull-requests"])
        self.assertEqual("write", app_token["with"]["permission-issues"])
        self.assertNotIn("permission-contents", app_token["with"])
        self.assertEqual("${{ steps.app-token.outputs.token }}", post["env"]["GH_TOKEN"])
        self.assertIn("trusted/.github/scripts/post_review.py post-design-plan", post["run"])
        self.assertIn("artifacts/design-plan.md", post["run"])

    def test_prompt_builders_do_not_prefeed_large_diff(self):
        combined = self.workflow_text + "\n" + self.resolve_workflow_text

        self.assertNotIn("Diff excerpt", combined)
        self.assertNotIn("PR diff excerpt", combined)
        self.assertNotIn("diff_file=", combined)
        self.assertNotIn('git -C workspace diff "$BASE_SHA" "$HEAD_SHA" >', combined)

        prompt_steps = [
            ("review", "Build reviewer prompt"),
            ("tech-lead", "Build tech-lead prompt"),
            ("design-normalize", "Build normalizer prompt"),
            ("design-analyze", "Build cluster analysis prompt"),
            ("design-coordinate", "Build coordinator prompt"),
        ]
        for job_name, step_name in prompt_steps:
            with self.subTest(job=job_name):
                run = self.step(job_name, step_name)["run"]
                self.assertIn('git -C workspace diff --name-only "$BASE_SHA" "$HEAD_SHA"', run)
                self.assertIn("changed_files", run)
                self.assertIn("BASE_SHA=${BASE_SHA}", run)
                self.assertIn("HEAD_SHA=${HEAD_SHA}", run)
                self.assertIn("Use current files as the source of truth", run)

    def test_stale_cleanup_refreshes_design_context_before_normalize(self):
        self.assertEqual(["resolve", "tech-lead"], self.job("design-stale-collect")["needs"])
        self.assertEqual(
            ["resolve", "tech-lead", "design-stale-collect"],
            self.job("design-stale-check")["needs"],
        )
        self.assertEqual(
            ["resolve", "tech-lead", "design-stale-collect", "design-stale-check"],
            self.job("design-stale-apply")["needs"],
        )
        self.assertEqual(["resolve", "tech-lead", "design-stale-apply"], self.job("design-context")["needs"])
        self.assertEqual(["resolve", "tech-lead", "design-context"], self.job("design-normalize")["needs"])
        self.assertIn("needs.design-stale-apply.result == 'success'", self.job("design-context")["if"])
        self.assertIn("needs.design-stale-apply.result == 'skipped'", self.job("design-context")["if"])

        collect = self.step("design-stale-collect", "Collect stale Codex inline comments")
        apply = self.step("design-stale-apply", "Apply stale cleanup decisions")
        context = self.step("design-context", "Build refreshed design review context")
        normalize_download = self.step("design-normalize", "Download refreshed review context")

        self.assertIn("post_review.py collect-resolutions", collect["run"])
        self.assertNotIn("--workspace", collect["run"])
        self.assertIn("post_review.py apply-resolutions", apply["run"])
        self.assertIn("post_review.py build-review-context", context["run"])
        self.assertEqual("codex-design-review-context", normalize_download["with"]["name"])
        self.assertEqual("artifacts", normalize_download["with"]["path"])

    def test_resolve_checker_uses_minimal_batches_and_sticky_summary(self):
        collect = self.resolve_job("collect")
        resolve_check = self.resolve_job("resolve-check")
        apply = self.resolve_job("apply")

        collect_run = next(step["run"] for step in collect["steps"] if step.get("name") == "Collect previous Codex comments")
        prompt_run = next(step["run"] for step in resolve_check["steps"] if step.get("name") == "Build resolve-check prompt")
        apply_run = next(step["run"] for step in apply["steps"] if step.get("name") == "Apply resolution decisions")
        app_token = next(step for step in apply["steps"] if step.get("name") == "Generate App installation token")

        self.assertIn("post_review.py collect-resolutions", collect_run)
        self.assertNotIn("--workspace", collect_run)
        self.assertIn("Use the files in this workspace as the source of truth", prompt_run)
        self.assertIn("BASE_SHA=${BASE_SHA}", prompt_run)
        self.assertIn("HEAD_SHA=${HEAD_SHA}", prompt_run)
        self.assertIn("Treat the batch JSON only as a list of review threads to verify", prompt_run)
        self.assertEqual("write", apply["permissions"]["issues"])
        self.assertEqual("write", apply["permissions"]["pull-requests"])
        self.assertEqual("write", app_token["with"]["permission-issues"])
        self.assertEqual("write", app_token["with"]["permission-pull-requests"])
        self.assertIn("post_review.py apply-resolutions", apply_run)

    def test_review_summary_uses_sticky_issue_comment_permissions(self):
        post = self.job("post")
        app_token = self.step("post", "Generate App installation token")
        post_step = self.step("post", "Post PR review")

        self.assertEqual("write", post["permissions"]["issues"])
        self.assertEqual("write", post["permissions"]["pull-requests"])
        self.assertEqual("write", app_token["with"]["permission-issues"])
        self.assertEqual("write", app_token["with"]["permission-pull-requests"])
        self.assertIn("post_review.py post-current", post_step["run"])


if __name__ == "__main__":
    unittest.main()
