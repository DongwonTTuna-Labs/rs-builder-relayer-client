import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


class CliTests(unittest.TestCase):
    def run_cli(self, *args):
        env_path = str(ROOT / "src")
        return subprocess.run(
            [sys.executable, "-m", "codex_review.cli", *args],
            cwd=ROOT,
            env={"PYTHONPATH": env_path},
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )

    def test_stage_command_dry_run_writes_manifest(self):
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / "stage00.json"
            result = self.run_cli("stage00-resolve-gate", "--dry-run", "--out", str(out))

            self.assertEqual("", result.stderr)
            self.assertEqual(0, result.returncode)
            payload = json.loads(out.read_text(encoding="utf-8"))
            self.assertEqual("codex.stage00.resolve_gate.v1", payload["schema_version"])
            self.assertEqual("stage00-resolve-gate", payload["stage"])
            self.assertEqual("dry_run", payload["status"])
            self.assertNotIn("relay_token", json.dumps(payload))

    def test_stage_command_without_dry_run_fails_closed(self):
        result = self.run_cli("stage01-review")

        self.assertNotEqual(0, result.returncode)
        self.assertIn("requires --dry-run in phase 1", result.stderr)

    def test_relay_contract_command_does_not_emit_secret_values(self):
        result = self.run_cli("relay-contract")

        self.assertEqual(0, result.returncode)
        self.assertIn("setup-codex-relay@main", result.stdout)
        self.assertIn("codex_args_sha256", result.stdout)
        self.assertNotIn("sk-clb-", result.stdout)

    def test_normalize_codex_args_command_removes_legacy_landlock(self):
        result = self.run_cli(
            "normalize-codex-args",
            "--raw",
            '["--enable","use_legacy_landlock","--ignore-user-config"]',
        )

        self.assertEqual("", result.stderr)
        self.assertEqual(0, result.returncode)
        self.assertEqual('["--ignore-user-config"]\n', result.stdout)

    def test_schema_command_prints_schema_file(self):
        result = self.run_cli("schema", "--name", "stage01-model-review.v1.schema.json")

        self.assertEqual("", result.stderr)
        self.assertEqual(0, result.returncode)
        payload = json.loads(result.stdout)
        self.assertEqual(["codex.stage01.model_review.v1"], payload["properties"]["schema_version"]["enum"])

    def test_render_prompt_appends_artifacts_in_order(self):
        with tempfile.TemporaryDirectory() as tmp:
            first = Path(tmp) / "first.json"
            second = Path(tmp) / "second.txt"
            out = Path(tmp) / "prompt.md"
            first.write_text('{"first":true}\n', encoding="utf-8")
            second.write_text("second\n", encoding="utf-8")

            result = self.run_cli(
                "render-prompt",
                "--stage",
                "stage03_design",
                "--out",
                str(out),
                "--append",
                str(first),
                "--append",
                str(second),
            )

            self.assertEqual("", result.stderr)
            self.assertEqual(0, result.returncode)
            rendered = out.read_text(encoding="utf-8")
            self.assertIn("Create codex.stage03.model_design.v1 JSON", rendered)
            self.assertLess(rendered.index('{"first":true}'), rendered.index("second"))


if __name__ == "__main__":
    unittest.main()
