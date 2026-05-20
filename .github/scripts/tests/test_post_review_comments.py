"""Unit tests for marker-only Codex comment identification."""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(REPO_ROOT / ".github" / "scripts"))

from post_review_comments import (  # noqa: E402
    INLINE_MARKER,
    STICKY_MARKER,
    finding_unique_key,
    is_codex_inline,
    is_codex_sticky,
)


class IsCodexInlineTest(unittest.TestCase):
    def test_marker_present_regardless_of_bot_login(self) -> None:
        comment = {
            "user": {"login": "dongwontuna-s-review-bot[bot]"},
            "body": f"{INLINE_MARKER} hello",
        }
        self.assertTrue(is_codex_inline(comment))

    def test_marker_present_with_github_actions_bot(self) -> None:
        comment = {
            "user": {"login": "github-actions[bot]"},
            "body": f"{INLINE_MARKER} hello",
        }
        self.assertTrue(is_codex_inline(comment))

    def test_no_marker_is_not_codex(self) -> None:
        comment = {
            "user": {"login": "dongwontuna-s-review-bot[bot]"},
            "body": "just a regular comment",
        }
        self.assertFalse(is_codex_inline(comment))

    def test_user_field_missing(self) -> None:
        comment = {"body": f"{INLINE_MARKER} hello"}
        self.assertTrue(is_codex_inline(comment))


class IsCodexStickyTest(unittest.TestCase):
    def test_starts_with_sticky_marker(self) -> None:
        comment = {
            "user": {"login": "dongwontuna-s-review-bot[bot]"},
            "body": f"{STICKY_MARKER}\n## Codex AI 리뷰\n…",
        }
        self.assertTrue(is_codex_sticky(comment))

    def test_marker_only_in_middle_not_sticky(self) -> None:
        comment = {"body": f"prefix {STICKY_MARKER}"}
        self.assertFalse(is_codex_sticky(comment))

    def test_empty_body(self) -> None:
        self.assertFalse(is_codex_sticky({"body": ""}))
        self.assertFalse(is_codex_sticky({}))


class FindingUniqueKeyTest(unittest.TestCase):
    def test_same_axis_same_location_same_key(self) -> None:
        f1 = {"file": "a.py", "line": 10, "agent": "security", "title": "title-a"}
        f2 = {"file": "a.py", "line": 10, "agent": "security", "title": "title-b"}
        self.assertEqual(finding_unique_key(f1), finding_unique_key(f2))

    def test_different_axis_different_key(self) -> None:
        f1 = {"file": "a.py", "line": 10, "agent": "security"}
        f2 = {"file": "a.py", "line": 10, "agent": "correctness"}
        self.assertNotEqual(finding_unique_key(f1), finding_unique_key(f2))


if __name__ == "__main__":
    unittest.main()
