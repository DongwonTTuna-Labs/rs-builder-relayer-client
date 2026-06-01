"""Stage02 techlead prompt."""
from __future__ import annotations
from pathlib import Path
from typing import Any
from codex_review.artifacts import write_text


def include_decision_action_contract(prompt: str) -> str:
    return prompt + "\nActions: publish_and_fix_now, summary_only_fix_now, defer_to_issue, deny_false_positive, needs_human, needs_design, publish_only, drop_duplicate. Cover every finding_id exactly once. Do not use needs_human as a generic uncertainty escape. For implementable OpenSpec-backed findings, choose needs_design or publish_and_fix_now. Use defer_to_issue for work outside this PR, missing OpenSpec source, fork push limits, or non-executable blockers."

def include_design_required_contract(prompt: str) -> str:
    return prompt + "\nSet needs_design=true for findings that require coordinated edits or autofix."

def build_techlead_prompt(combined_findings: dict[str, Any], pr_context: dict[str, Any], review_context: str, docs_context: str, config: dict[str, Any]) -> str:
    prompt=f"""You are the Codex Review tech lead. Reduce axis findings to actionable decisions.
Return JSON schema_version stage02-techlead-decision.v1.

{docs_context}

{review_context}

Combined findings:
{combined_findings}

PR context:
{pr_context}
"""
    return include_design_required_contract(include_decision_action_contract(prompt))

def write_techlead_prompt(prompt: str, out_path: str | Path) -> Path:
    return write_text(out_path, prompt)
