import pytest
from codex_review.stages.stage04_design_chief.validate import validate_chief_decision
from codex_review.stages.stage04_design_chief.route import route_after_design_chief

CFG={"autofix":{"allowed_prefixes":["src/"],"max_tasks":3}}
PLAN={"edit_sequence":[{"task_id":"t1","files":["src/a.py"]}],"tests":["pytest"],"open_questions":[]}


def test_approval_requires_fix_policy():
    out=validate_chief_decision({"status":"approved_for_fix","fix_policy":{"allowed_files":["src/a.py"],"max_tasks":1}}, PLAN, CFG)
    assert out["status"] == "approved_for_fix"
    assert route_after_design_chief(out)["route"] == "run_stage05"


def test_open_questions_block_approval():
    plan={**PLAN,"open_questions":["What about API?"]}
    with pytest.raises(Exception):
        validate_chief_decision({"status":"approved_for_fix","fix_policy":{"allowed_files":["src/a.py"],"max_tasks":1}}, plan, CFG)
