from pathlib import Path

ROOT = Path(__file__).resolve().parents[4]


def test_no_separate_resolve_checker_workflow():
    names = [p.name for p in (ROOT / ".github" / "workflows").glob("*")]
    assert "resolve-checker.yml" not in names
    assert "resolve-checker.yaml" not in names


def test_resolve_logic_is_stage00_integrated():
    # Resolve/stage00 now lives in the split review workflow rather than a
    # separate resolve-checker; assert it is integrated wherever review runs.
    workflow = (ROOT / ".github" / "workflows" / "codex-review.yml").read_text(encoding="utf-8")
    assert "stage00 collect" in workflow
    assert "stage00 apply" in workflow
    assert "stage00 route" in workflow
