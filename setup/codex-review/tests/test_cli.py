import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


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


if __name__ == "__main__":
    unittest.main()
