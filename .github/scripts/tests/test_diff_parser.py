"""Unit tests for the diff RIGHT-side line parser (off-by-one regressions)."""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(REPO_ROOT / ".github" / "scripts"))

from post_review_comments import changed_right_lines as changed_right_lines_set  # noqa: E402
from prepare_pr_context import changed_right_lines as changed_right_lines_list  # noqa: E402


SIMPLE_HUNK = """\
@@ -1,3 +1,4 @@
 keep
+added
 keep2
 keep3
"""

MULTI_HUNK = """\
@@ -1,2 +1,3 @@
 a
+b
 c
@@ -10,2 +11,3 @@
 d
+e
 f
"""

DELETION_ONLY_HUNK = """\
@@ -5,3 +5,1 @@
 keep
-gone
-gone2
"""

WITH_PLUS_PLUS_PLUS_HEADER = """\
+++ b/new.py
@@ -0,0 +1,2 @@
+line1
+line2
"""

EMPTY = ""


class DiffParserBothImplsTest(unittest.TestCase):
    def assertSameLines(self, patch: str, expected: list[int]) -> None:
        as_list = changed_right_lines_list(patch)
        as_set = changed_right_lines_set(patch)
        self.assertEqual(as_list, expected, "prepare_pr_context order mismatch")
        self.assertEqual(as_set, set(expected), "post_review_comments set mismatch")

    def test_single_hunk_first_added_line(self) -> None:
        self.assertSameLines(SIMPLE_HUNK, [2])

    def test_multi_hunk_lines(self) -> None:
        self.assertSameLines(MULTI_HUNK, [2, 12])

    def test_deletion_only_hunk_yields_no_added(self) -> None:
        self.assertSameLines(DELETION_ONLY_HUNK, [])

    def test_plus_plus_plus_header_not_counted(self) -> None:
        # `+++ b/new.py` is the file-header marker, not an added line.
        self.assertSameLines(WITH_PLUS_PLUS_PLUS_HEADER, [1, 2])

    def test_empty_patch(self) -> None:
        self.assertSameLines(EMPTY, [])

    def test_only_context_lines(self) -> None:
        patch = "@@ -1,2 +1,2 @@\n a\n b\n"
        self.assertSameLines(patch, [])


if __name__ == "__main__":
    unittest.main()
