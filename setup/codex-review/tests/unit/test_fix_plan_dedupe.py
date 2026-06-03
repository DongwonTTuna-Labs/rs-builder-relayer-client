from codex_review.stages.stage05_fix_dispatch.plan import plan_fix_tasks


def test_plan_dedupes_duplicate_model_task_ids():
    # The design model emitted two steps with the same task_id touching
    # different files (so they don't merge). plan must not hard-fail; it
    # disambiguates the collision deterministically.
    design_plan = {
        "edit_sequence": [
            {"task_id": "dup", "files": ["a.py"], "summary": "fix a"},
            {"task_id": "dup", "files": ["b.py"], "summary": "fix b"},
        ],
    }
    chief = {"status": "approved_for_fix", "fix_policy": {"max_tasks": 4}}
    manifest = plan_fix_tasks(design_plan, chief, {})
    ids = [t["task_id"] for t in manifest["tasks"]]
    assert ids == ["dup", "dup-2"], ids
    assert len(ids) == len(set(ids))


def test_plan_keeps_distinct_ids_untouched():
    design_plan = {
        "edit_sequence": [
            {"task_id": "one", "files": ["a.py"], "summary": "fix a"},
            {"task_id": "two", "files": ["b.py"], "summary": "fix b"},
        ],
    }
    chief = {"status": "approved_for_fix", "fix_policy": {"max_tasks": 4}}
    manifest = plan_fix_tasks(design_plan, chief, {})
    assert [t["task_id"] for t in manifest["tasks"]] == ["one", "two"]
