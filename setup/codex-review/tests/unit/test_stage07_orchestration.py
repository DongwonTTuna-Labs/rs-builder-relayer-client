from codex_review.stages.stage07_push.orchestrate import run_push_flow


def test_push_flow_no_fix_returns_safe_result(tmp_path):
    result = run_push_flow({"schema_version":"stage06-merged-fix.v1", "status":"no_fix", "patch":""}, {"head_sha":"abc"}, {}, tmp_path, None, dry_run=True)
    assert result["pushed"] is False
    assert result["status"] == "no_fix"
