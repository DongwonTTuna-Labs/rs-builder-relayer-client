"""Secret redaction helper shared across Codex PR review scripts.

This module is intentionally repo-local (not a package) so it can be invoked
from sibling scripts via `from codex_redaction import redact, SECRET_PATTERNS`.
"""

from __future__ import annotations

import re
from typing import Pattern

SECRET_PATTERNS: list[Pattern[str]] = [
    re.compile(
        r"-----BEGIN (?:[A-Z0-9 -]+ )?PRIVATE KEY(?: BLOCK)?-----.*?"
        r"-----END (?:[A-Z0-9 -]+ )?PRIVATE KEY(?: BLOCK)?-----",
        re.DOTALL,
    ),
    re.compile(r"github_pat_[A-Za-z0-9_]{20,}"),
    re.compile(r"gh[pousr]_[A-Za-z0-9_]{20,}"),
    re.compile(r"sk-[A-Za-z0-9_-]{20,}"),
    re.compile(r"https://hooks\.slack\.com/services/[A-Za-z0-9/_-]{20,}", re.IGNORECASE),
    re.compile(r"https://discord(?:app)?\.com/api/webhooks/\d+/[A-Za-z0-9._~+/=-]{20,}", re.IGNORECASE),
    re.compile(r"(?i)([?&](?:token|key|secret|signature|sig)=)[A-Za-z0-9._~+/=-]{12,}"),
    re.compile(r"(?i)(bearer\s+)[A-Za-z0-9._~+/=-]{20,}"),
    re.compile(
        r"(?i)\b(?:authorization|proxy-authorization|x-api-key|api-key|private-token|x-auth-token)"
        r"\s*:\s*(?:bearer|basic|token)?\s*[A-Za-z0-9._~+/=-]{12,}"
    ),
    re.compile(r"AKIA[0-9A-Z]{16}"),
]

SECRET_ASSIGNMENT_PATTERNS: list[Pattern[str]] = [
    re.compile(
        r"(?i)(?:api[_-]?key|secret|token|password|passwd|private[_-]?key|client[_-]?secret|database_url|db_url|cloudflare_api_token|webhook_url|webhook|slack_bot)"
        r"['\"]?\s*[:=]\s*['\"][^'\"\s]{12,}['\"]"
    ),
    re.compile(
        r"(?i)\b[A-Z0-9_]*(?:API[_-]?KEY|SECRET|TOKEN|PASSWORD|PASSWD|PRIVATE[_-]?KEY|CLIENT[_-]?SECRET|DATABASE_URL|DB_URL|WEBHOOK_URL|WEBHOOK|SLACK_BOT)"
        r"[A-Z0-9_]*=(?=[A-Za-z0-9._~:/+@=-]*[0-9])[A-Za-z0-9][A-Za-z0-9._~:/+@=-]{11,}"
    ),
    re.compile(r"[a-z][a-z0-9+.-]*://[^/\s:@]+:[^@\s]+@[^\s]+", re.IGNORECASE),
]
UNQUOTED_SECRET_ASSIGNMENT_RE = re.compile(
    r"(?i)\b(?P<key>[A-Z0-9_]*(?:API[_-]?KEY|SECRET|TOKEN|PASSWORD|PASSWD|PRIVATE[_-]?KEY|CLIENT[_-]?SECRET|DATABASE_URL|DB_URL|CLOUDFLARE_API_TOKEN|WEBHOOK_URL|WEBHOOK|SLACK_BOT)[A-Z0-9_]*)"
    r"\s*[:=]\s*(?P<value>[A-Za-z0-9][A-Za-z0-9._~:/+@=-]{11,})"
)

PLACEHOLDER = "<redacted-secret>"

SAFE_REFERENCE_PATTERNS: list[Pattern[str]] = [
    re.compile(r"\$\{\{\s*secrets\.[^}]+\}\}"),
    re.compile(r"\$\{\{\s*vars\.[^}]+\}\}"),
]
SAFE_IDENTIFIER_VALUE_RE = re.compile(r"[A-Za-z_][A-Za-z0-9_]*")
SAFE_DOTTED_REFERENCE_RE = re.compile(r"[A-Za-z_][A-Za-z0-9_]*(?:\.[A-Za-z_][A-Za-z0-9_]*)+")
SAFE_SECRET_HELPER_NAMES = {
    "getenv",
    "get_secret",
    "load_secret",
    "read_secret",
    "require_env",
    "require_secret",
    "require_token",
}


def redact(text: str | None) -> str:
    """Replace any secret-like substring with a fixed placeholder.

    Safe to call repeatedly; idempotent once redacted.
    """
    redacted = text or ""
    for pattern in SECRET_PATTERNS:
        redacted = pattern.sub(PLACEHOLDER, redacted)
    for pattern in SECRET_ASSIGNMENT_PATTERNS:
        redacted = pattern.sub(redact_secret_assignment_match, redacted)
    redacted = UNQUOTED_SECRET_ASSIGNMENT_RE.sub(redact_unquoted_assignment, redacted)
    return redacted


def redact_secret_assignment_match(match: re.Match[str]) -> str:
    segment = match.group(0)
    if is_safe_template_assignment(segment):
        return segment
    return PLACEHOLDER


def is_safe_template_assignment(segment: str) -> bool:
    if "${{" not in segment:
        return False
    value_match = re.search(r"[:=]\s*(?P<value>.+)$", segment, re.DOTALL)
    if not value_match:
        return False
    value = value_match.group("value").strip().strip("'\"")
    for pattern in SAFE_REFERENCE_PATTERNS:
        value = pattern.sub("", value)
    return value.strip().strip(";,\")'") == ""


def redact_unquoted_assignment(match: re.Match[str]) -> str:
    value = match.group("value").rstrip(";,")
    if is_safe_helper_call(match.string, match.end("value"), value):
        return match.group(0)
    if is_safe_unquoted_reference(value):
        return match.group(0)
    return f"{match.group('key')}={PLACEHOLDER}"


def find_unredacted_secret_risks(text: str | None) -> list[str]:
    """Return sanitized snippets that still look like literal credentials."""

    risks: list[str] = []
    redacted = text or ""
    for pattern in SECRET_PATTERNS:
        if not pattern.flags & re.DOTALL:
            continue
        for match in pattern.finditer(redacted):
            risks.append(redact(match.group(0).strip())[:180])
    for line in redacted.splitlines():
        scan_line = line
        for pattern in SAFE_REFERENCE_PATTERNS:
            scan_line = pattern.sub(PLACEHOLDER, scan_line)
        scan_line = scan_line.replace(PLACEHOLDER, "SAFE")
        found = False
        for pattern in SECRET_ASSIGNMENT_PATTERNS:
            if pattern.search(scan_line):
                risks.append(redact(scan_line.strip())[:180])
                found = True
                break
        if found:
            continue
        for pattern in SECRET_PATTERNS:
            if pattern.search(scan_line):
                risks.append(redact(scan_line.strip())[:180])
                found = True
                break
        if found:
            continue
        for match in UNQUOTED_SECRET_ASSIGNMENT_RE.finditer(scan_line):
            value = match.group("value").rstrip(";,")
            if is_safe_helper_call(scan_line, match.end("value"), value):
                continue
            if is_safe_unquoted_reference(value):
                continue
            risks.append(redact(scan_line.strip())[:180])
            break
    return risks


def is_safe_unquoted_reference(value: str) -> bool:
    if value.startswith("$"):
        return True
    if value.casefold() in {"true", "false", "null", "none"}:
        return True
    if SAFE_DOTTED_REFERENCE_RE.fullmatch(value) and len(value) < 64:
        return True
    return False


def is_safe_helper_call(text: str, value_end: int, value: str) -> bool:
    if text[value_end : value_end + 1] != "(":
        return False
    if not SAFE_IDENTIFIER_VALUE_RE.fullmatch(value):
        return False
    return value in SAFE_SECRET_HELPER_NAMES
