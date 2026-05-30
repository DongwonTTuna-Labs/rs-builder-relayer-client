import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "src"))

from codex_review.validators import ContractViolation, require_keys, require_schema_version


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


if __name__ == "__main__":
    unittest.main()
