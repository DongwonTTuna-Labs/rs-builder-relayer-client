from pathlib import Path

ROOT = Path(__file__).resolve().parents[4]


def test_no_separate_resolve_checker_workflow():
    names = [p.name for p in (ROOT / ".github" / "workflows").glob("*")]
    assert "resolve-checker.yml" not in names
    assert "resolve-checker.yaml" not in names


def test_resolve_logic_is_resolve_gate_integrated():
    # Resolve/resolve_gate now lives in the split review workflow rather than a
    # separate resolve-checker; assert it is integrated wherever review runs.
    workflow = (ROOT / ".github" / "workflows" / "codex-review.yml").read_text(encoding="utf-8")
    assert "resolve_gate collect" in workflow
    assert "resolve_gate apply" in workflow
    assert "resolve_gate route" in workflow
