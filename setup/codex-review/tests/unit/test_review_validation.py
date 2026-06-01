import pytest
from codex_review.stages.stage01_review.validate import validate_axis_findings
from codex_review.stages.stage01_review.combine import combine_axis_findings

CFG={"review":{"axes":["correctness"],"max_findings_per_axis":3,"require_changed_right_line":True}}


def finding(fid="f1", line=10):
    return {"finding_id":fid,"severity":"medium","file":"src/a.py","line":line,"root_cause_key":"rc1","title":"Bug","summary":"Bug","recommendation":"Fix"}


def test_axis_findings_validate_location_and_shape():
    out=validate_axis_findings("correctness", {"axis":"correctness","findings":[finding()]}, {}, {"src/a.py":[10]}, CFG)
    assert out["finding_count"] == 1


def test_axis_findings_reject_unchanged_line():
    with pytest.raises(Exception):
        validate_axis_findings("correctness", {"axis":"correctness","findings":[finding(line=11)]}, {}, {"src/a.py":[10]}, CFG)


def test_combine_rejects_duplicate_ids():
    a={"axis":"correctness","findings":[finding("f1")]}
    b={"axis":"correctness","findings":[finding("f1")]}
    with pytest.raises(Exception):
        combine_axis_findings([a,b])
