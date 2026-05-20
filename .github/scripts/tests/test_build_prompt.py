"""Unit tests for build_prompt.load_prompt_body source selection."""

from __future__ import annotations

import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

REPO_ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(REPO_ROOT / ".github" / "scripts"))

import build_prompt  # noqa: E402


def _seed_agent_file(base_dir: Path, axis: str, body: str) -> None:
    agent_dir = base_dir / ".codex" / "agents"
    agent_dir.mkdir(parents=True, exist_ok=True)
    (agent_dir / f"{axis}-reviewer.md").write_text(body, encoding="utf-8")


class LoadPromptBodyTest(unittest.TestCase):
    def _fake_completed(self, returncode: int, stdout: str = "") -> subprocess.CompletedProcess:
        return subprocess.CompletedProcess(
            args=[], returncode=returncode, stdout=stdout, stderr=""
        )

    def test_local_base_dir_file_wins(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp)
            _seed_agent_file(base, "security", "LOCAL BODY")
            with patch.object(subprocess, "run") as run_mock:
                body = build_prompt.load_prompt_body("security", base, "main")
            run_mock.assert_not_called()
        self.assertEqual(body, "LOCAL BODY")

    def test_git_show_fallback_when_no_local_file(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp)
            with patch.object(
                subprocess,
                "run",
                return_value=self._fake_completed(0, "BASE BODY"),
            ) as run_mock:
                body = build_prompt.load_prompt_body("security", base, "main")
            cmd = run_mock.call_args.args[0]
            self.assertIn("origin/main:.codex/agents/security-reviewer.md", cmd)
        self.assertEqual(body, "BASE BODY")

    def test_raises_when_neither_local_nor_git_works(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp)
            with patch.object(
                subprocess, "run", return_value=self._fake_completed(128)
            ):
                with self.assertRaises(SystemExit):
                    build_prompt.load_prompt_body("performance", base, "main")

    def test_empty_base_ref_skips_git_show(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp)
            with patch.object(subprocess, "run") as run_mock:
                with self.assertRaises(SystemExit):
                    build_prompt.load_prompt_body("correctness", base, "")
            run_mock.assert_not_called()


if __name__ == "__main__":
    unittest.main()
