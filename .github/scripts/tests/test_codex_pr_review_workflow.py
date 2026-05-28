import unittest
from pathlib import Path

import yaml


WORKFLOW_PATH = Path(__file__).resolve().parents[2] / "workflows" / "codex-pr-review.yml"


class CodexPrReviewWorkflowTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.workflow = yaml.safe_load(WORKFLOW_PATH.read_text(encoding="utf-8"))

    def job(self, name):
        return self.workflow["jobs"][name]

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

    def test_design_coordinate_keeps_write_permissions_out_of_model_job(self):
        permissions = self.job("design-coordinate")["permissions"]

        self.assertEqual("read", permissions["contents"])
        self.assertEqual("read", permissions["pull-requests"])
        self.assertEqual("write", permissions["id-token"])
        self.assertNotIn("issues", permissions)

    def test_design_coordinate_uploads_plan_artifact(self):
        upload = self.step("design-coordinate", "Upload design plan")
        artifact_paths = upload["with"]["path"]

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
        checkout = self.step("design-post", "Checkout trusted PR scripts")
        download = self.step("design-post", "Download design plan")
        post = self.step("design-post", "Post sticky design plan")

        self.assertEqual(["resolve", "tech-lead", "design-coordinate"], job["needs"])
        self.assertIn("needs.design-coordinate.result == 'success'", job["if"])
        self.assertEqual("write", permissions["issues"])
        self.assertEqual("read", permissions["contents"])
        self.assertNotIn("pull-requests", permissions)
        self.assertNotIn("id-token", permissions)
        self.assertNotIn("Checkout PR head", step_names)
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
        self.assertIn("trusted/.github/scripts/post_review.py post-design-plan", post["run"])
        self.assertIn("artifacts/design-plan.md", post["run"])


if __name__ == "__main__":
    unittest.main()
