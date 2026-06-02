from __future__ import annotations

import pytest

from codex_review.stages.stage09_issue_fallback.issue import apply_issue_fallback, build_issue_fallback_plan, render_issue_fallback_body


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


def test_issue_fallback_plan_includes_stage02_deferred_items():
    plan = build_issue_fallback_plan(
        reason="stage02_defer_to_issue",
        pr_context={"owner":"o", "repo":"r", "repository":"o/r", "pr_number":7},
        openspec_context={"present": True, "source_summary": ["openspec/changes/demo/tasks.md"]},
        deferred_items=[{
            "finding_id": "F-1",
            "root_cause_key": "outside-pr-scope",
            "title": "Move unrelated migration to a follow-up",
            "file": "src/lib.rs",
            "line": 12,
            "recommendation": "Track this outside the current PR branch.",
        }],
    )

    assert plan["deferred_count"] == 1
    assert "Deferred items" in plan["body"]
    assert "F-1" in plan["body"]
    assert "outside-pr-scope" in plan["body"]
    assert "stage02_defer_to_issue" in plan["title"]


def test_issue_fallback_actual_apply_requires_app_token():
    plan = {
        "idempotency_key": "abc",
        "title": "Codex review fallback: demo",
        "body": "body",
    }
    with pytest.raises(Exception, match="GitHub App installation token"):
        apply_issue_fallback(plan, {"owner": "o", "repo": "r"}, None, dry_run=False)


def test_issue_fallback_dry_run_remains_explicit():
    plan = {
        "idempotency_key": "abc",
        "title": "Codex review fallback: demo",
        "body": "body",
    }
    result = apply_issue_fallback(plan, {"owner": "o", "repo": "r"}, None, dry_run=True)
    assert result["status"] == "dry_run"


def test_issue_fallback_no_diff_repeat_uses_specific_follow_up():
    plan = build_issue_fallback_plan(
        reason="no_diff_repeat",
        pr_context={"owner":"o", "repo":"r", "repository":"o/r", "pr_number":7},
        openspec_context={"present": True, "source_summary": ["openspec/changes/demo/tasks.md"]},
        attempted_stages=["stage07"],
    )

    assert "non-empty patch" in plan["required_follow_up"]
    assert "no_diff_repeat" in plan["title"]
