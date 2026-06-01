"""Stage01 prompt builder."""
from __future__ import annotations
from pathlib import Path
from typing import Any
from codex_review.artifacts import write_text


def include_axis_specific_focus(axis: str) -> str:
    focuses={"correctness":"bugs, edge cases, state transitions","security":"secrets, auth, injection, unsafe trust boundaries","performance":"unbounded work, memory, network, algorithms","test-coverage":"missing tests and regression coverage","domain":"project-specific correctness and product requirements"}
    return focuses.get(axis, "general review")

def include_changed_line_contract(prompt: str, changed_line_map: dict[str, Any]) -> str:
    return prompt + "\n\nOnly emit findings with file/line on changed RIGHT-side lines. Changed line map:\n" + str(changed_line_map)

def build_axis_prompt(axis: str, pr_context: dict[str, Any], review_context: str, docs_context: str, config: dict[str, Any]) -> str:
    prompt=f"""You are the {axis} reviewer. Focus on {include_axis_specific_focus(axis)}.
Return JSON schema_version stage01-axis-findings.v1 with axis and findings.
Each finding needs finding_id, severity, file, line, root_cause_key, title, summary, recommendation.
Review the PR against its title/body and any OpenSpec context in the repository docs. Treat OpenSpec proposal, design, tasks, and specs as source of truth. Look for missing implementation, spec mismatch, incomplete tasks, and regression risk.

{docs_context}

{review_context}

## PR context
{pr_context}
"""
    return include_changed_line_contract(prompt, pr_context.get("changed_line_map", {}))

def write_axis_prompt(axis: str, prompt: str, out_path: str | Path) -> Path:
    return write_text(out_path, prompt)
