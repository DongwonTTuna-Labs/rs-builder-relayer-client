"""Stage06 fix merge artifact contract logic."""

from __future__ import annotations

from typing import Any

from .stage07 import validation_command_argv
from .validators import (
    parse_unified_diff_paths,
    parse_unified_diff_scope_paths,
    require_keys,
    require_schema_version,
    validate_unified_diff_hunks,
)


FIX_DISPATCH_SCHEMA = "codex.stage05.fix_dispatch.v1"
FIX_OUTPUTS_SCHEMA = "codex.stage06.fix_outputs.v1"
FIX_MERGE_SCHEMA = "codex.stage06.fix_merge.v1"
OUTPUT_STATUSES = {"completed", "conflict"}


def _require_string(value: Any, field: str) -> str:
    text = str(value or "").strip()
    if not text:
        raise ValueError(f"{field} is required")
    return text


def _string_list(value: Any, field: str, *, allow_empty: bool = True) -> list[str]:
    if not isinstance(value, list):
        raise ValueError(f"{field} must be an array")
    items = [_require_string(item, f"{field} entry") for item in value]
    if not allow_empty and not items:
        raise ValueError(f"{field} must be a non-empty array")
    if len(set(items)) != len(items):
        raise ValueError(f"{field} must not contain duplicates")
    return items


def _validate_task(raw_task: Any) -> dict[str, Any]:
    if not isinstance(raw_task, dict):
        raise ValueError("tasks entries must be objects")
    require_keys(raw_task, ["task_id", "allowed_files"])
    return {
        "task_id": _require_string(raw_task.get("task_id"), "task_id"),
        "allowed_files": _string_list(raw_task.get("allowed_files"), "allowed_files", allow_empty=False),
        "test_plan": _string_list(raw_task.get("test_plan") or [], "test_plan"),
    }


def validate_dispatch(payload: dict[str, Any]) -> dict[str, Any]:
    require_schema_version(payload, FIX_DISPATCH_SCHEMA)
    require_keys(payload, ["status", "repository", "pr_number", "base_sha", "head_sha", "task_count", "tasks"])
    if _require_string(payload.get("status"), "status") != "ready":
        raise ValueError("fix dispatch status must be ready")
    raw_tasks = payload.get("tasks")
    if not isinstance(raw_tasks, list) or not raw_tasks:
        raise ValueError("tasks must be a non-empty array")
    tasks = [_validate_task(task) for task in raw_tasks]
    task_ids = [task["task_id"] for task in tasks]
    if len(set(task_ids)) != len(task_ids):
        raise ValueError("duplicate task_id")
    if payload.get("task_count") != len(tasks):
        raise ValueError("task_count does not match tasks")
    return {
        "repository": _require_string(payload.get("repository"), "repository"),
        "pr_number": _require_string(payload.get("pr_number"), "pr_number"),
        "base_sha": _require_string(payload.get("base_sha"), "base_sha"),
        "head_sha": _require_string(payload.get("head_sha"), "head_sha"),
        "tasks": tasks,
        "tasks_by_id": {task["task_id"]: task for task in tasks},
        "task_ids": task_ids,
    }


def build_conflict_fix_outputs(dispatch_payload: dict[str, Any], reason: str) -> dict[str, Any]:
    dispatch = validate_dispatch(dispatch_payload)
    conflict_reason = _require_string(reason, "reason")
    return {
        "schema_version": FIX_OUTPUTS_SCHEMA,
        "outputs": [
            {
                "task_id": task_id,
                "status": "conflict",
                "patch": "",
                "touched_files": [],
                "tests": [],
                "conflict_reason": conflict_reason,
            }
            for task_id in dispatch["task_ids"]
        ],
    }


def _validate_output(raw_output: Any, task: dict[str, Any]) -> dict[str, Any]:
    if not isinstance(raw_output, dict):
        raise ValueError("outputs entries must be objects")
    require_keys(raw_output, ["task_id", "status", "patch", "touched_files", "tests", "conflict_reason"])
    task_id = _require_string(raw_output.get("task_id"), "task_id")
    status = _require_string(raw_output.get("status"), f"{task_id} status")
    if status not in OUTPUT_STATUSES:
        raise ValueError(f"{task_id} has unknown status: {status}")
    touched_files = _string_list(
        raw_output.get("touched_files"),
        f"{task_id} touched_files",
        allow_empty=status == "conflict",
    )
    patch = str(raw_output.get("patch") or "")
    if patch.strip():
        validate_unified_diff_hunks(patch)
        patch_files = parse_unified_diff_paths(patch)
        patch_scope_files = parse_unified_diff_scope_paths(patch)
    else:
        patch_files = []
        patch_scope_files = []
    allowed_files = set(task["allowed_files"])
    for path in sorted(set(touched_files) | set(patch_scope_files)):
        if path not in allowed_files:
            raise ValueError(f"{task_id} touched file is not allowed: {path}")
    if sorted(touched_files) != patch_files:
        raise ValueError(f"{task_id} patch files must match touched_files")
    conflict_reason = str(raw_output.get("conflict_reason") or "").strip()
    if status == "completed" and not patch.strip():
        raise ValueError(f"{task_id} completed output requires patch")
    if status == "completed" and not patch_files:
        raise ValueError(f"{task_id} completed output requires file-bearing patch")
    if status == "conflict" and not conflict_reason:
        raise ValueError(f"{task_id} conflict output requires conflict_reason")
    return {
        "task_id": task_id,
        "status": status,
        "patch": patch,
        "touched_files": touched_files,
        "tests": _string_list(raw_output.get("tests"), f"{task_id} tests"),
        "conflict_reason": conflict_reason,
    }


def validate_fix_outputs(payload: dict[str, Any], dispatch: dict[str, Any]) -> list[dict[str, Any]]:
    require_schema_version(payload, FIX_OUTPUTS_SCHEMA)
    require_keys(payload, ["outputs"])
    raw_outputs = payload.get("outputs")
    if not isinstance(raw_outputs, list):
        raise ValueError("outputs must be an array")
    output_ids = [str(output.get("task_id") or "") for output in raw_outputs if isinstance(output, dict)]
    if set(output_ids) != set(dispatch["task_ids"]) or len(output_ids) != len(set(output_ids)):
        raise ValueError("fix output task ids must match dispatch task ids")
    return [_validate_output(output, dispatch["tasks_by_id"][str(output["task_id"])]) for output in raw_outputs]


def _partition_stage07_validation_commands(commands: list[str]) -> tuple[list[str], list[str]]:
    validation_commands: list[str] = []
    deferred_validation_commands: list[str] = []
    for command in commands:
        try:
            validation_command_argv(command)
        except ValueError:
            deferred_validation_commands.append(command)
        else:
            validation_commands.append(command)
    if not validation_commands:
        validation_commands.append("git diff --check")
    return validation_commands, deferred_validation_commands


def build_fix_merge_result(dispatch_payload: dict[str, Any], fix_outputs_payload: dict[str, Any]) -> dict[str, Any]:
    dispatch = validate_dispatch(dispatch_payload)
    outputs = validate_fix_outputs(fix_outputs_payload, dispatch)
    conflicts = [
        f"{output['task_id']}: {output['conflict_reason']}"
        for output in outputs
        if output["status"] == "conflict"
    ]
    completed_file_tasks: dict[str, list[str]] = {}
    for output in outputs:
        if output["status"] != "completed":
            continue
        for path in output["touched_files"]:
            completed_file_tasks.setdefault(path, []).append(output["task_id"])
    conflicts.extend(
        f"multiple completed outputs touch the same file: {path} ({', '.join(task_ids)})"
        for path, task_ids in sorted(completed_file_tasks.items())
        if len(task_ids) > 1
    )
    touched_files = sorted({path for output in outputs for path in output["touched_files"]})
    candidate_patch = "\n".join(
        output["patch"].rstrip("\n")
        for output in outputs
        if output["status"] == "completed"
    )
    raw_validation_commands = list(
        dict.fromkeys(
            command
            for task in dispatch["tasks"]
            for command in task["test_plan"]
        )
    )
    raw_validation_commands = list(
        dict.fromkeys(
            [
                *raw_validation_commands,
                *[
                    command
                    for output in outputs
                    for command in output["tests"]
                ],
            ]
        )
    )
    validation_commands, deferred_validation_commands = _partition_stage07_validation_commands(
        raw_validation_commands
    )
    return {
        "schema_version": FIX_MERGE_SCHEMA,
        "stage": "stage06-fix-merge",
        "status": "conflict" if conflicts else "ready",
        "can_continue": not conflicts,
        "push_allowed": False,
        "repository": dispatch["repository"],
        "pr_number": dispatch["pr_number"],
        "base_sha": dispatch["base_sha"],
        "head_sha": dispatch["head_sha"],
        "task_ids": dispatch["task_ids"],
        "touched_files": touched_files,
        "validation_commands": validation_commands,
        "deferred_validation_commands": deferred_validation_commands,
        "candidate_patch": "" if conflicts else candidate_patch + "\n",
        "conflicts": conflicts,
    }
