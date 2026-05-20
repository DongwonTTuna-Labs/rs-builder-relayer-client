"""Secret redaction helper shared across Codex PR review scripts.

This module is intentionally repo-local (not a package) so it can be invoked
from sibling scripts via `from codex_redaction import redact, SECRET_PATTERNS`.

rs-builder-relayer-client uses an extended pattern list compared to the
baseline (bioden / polymarket-liquidity-farming-rs) because this repository
deals with Ethereum / Polymarket signing flows where 32-byte hex private
keys, wallet mnemonics, and signer key environment variables are common.
A leak of any one of those is catastrophic, so we redact them at the
context-building and comment-rendering boundaries.
"""

from __future__ import annotations

import re
from typing import Pattern

SECRET_PATTERNS: list[Pattern[str]] = [
    # Generic baseline (shared across all Codex PR review repos).
    re.compile(
        r"-----BEGIN [A-Z ]*PRIVATE KEY-----.*?-----END [A-Z ]*PRIVATE KEY-----",
        re.DOTALL,
    ),
    re.compile(r"github_pat_[A-Za-z0-9_]{20,}"),
    re.compile(r"gh[pousr]_[A-Za-z0-9_]{20,}"),
    re.compile(r"sk-[A-Za-z0-9_-]{20,}"),
    re.compile(r"(?i)(bearer\s+)[A-Za-z0-9._~+/=-]{20,}"),
    re.compile(r"AKIA[0-9A-Z]{16}"),
    # rs-builder-relayer-client specific: wallet / signer / polymarket env var
    # assignments that surface raw key material in source or test fixtures.
    re.compile(
        r"(?i)(WALLET_PRIVATE_KEY|SIGNER_PRIVATE_KEY|POLYMARKET_PRIVATE_KEY|RELAYER_PRIVATE_KEY)\s*[:=]\s*['\"]?(0x)?[0-9a-fA-F]{32,128}['\"]?"
    ),
    # 32-byte hex string with 0x prefix (Ethereum private key shape).
    # Anchored to a word boundary to avoid false positives on commit SHAs etc.
    re.compile(r"\b0x[0-9a-fA-F]{64}\b"),
    # BIP-39 style 12 / 24 word mnemonic (very rough; redact aggressively to be safe).
    re.compile(
        r"(?i)(mnemonic|seed[_\s]?phrase)\s*[:=]\s*['\"]?(\w+\s+){11,23}\w+['\"]?"
    ),
]

PLACEHOLDER = "<redacted-secret>"


def redact(text: str | None) -> str:
    """Replace any secret-like substring with a fixed placeholder.

    Safe to call repeatedly; idempotent once redacted.
    """
    redacted = text or ""
    for pattern in SECRET_PATTERNS:
        redacted = pattern.sub(PLACEHOLDER, redacted)
    return redacted
