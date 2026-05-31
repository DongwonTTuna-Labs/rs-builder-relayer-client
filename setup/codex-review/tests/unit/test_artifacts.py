import json
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "src"))

from codex_review.artifacts import read_json_artifact, write_json_artifact


class ArtifactTests(unittest.TestCase):
    def test_write_json_artifact_is_deterministic(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "artifact.json"
            write_json_artifact(path, {"b": 2, "a": 1})

            self.assertEqual('{\n  "a": 1,\n  "b": 2\n}\n', path.read_text(encoding="utf-8"))

    def test_read_json_artifact_requires_object_root(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "artifact.json"
            path.write_text("[1, 2, 3]\n", encoding="utf-8")

            with self.assertRaises(ValueError) as ctx:
                read_json_artifact(path)

            self.assertIn("must contain a JSON object", str(ctx.exception))

    def test_write_json_artifact_rejects_non_object_root(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "artifact.json"

            with self.assertRaises(ValueError) as ctx:
                write_json_artifact(path, ["not", "object"])

            self.assertIn("must be a JSON object", str(ctx.exception))


if __name__ == "__main__":
    unittest.main()
