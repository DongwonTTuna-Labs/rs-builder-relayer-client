"""Minimal logging helpers for scripts used in GitHub Actions."""

from __future__ import annotations

import sys


def log(message: str) -> None:
    print(message, file=sys.stderr)
