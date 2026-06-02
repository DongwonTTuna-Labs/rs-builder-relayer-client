"""Bounded JSON serialization shared by model prompt builders.

Aggregator stages used to interpolate Python dicts directly into prompts
(``f"...{some_dict}..."`` => ``str(dict)``), which is token-heavy and hard for the
model to parse. ``compact_json`` is the one serializer every prompt builder should
use: deterministic JSON, optionally bounded by a char or token budget.
"""
from __future__ import annotations

import json
from typing import Any

from .token_budget import tokens_to_chars

TRUNCATION_MARKER = "\n...[truncated]"


def compact_json(value: Any, *, max_tokens: int | None = None, max_chars: int | None = None, indent: int | None = 2) -> str:
    """Serialize ``value`` to deterministic JSON, optionally truncated to a budget.

    ``max_chars`` takes precedence over ``max_tokens`` when both are given. When
    neither is set the full JSON is returned.
    """
    text = json.dumps(value, ensure_ascii=False, indent=indent, sort_keys=True)
    limit = max_chars if max_chars is not None else (tokens_to_chars(max_tokens) if max_tokens is not None else None)
    if limit is not None and len(text) > limit:
        return text[:limit] + TRUNCATION_MARKER
    return text
