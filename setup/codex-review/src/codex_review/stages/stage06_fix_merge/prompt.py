"""Fix merge prompt."""
from __future__ import annotations
from pathlib import Path
from typing import Any
from codex_review.artifacts import write_text

def include_final_patch_contract(prompt: str) -> str:
    return prompt + "\nReturn stage06-merged-fix.v1 JSON with one final unified diff patch. No commits, pushes, or comments."

def build_fix_merge_prompt(premerge_report: dict[str, Any], collection: dict[str, Any], design_plan: dict[str, Any], chief_decision: dict[str, Any], source_context: dict[str, Any] | str) -> str:
    return include_final_patch_contract(f"Merge conflicting patches.\nPremerge: {premerge_report}\nCollection: {collection}\nDesign: {design_plan}\nChief: {chief_decision}\nSource: {source_context}\n")

def write_fix_merge_prompt(prompt: str, out_path: str | Path) -> Path:
    return write_text(out_path, prompt)
