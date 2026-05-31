"""Path resolution helpers for the Codex review tool."""

from __future__ import annotations

from pathlib import Path

from .constants import PACKAGE_ROOT, PROMPTS_DIR, SCHEMAS_DIR


def package_root() -> Path:
    return PACKAGE_ROOT


def schema_path(name: str) -> Path:
    return SCHEMAS_DIR / name


def prompt_path(stage: str, name: str = "prompt.md") -> Path:
    return PROMPTS_DIR / stage / name
