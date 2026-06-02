import pytest
from codex_review.stages.stage02_techlead.validate import validate_techlead_decision
from codex_review.stages.stage02_techlead.classify import build_review_publication

COMBINED={"findings":[{"finding_id":"f1","file":"src/a.py","line":1,"root_cause_key":"r","title":"T","summary":"S"}]}
CFG={"autofix":{"dangerous_keywords":["secret","auth","nonce"]}}


def test_techlead_requires_finding_coverage():
    with pytest.raises(Exception):
        validate_techlead_decision({"decisions":[]}, COMBINED, CFG)


def test_techlead_validates_action_and_publication():
    decision=validate_techlead_decision({"decisions":[{"finding_id":"f1","action":"publish_only"}]}, COMBINED, CFG)
    pub=build_review_publication(decision, COMBINED, CFG)
    assert pub["inline_comments"][0]["finding_id"] == "f1"


def test_semantic_risk_does_not_block_fix_routing():
    decision=validate_techlead_decision({"decisions":[{"finding_id":"f1","action":"publish_and_fix_now","risk":"secret/auth/nonce handling"}]}, COMBINED, CFG)
    assert decision["status"] == "needs_design"
    assert decision["decisions"][0]["semantic_risk_hints"] == ["auth", "nonce", "secret"]


def test_generic_needs_human_is_normalized_to_design_for_openspec_loop():
    decision=validate_techlead_decision({"decisions":[{"finding_id":"f1","action":"needs_human","reason":"uncertain, but implementable"}]}, COMBINED, CFG)
    assert decision["status"] == "needs_design"
    assert decision["decisions"][0]["action"] == "needs_design"
    assert decision["decisions"][0]["normalized_from"] == "needs_human"


def test_explicit_non_executable_blocker_can_still_stop_for_human():
    decision=validate_techlead_decision({"decisions":[{"finding_id":"f1","action":"needs_human","blocker_type":"secret_required"}]}, COMBINED, CFG)
    assert decision["status"] == "needs_human"
    assert decision["decisions"][0]["action"] == "needs_human"
