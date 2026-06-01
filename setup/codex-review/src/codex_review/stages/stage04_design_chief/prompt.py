"""Design chief prompt."""
from __future__ import annotations
from pathlib import Path
from typing import Any
from codex_review.artifacts import write_text

def include_approval_contract(prompt: str) -> str:
    return prompt + "\nReturn status approved_for_fix, needs_human, rejected_plan, or no_fix_needed. If the plan is OpenSpec-backed, has edit_sequence/tests, and has no execution_blockers, return approved_for_fix. Use needs_human only for secret/live credential needs, unsafe fork mutation, missing OpenSpec source, or other non-executable blockers."

def include_fix_policy_requirements(prompt: str) -> str:
    return prompt + "\nIf approved_for_fix, include fix_policy with allowed_files/allowed_prefixes, forbidden_files, max_tasks, max_patch_bytes."

def build_design_chief_prompt(design_plan: dict[str, Any], techlead_decision: dict[str, Any], pr_context: dict[str, Any], config: dict[str, Any]) -> str:
    prompt=f"Review the design plan for safe autofix.\nPlan: {design_plan}\nTechlead: {techlead_decision}\nPR: {pr_context}\nPolicy: {config.get('autofix', {})}\n"
    return include_fix_policy_requirements(include_approval_contract(prompt))

def write_design_chief_prompt(prompt: str, out_path: str | Path) -> Path:
    return write_text(out_path, prompt)
