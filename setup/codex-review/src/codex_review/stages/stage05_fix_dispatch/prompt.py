"""Fix agent prompt."""
from __future__ import annotations
from pathlib import Path
from typing import Any
from codex_review.artifacts import write_text

def include_patch_output_contract(prompt: str) -> str:
    return prompt + "\nReturn JSON schema_version stage05-fix-agent-result.v1. Produce a unified diff patch only; do not commit, push, or comment."

def include_no_safe_fix_contract(prompt: str) -> str:
    return prompt + "\nIf a safe patch cannot be produced within allowed_files, return status no_safe_fix with reason."

def build_fix_agent_prompt(task: dict[str, Any], design_plan: dict[str, Any], chief_decision: dict[str, Any], source_context: dict[str, Any] | str, config: dict[str, Any]) -> str:
    prompt=f"Fix task {task.get('task_id')}: {task.get('summary')}\nAllowed files: {task.get('allowed_files')}\nDesign: {design_plan}\nChief decision: {chief_decision}\nSource context: {source_context}\n"
    return include_no_safe_fix_contract(include_patch_output_contract(prompt))

def write_fix_agent_prompt(task_id: str, prompt: str, out_path: str | Path) -> Path:
    return write_text(out_path, prompt)
