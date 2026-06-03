"""Constants and schema-version helpers for Codex Review v3."""
from __future__ import annotations

STAGES = [
    "stage00_resolve_gate",
    "stage01_review",
    "stage02_techlead",
    "stage03_design",
    "stage04_design_chief",
    "stage05_fix_dispatch",
    "stage06_fix_merge",
    "stage07_push",
    "stage08_reentry",
]

MARKERS = {
    "inline_review": "codex-review:inline",
    "lifecycle": "codex-review:lifecycle",
    "sticky_review_summary": "codex-review:review-summary",
    "sticky_resolve_summary": "codex-review:resolve-summary",
    "loop_state": "codex-review:loop-state",
    "deferred_issue": "codex-review:deferred-issue",
    "design_summary": "codex-review:design-summary",
}

_SCHEMA_BY_ARTIFACT = {
    "event-context": "event-context.v1",
    "pr-context": "pr-context.v1",
    "loop-state": "loop-state.v1",
    "shared-review-finding": "shared-review-finding.v1",
    "stage00-thread-inventory": "stage00-thread-inventory.v1",
    "stage00-lifecycle-result": "stage00-lifecycle-result.v1",
    "stage00-gate-result": "stage00-gate-result.v1",
    "stage01-axis-findings": "stage01-axis-findings.v1",
    "stage01-combined-findings": "stage01-combined-findings.v1",
    "stage02-techlead-decision": "stage02-techlead-decision.v1",
    "stage02-review-publication": "stage02-review-publication.v1",
    "stage03-design-context": "stage03-design-context.v1",
    "stage03-design-inventory": "stage03-design-inventory.v1",
    "stage03-design-clusters": "stage03-design-clusters.v1",
    "stage03-cluster-analysis": "stage03-cluster-analysis.v1",
    "stage03-design-plan": "stage03-design-plan.v1",
    "stage04-design-chief-decision": "stage04-design-chief-decision.v1",
    "stage05-fix-task-manifest": "stage05-fix-task-manifest.v1",
    "stage05-fix-agent-result": "stage05-fix-agent-result.v1",
    "stage05-fix-collection-result": "stage05-fix-collection-result.v1",
    "stage06-premerge-report": "stage06-premerge-report.v1",
    "stage06-merged-fix": "stage06-merged-fix.v1",
    "stage07-push-result": "stage07-push-result.v1",
    "stage08-loop-reentry": "stage08-loop-reentry.v1",
}

TERMINAL_LIFECYCLE_STATES = {
    "resolved_by_code",
    "defer_to_issue",
    "duplicate_of_issue",
    "false_positive",
    "stale_obsolete",
}
NON_TERMINAL_LIFECYCLE_STATES = {"fix_now", "current_head_keep_open", "needs_human", "blocked_by_conflict"}
LIFECYCLE_STATES = TERMINAL_LIFECYCLE_STATES | NON_TERMINAL_LIFECYCLE_STATES

TECHLEAD_ACTIONS = {
    "publish_and_fix_now",
    "summary_only_fix_now",
    "defer_to_issue",
    "deny_false_positive",
    "needs_human",
    "needs_design",
    "publish_only",
    "drop_duplicate",
}

SEVERITIES = {"critical", "high", "medium", "low", "info"}


def schema_version_for(artifact_name: str) -> str:
    key = artifact_name.removesuffix(".json").removesuffix(".schema")
    key = key.removesuffix(".v1")
    if key.endswith(".v1.schema"):
        key = key[:-10]
    if key in _SCHEMA_BY_ARTIFACT:
        return _SCHEMA_BY_ARTIFACT[key]
    # Allow callers to pass the exact version string.
    if key.endswith(".v1"):
        return key
    raise KeyError(f"unknown artifact schema version: {artifact_name}")


def marker_names() -> dict[str, str]:
    return dict(MARKERS)


def stage_names() -> list[str]:
    return list(STAGES)
