import pytest
from codex_review.stages.stage02_techlead.validate import validate_techlead_decision
from codex_review.stages.stage02_techlead.classify import build_review_publication

COMBINED={"findings":[{"finding_id":"f1","file":"src/a.py","line":1,"root_cause_key":"r","title":"T","summary":"S"}]}
CFG={"autofix":{"dangerous_keywords":["secret"]}}


def test_techlead_requires_finding_coverage():
    with pytest.raises(Exception):
        validate_techlead_decision({"decisions":[]}, COMBINED, CFG)


def test_techlead_validates_action_and_publication():
    decision=validate_techlead_decision({"decisions":[{"finding_id":"f1","action":"publish_only"}]}, COMBINED, CFG)
    pub=build_review_publication(decision, COMBINED, CFG)
    assert pub["inline_comments"][0]["finding_id"] == "f1"


def test_unsafe_autofix_requires_human_override():
    with pytest.raises(Exception):
        validate_techlead_decision({"decisions":[{"finding_id":"f1","action":"publish_and_fix_now","risk":"secret handling"}]}, COMBINED, CFG)
