"""Shared constants for Codex review automation."""

from __future__ import annotations

from pathlib import Path


PACKAGE_ROOT = Path(__file__).resolve().parents[2]
SCHEMAS_DIR = PACKAGE_ROOT / "schemas"
PROMPTS_DIR = PACKAGE_ROOT / "prompts"
