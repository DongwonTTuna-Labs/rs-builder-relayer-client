from __future__ import annotations

from codex_review.stages.stage09_issue_fallback.issue import build_issue_fallback_plan, render_issue_fallback_body


def test_issue_fallback_plan_is_idempotent_and_openspec_aware():
    pr_context = {
        "owner": "DongwonTTuna-Labs",
        "repo": "rs-builder-relayer-client",
        "repository": "DongwonTTuna-Labs/rs-builder-relayer-client",
        "pr_number": 41,
        "html_url": "https://github.com/DongwonTTuna-Labs/rs-builder-relayer-client/pull/41",
    }
    openspec_context = {
        "present": True,
        "source_summary": ["openspec/changes/demo/tasks.md"],
    }

    plan = build_issue_fallback_plan(
        reason="missing_openspec_spec",
        pr_context=pr_context,
        openspec_context=openspec_context,
        attempted_stages=["stage03", "stage04"],
    )

    assert plan["schema_version"] == "stage09-issue-fallback.v1"
    assert plan["idempotency_key"]
    assert "missing_openspec_spec" in plan["title"]
    assert "openspec/changes/demo/tasks.md" in plan["body"]
    assert "codex-review:issue-fallback" in plan["body"]


def test_issue_fallback_body_names_required_follow_up():
    body = render_issue_fallback_body(
        idempotency_key="abc123",
        reason="no-diff-repeat",
        pr_context={"pr_number": 41},
        openspec_context={"present": False, "decision": "missing_openspec_spec"},
        attempted_stages=["stage07"],
        required_follow_up="Create an OpenSpec change or adjust the PR body link.",
    )

    assert "Required follow-up" in body
    assert "Create an OpenSpec change" in body
    assert "Source PR: #41" in body
