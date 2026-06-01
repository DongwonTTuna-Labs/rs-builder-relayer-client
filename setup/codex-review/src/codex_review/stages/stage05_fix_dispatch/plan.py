"""Plan fix tasks from approved design."""
from __future__ import annotations
from pathlib import Path
from typing import Any
from codex_review.artifacts import write_json
from codex_review.errors import ValidationError


def merge_tasks_touching_same_files(tasks: list[dict[str, Any]]) -> list[dict[str, Any]]:
    groups=[]
    for task in tasks:
        files=set(task.get("allowed_files") or task.get("files") or [])
        merged=False
        for g in groups:
            if files & set(g.get("allowed_files", [])):
                g["allowed_files"]=sorted(set(g.get("allowed_files", [])) | files)
                g["steps"].extend(task.get("steps", [task.get("summary")]))
                merged=True; break
        if not merged:
            groups.append({"task_id": task.get("task_id") or task.get("id") or f"fix-{len(groups)+1}", "summary": task.get("summary", "Apply design step"), "allowed_files": sorted(files), "steps": task.get("steps", [task.get("summary")])})
    return groups


def plan_fix_tasks(design_plan: dict[str, Any], chief_decision: dict[str, Any], config: dict[str, Any]) -> dict[str, Any]:
    tasks=[]
    for idx, step in enumerate(design_plan.get("edit_sequence", []), 1):
        files=step.get("files") or step.get("allowed_files") or chief_decision.get("fix_policy", {}).get("allowed_files") or []
        tasks.append({"task_id": step.get("task_id") or step.get("id") or f"fix-{idx}", "summary": step.get("summary") or str(step), "allowed_files": files, "tests": step.get("tests") or design_plan.get("tests", [])})
    if not tasks and chief_decision.get("status") == "approved_for_fix":
        raise ValidationError("approved design has no fix tasks")
    manifest={"schema_version":"stage05-fix-task-manifest.v1","tasks":merge_tasks_touching_same_files(tasks),"plan_hash":design_plan.get("plan_hash"),"fix_policy":chief_decision.get("fix_policy", config.get("autofix", {}))}
    validate_task_manifest(manifest, chief_decision, config)
    return manifest


def validate_task_manifest(manifest: dict[str, Any], chief_decision: dict[str, Any], config: dict[str, Any]) -> None:
    max_tasks=int(chief_decision.get("fix_policy", {}).get("max_tasks", config.get("autofix", {}).get("max_tasks", 4)) or 4)
    tasks=manifest.get("tasks", [])
    if len(tasks)>max_tasks: raise ValidationError(f"too many fix tasks: {len(tasks)} > {max_tasks}")
    ids=[t.get("task_id") for t in tasks]
    if len(ids)!=len(set(ids)): raise ValidationError("duplicate task_id in manifest")
    for t in tasks:
        if not t.get("allowed_files"): raise ValidationError(f"task missing allowed_files: {t.get('task_id')}")


def write_fix_task_manifest(manifest: dict[str, Any], out_path: str | Path) -> Path:
    return write_json(out_path, manifest, "stage05-fix-task-manifest.v1")
