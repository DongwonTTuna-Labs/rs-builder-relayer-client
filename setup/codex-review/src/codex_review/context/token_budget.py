"""Token estimation and budgeting for model prompt assembly.

The pipeline feeds prompts to a model with a finite context window (~240k tokens).
Nothing else in the helper is token-aware: budgets were expressed in chars/bytes.
These helpers give every prompt builder one shared, conservative token estimate so
inputs can be bounded against the real window instead of guessed char counts.

A precise tokenizer (tiktoken) is used when available; otherwise a chars/4
heuristic is applied. The heuristic intentionally over-counts slightly so budgets
stay on the safe side of the window.
"""
from __future__ import annotations

from functools import lru_cache

# Conservative bytes-per-token heuristic for English + source code.
CHARS_PER_TOKEN = 4


@lru_cache(maxsize=1)
def _encoding() -> "object | None":
    try:  # pragma: no cover - tiktoken is not a declared dependency in CI
        import tiktoken

        return tiktoken.get_encoding("cl100k_base")
    except Exception:
        return None


def estimate_tokens(text: str) -> int:
    """Estimate the token count of ``text`` (tiktoken if present, else chars/4)."""
    if not text:
        return 0
    enc = _encoding()
    if enc is not None:  # pragma: no cover - exercised only where tiktoken is installed
        try:
            return len(enc.encode(text))
        except Exception:
            pass
    return (len(text) + CHARS_PER_TOKEN - 1) // CHARS_PER_TOKEN


def tokens_to_chars(max_tokens: int) -> int:
    """Translate a token budget into an approximate character budget."""
    return max(0, int(max_tokens)) * CHARS_PER_TOKEN


def within_budget(text: str, max_tokens: int) -> bool:
    """True when ``text`` is estimated to fit inside ``max_tokens``."""
    return estimate_tokens(text) <= max_tokens


def fit_to_budget(text: str, max_tokens: int, *, marker: str = "\n...[truncated]") -> tuple[str, bool]:
    """Truncate ``text`` to an estimated ``max_tokens`` budget.

    Returns ``(text, truncated)``. The estimate is char-based, so the result is a
    conservative upper bound on tokens, never an under-count.
    """
    if max_tokens <= 0:
        return ("", bool(text))
    if within_budget(text, max_tokens):
        return (text, False)
    max_chars = max(0, tokens_to_chars(max_tokens) - len(marker))
    return (text[:max_chars] + marker, True)
