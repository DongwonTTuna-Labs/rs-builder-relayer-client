"""Stage modules for Codex Review v3."""
from __future__ import annotations

STAGE_ORDER = [
    "stage00_resolve_gate",
    "stage01_review",
    "stage02_techlead",
    "stage03_design",
    "stage04_design_chief",
    "stage05_fix_dispatch",
    "stage06_fix_merge",
    "stage07_push",
    "stage08_reentry",
]


def stage_order() -> list[str]:
    return list(STAGE_ORDER)
