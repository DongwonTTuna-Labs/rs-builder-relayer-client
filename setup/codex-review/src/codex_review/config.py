"""Configuration loader for Codex review automation."""

from __future__ import annotations

from pathlib import Path

from .constants import PACKAGE_ROOT


def config_path() -> Path:
    return PACKAGE_ROOT / "config.yml"
