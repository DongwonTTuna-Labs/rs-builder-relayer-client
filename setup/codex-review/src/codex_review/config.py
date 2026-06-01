"""Config loader and policy accessors."""
from __future__ import annotations

from pathlib import Path
from typing import Any

import yaml

from .errors import ValidationError
from .paths import setup_root

DEFAULT_CONFIG: dict[str, Any] = {
    "base_branch": "main",
    "trusted": {"user": "", "codex_review_authors": []},
    "review": {
        "axes": ["correctness", "security", "performance", "test-coverage", "domain"],
        "max_findings_per_axis": 13,
        "max_inline_comments": 12,
        "max_inline_comments_per_file": 3,
        "require_changed_right_line": True,
    },
    "lifecycle": {
        "max_threads_per_triage": 16,
        "max_root_cause_groups_per_run": 8,
        "char_budget": 24000,
        "terminal_states": ["resolved_by_code", "defer_to_issue", "duplicate_of_issue", "false_positive", "stale_obsolete"],
        "non_terminal_states": ["fix_now", "current_head_keep_open", "needs_human", "blocked_by_conflict"],
    },
    "design": {"require_design_chief": True, "max_clusters": 12, "max_cluster_analysis_batch_size": 4, "fail_on_open_questions": True},
    "autofix": {
        "enabled": False,
        "max_tasks": 4,
        "max_commits": 2,
        "max_files": 8,
        "max_patch_bytes": 120000,
        "allowed_prefixes": ["src/", "tests/", "docs/"],
        "forbidden_prefixes": [".git/", ".github/workflows/", "setup/codex-review/prompts/", "setup/codex-review/schemas/"],
        "forbidden_files": [],
        "dangerous_keywords": ["secret", "private key", "api key", "nonce", "signature", "signing", "public api"],
    },
    "tests": {"default_commands": []},
}


def _deep_merge(base: dict[str, Any], override: dict[str, Any]) -> dict[str, Any]:
    out = dict(base)
    for key, value in override.items():
        if isinstance(value, dict) and isinstance(out.get(key), dict):
            out[key] = _deep_merge(out[key], value)
        else:
            out[key] = value
    return out


def load_config(path: str | Path | None = None) -> dict[str, Any]:
    p = Path(path) if path else setup_root() / "config.yml"
    if not p.exists():
        raise ValidationError(f"missing config file: {p}")
    raw = yaml.safe_load(p.read_text(encoding="utf-8")) or {}
    if not isinstance(raw, dict):
        raise ValidationError("config.yml must contain a YAML object")
    config = _deep_merge(DEFAULT_CONFIG, raw)
    validate_config(config)
    return config


def get_review_axes(config: dict[str, Any]) -> list[str]:
    axes = list(config.get("review", {}).get("axes", []))
    if not axes:
        raise ValidationError("review.axes must not be empty")
    return axes


def get_lifecycle_policy(config: dict[str, Any]) -> dict[str, Any]:
    return dict(config.get("lifecycle", {}))


def get_autofix_policy(config: dict[str, Any]) -> dict[str, Any]:
    return dict(config.get("autofix", {}))


def validate_config(config: dict[str, Any]) -> None:
    for section in ["trusted", "review", "lifecycle", "design", "autofix", "tests"]:
        if section not in config or not isinstance(config[section], dict):
            raise ValidationError(f"config section {section!r} is required")
    axes = config["review"].get("axes")
    if not isinstance(axes, list) or not axes or len(set(axes)) != len(axes):
        raise ValidationError("review.axes must be a non-empty list with unique values")
    auto = config["autofix"]
    for int_key in ["max_tasks", "max_commits", "max_files", "max_patch_bytes"]:
        if int(auto.get(int_key, 0)) < 0:
            raise ValidationError(f"autofix.{int_key} must be non-negative")
    forbidden_prefixes = set(auto.get("forbidden_prefixes", []))
    for prefix in auto.get("allowed_prefixes", []):
        if prefix in forbidden_prefixes:
            raise ValidationError(f"prefix cannot be both allowed and forbidden: {prefix}")
