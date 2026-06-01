import json
import pytest

from codex_review.stages.stage05_fix_dispatch.collect import collect_agent_results


def test_collect_agent_results_adds_safe_missing_defaults(tmp_path):
    manifest = {"tasks": [{"task_id":"t1"}, {"task_id":"t2"}]}
    p = tmp_path / "r1.json"
    p.write_text(json.dumps({"schema_version":"stage05-fix-agent-result.v1", "task_id":"t1", "status":"no_safe_fix"}), encoding="utf-8")
    out = collect_agent_results(manifest, [p])
    assert out["ready_for_merge"] is False
    assert out["missing_task_ids"] == ["t2"]
    assert [r["task_id"] for r in out["results"]] == ["t1", "t2"]


def test_collect_agent_results_rejects_unknown_task(tmp_path):
    manifest = {"tasks": [{"task_id":"t1"}]}
    p = tmp_path / "bad.json"
    p.write_text(json.dumps({"schema_version":"stage05-fix-agent-result.v1", "task_id":"other", "status":"no_safe_fix"}), encoding="utf-8")
    with pytest.raises(Exception):
        collect_agent_results(manifest, [p])
