import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "src"))

from codex_review.validators import (
    ContractViolation,
    parse_unified_diff_paths,
    parse_unified_diff_scope_paths,
    require_keys,
    require_schema_version,
)


class ValidatorTests(unittest.TestCase):
    def test_require_schema_version_accepts_exact_match(self):
        require_schema_version({"schema_version": "codex.stage00.resolve_gate.v1"}, "codex.stage00.resolve_gate.v1")

    def test_require_schema_version_fails_closed_on_missing_version(self):
        with self.assertRaises(ContractViolation) as ctx:
            require_schema_version({}, "codex.stage00.resolve_gate.v1")

        self.assertIn("schema_version", str(ctx.exception))

    def test_require_schema_version_fails_closed_on_wrong_version(self):
        with self.assertRaises(ContractViolation) as ctx:
            require_schema_version({"schema_version": "wrong"}, "codex.stage00.resolve_gate.v1")

        self.assertIn("expected codex.stage00.resolve_gate.v1", str(ctx.exception))

    def test_require_keys_reports_missing_keys(self):
        with self.assertRaises(ContractViolation) as ctx:
            require_keys({"a": 1}, ["a", "b", "c"])

        self.assertIn("missing required keys: b, c", str(ctx.exception))

    def test_parse_unified_diff_paths_uses_rename_destination(self):
        patch = """diff --git a/src/old.rs b/src/new.rs
similarity index 80%
rename from src/old.rs
rename to src/new.rs
--- a/src/old.rs
+++ b/src/new.rs
@@ -1 +1 @@
-old
+new
"""

        self.assertEqual(["src/new.rs"], parse_unified_diff_paths(patch))
        self.assertEqual(["src/new.rs", "src/old.rs"], parse_unified_diff_scope_paths(patch))

    def test_parse_unified_diff_paths_uses_copy_destination(self):
        patch = """diff --git a/src/original.rs b/src/copied.rs
similarity index 100%
copy from src/original.rs
copy to src/copied.rs
--- a/src/original.rs
+++ b/src/copied.rs
@@ -1 +1 @@
 value
"""

        self.assertEqual(["src/copied.rs"], parse_unified_diff_paths(patch))
        self.assertEqual(["src/copied.rs", "src/original.rs"], parse_unified_diff_scope_paths(patch))

    def test_parse_unified_diff_paths_uses_deleted_path_for_delete(self):
        patch = """diff --git a/src/deleted.rs b/src/deleted.rs
deleted file mode 100644
--- a/src/deleted.rs
+++ /dev/null
@@ -1 +0,0 @@
-old
"""

        self.assertEqual(["src/deleted.rs"], parse_unified_diff_paths(patch))

    def test_parse_unified_diff_paths_uses_changed_path_for_add(self):
        patch = """diff --git a/src/added.rs b/src/added.rs
new file mode 100644
--- /dev/null
+++ b/src/added.rs
@@ -0,0 +1 @@
+new
"""

        self.assertEqual(["src/added.rs"], parse_unified_diff_paths(patch))

    def test_parse_unified_diff_paths_reads_binary_headers(self):
        patch = """diff --git a/assets/old.bin b/assets/new.bin
Binary files a/assets/old.bin and b/assets/new.bin differ
"""

        self.assertEqual(["assets/new.bin"], parse_unified_diff_paths(patch))
        self.assertEqual(["assets/new.bin", "assets/old.bin"], parse_unified_diff_scope_paths(patch))

    def test_parse_unified_diff_paths_rejects_parent_escape(self):
        with self.assertRaises(ContractViolation) as ctx:
            parse_unified_diff_paths("diff --git a/src/lib.rs b/../outside\n")

        self.assertIn("unsafe patch path", str(ctx.exception))


if __name__ == "__main__":
    unittest.main()
