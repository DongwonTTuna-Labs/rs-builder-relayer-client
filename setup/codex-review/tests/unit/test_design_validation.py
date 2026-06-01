import pytest
from codex_review.stages.stage03_design.coordinate import validate_design_plan
from codex_review.stages.stage03_design.cluster import validate_design_clusters

CFG={"design":{},"autofix":{"dangerous_keywords":[]}}
CTX={"findings":[{"finding_id":"f1"}]}


def test_design_plan_requires_tests_for_findings():
    with pytest.raises(Exception):
        validate_design_plan({"edit_sequence":[{"task_id":"t1"}],"tests":[]}, CTX, CFG)


def test_design_plan_adds_hash():
    out=validate_design_plan({"edit_sequence":[{"task_id":"t1"}],"tests":["pytest"]}, CTX, CFG)
    assert out["plan_hash"]


def test_design_plan_carries_openspec_acceptance_contract():
    ctx={**CTX,"openspec_backed":True,"openspec_context":{"source_summary":["openspec/changes/demo/tasks.md"]}}
    out=validate_design_plan(
        {
            "edit_sequence":[{"task_id":"t1","allowed_files":["src/a.rs"],"acceptance_criteria":["spec passes"]}],
            "tests":["pytest"],
        },
        ctx,
        CFG,
    )
    assert out["openspec_backed"] is True
    assert out["acceptance_criteria"] == ["spec passes"]
    assert out["openspec_sources"] == ["openspec/changes/demo/tasks.md"]


def test_design_plan_rejects_open_questions_field():
    with pytest.raises(Exception, match="does not accept open_questions"):
        validate_design_plan({"edit_sequence":[{"task_id":"t1"}],"tests":["pytest"],"open_questions":["What about API?"]}, CTX, CFG)


def test_clusters_must_cover_inventory():
    with pytest.raises(Exception):
        validate_design_clusters({"clusters":[]}, {"items":[{"finding_id":"f1"}]})
