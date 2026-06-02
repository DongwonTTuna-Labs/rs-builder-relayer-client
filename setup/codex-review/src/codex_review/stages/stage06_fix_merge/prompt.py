"""Fix merge prompt."""
from __future__ import annotations
from pathlib import Path
from typing import Any
from codex_review.artifacts import write_text
from codex_review.context.budget import compact_json

def include_final_patch_contract(prompt: str) -> str:
    return prompt + "\nReturn stage06-merged-fix.v1 JSON with one final unified diff patch. No commits, pushes, or comments."

def _section(value: dict[str, Any] | str) -> str:
    return value if isinstance(value, str) else compact_json(value)

def build_fix_merge_prompt(premerge_report: dict[str, Any], collection: dict[str, Any], design_plan: dict[str, Any], chief_decision: dict[str, Any], source_context: dict[str, Any] | str) -> str:
    return include_final_patch_contract(
        "Merge conflicting patches.\n"
        f"Premerge: {_section(premerge_report)}\n"
        f"Collection: {_section(collection)}\n"
        f"Design: {_section(design_plan)}\n"
        f"Chief: {_section(chief_decision)}\n"
        f"Source: {_section(source_context)}\n"
    )

def write_fix_merge_prompt(prompt: str, out_path: str | Path) -> Path:
    return write_text(out_path, prompt)
