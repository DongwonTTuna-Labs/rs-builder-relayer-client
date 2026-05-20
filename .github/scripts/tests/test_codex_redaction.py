"""Regression tests for codex_redaction.redact()."""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(REPO_ROOT / ".github" / "scripts"))

from codex_redaction import PLACEHOLDER, redact  # noqa: E402


class RedactPatternTest(unittest.TestCase):
    def test_pem_private_key_block(self) -> None:
        text = (
            "header\n"
            "-----BEGIN RSA PRIVATE KEY-----\n"
            "AAAA\nBBBB\n"
            "-----END RSA PRIVATE KEY-----\n"
            "trailer"
        )
        self.assertIn(PLACEHOLDER, redact(text))
        self.assertNotIn("AAAA", redact(text))

    def test_github_pat_token(self) -> None:
        token = "github_pat_" + "x" * 30
        self.assertEqual(redact(f"x {token} y"), f"x {PLACEHOLDER} y")

    def test_ghp_classic_token(self) -> None:
        token = "ghp_" + "y" * 30
        self.assertIn(PLACEHOLDER, redact(token))

    def test_openai_secret_key(self) -> None:
        token = "sk-" + "abc123" * 5
        self.assertEqual(redact(token), PLACEHOLDER)

    def test_bearer_header(self) -> None:
        self.assertIn(
            PLACEHOLDER, redact("Authorization: Bearer " + "z" * 40)
        )

    def test_aws_access_key(self) -> None:
        self.assertIn(PLACEHOLDER, redact("AKIA0123456789ABCDEF"))

    def test_idempotent(self) -> None:
        token = "ghs_" + "k" * 30
        once = redact(f"x {token}")
        twice = redact(once)
        self.assertEqual(once, twice)

    def test_none_safe(self) -> None:
        self.assertEqual(redact(None), "")

    def test_empty_string_safe(self) -> None:
        self.assertEqual(redact(""), "")

    def test_no_secret_unchanged(self) -> None:
        text = "hello world, no secrets here"
        self.assertEqual(redact(text), text)


if __name__ == "__main__":
    unittest.main()
