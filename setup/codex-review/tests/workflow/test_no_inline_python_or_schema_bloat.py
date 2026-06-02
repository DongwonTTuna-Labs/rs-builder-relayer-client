from _pipeline import all_text


def test_no_inline_python_or_schema_bloat():
    # Invariant holds across every pipeline workflow file.
    text = all_text()
    assert "python - <<" not in text
    assert "python3 - <<" not in text
    assert "json-schema.org" not in text
    assert text.count("setup/codex-review/bin/codex-review") >= 8
