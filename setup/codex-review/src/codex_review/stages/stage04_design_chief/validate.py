"""Design chief decision validation."""
from __future__ import annotations
from pathlib import Path
from typing import Any
from codex_review.artifacts import write_json
from codex_review.errors import ValidationError

VALID_STATUSES={"approved_for_fix","needs_human","rejected_plan","no_fix_needed"}


def validate_fix_policy(policy: dict[str, Any], design_plan: dict[str, Any], config: dict[str, Any]) -> None:
    if not isinstance(policy, dict): raise ValidationError("fix_policy must be an object")
    allowed=policy.get("allowed_files") or policy.get("allowed_prefixes") or config.get("autofix", {}).get("allowed_prefixes")
    if not allowed: raise ValidationError("fix_policy requires allowed_files or allowed_prefixes")
    max_tasks=int(policy.get("max_tasks", config.get("autofix", {}).get("max_tasks", 0)) or 0)
    if max_tasks <= 0: raise ValidationError("fix_policy.max_tasks must be positive")


def validate_task_hints(task_hints: list[dict[str, Any]], design_plan: dict[str, Any]) -> None:
    if task_hints is None: return
    ids={s.get("task_id") or s.get("id") for s in design_plan.get("edit_sequence", [])}
    for hint in task_hints:
        if hint.get("task_id") and ids and hint.get("task_id") not in ids:
            raise ValidationError(f"task_hint references unknown task: {hint.get('task_id')}")


def block_approval_when_human_review_required(decision: dict[str, Any], design_plan: dict[str, Any]) -> None:
    if decision.get("status") == "approved_for_fix" and design_plan.get("requires_human_review"):
        raise ValidationError("cannot approve fix when design plan requires human review")


def _has_execution_blocker(design_plan: dict[str, Any]) -> bool:
    return bool(design_plan.get("requires_human_review") or design_plan.get("execution_blockers"))


def promote_openspec_backed_plan(decision: dict[str, Any], design_plan: dict[str, Any]) -> dict[str, Any]:
    if decision.get("status") != "needs_human":
        return decision
    if not design_plan.get("openspec_backed"):
        return decision
    if _has_execution_blocker(design_plan) or not design_plan.get("edit_sequence"):
        return decision
    out = dict(decision)
    out["status"] = "approved_for_fix"
    out["reason"] = "OpenSpec-backed design plan is executable; needs_human was normalized to approved_for_fix."
    return out


def validate_chief_decision(decision: dict[str, Any], design_plan: dict[str, Any], config: dict[str, Any]) -> dict[str, Any]:
    out=promote_openspec_backed_plan(dict(decision), design_plan)
    out["schema_version"]="stage04-design-chief-decision.v1"
    status=out.get("status")
    if status not in VALID_STATUSES: raise ValidationError(f"invalid chief status: {status}")
    block_approval_when_human_review_required(out, design_plan)
    if status == "approved_for_fix":
        out.setdefault("fix_policy", {})
        # Merge config defaults while letting chief constrain more tightly.
        merged={**config.get("autofix", {}), **out.get("fix_policy", {})}
        out["fix_policy"]=merged
        validate_fix_policy(merged, design_plan, config)
        validate_task_hints(out.get("task_hints", []), design_plan)
    return out


def write_validated_chief_decision(decision: dict[str, Any], out_path: str | Path) -> Path:
    return write_json(out_path, decision, "stage04-design-chief-decision.v1")
