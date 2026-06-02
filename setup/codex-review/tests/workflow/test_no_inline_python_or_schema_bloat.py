from pathlib import Path

ROOT = Path(__file__).resolve().parents[4]
WORKFLOW = ROOT / ".github" / "workflows" / "codex-review-orchestrator.yml"


def test_no_inline_python_or_schema_bloat():
    text = WORKFLOW.read_text(encoding="utf-8")
    assert "python - <<" not in text
    assert "python3 - <<" not in text
    assert "json-schema.org" not in text
    assert text.count("setup/codex-review/bin/codex-review") >= 8
