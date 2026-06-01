"""Autofix patch policy validation."""
from __future__ import annotations

import fnmatch
import re
import subprocess
from codex_review.security.subprocess_env import sanitized_env
from pathlib import Path
from typing import Any

from codex_review.errors import PolicyViolation
from codex_review.paths import safe_relative_path
from .redaction import scan_patch_for_secrets

DIFF_PATH_RE = re.compile(r"^diff --git a/(.+?) b/(.+)$")
FORBIDDEN_PATCH_MARKERS = ["Binary files ", "GIT binary patch", "rename from ", "rename to ", "copy from ", "copy to ", "new file mode 120000", "old mode ", "new mode "]


def _normalize(path: str) -> str:
    path = path.strip()
    if path == "/dev/null":
        return path
    if path.startswith("a/") or path.startswith("b/"):
        path = path[2:]
    return safe_relative_path(path)


def parse_patch_touched_files(patch_text: str) -> list[str]:
    touched=set()
    for line in (patch_text or "").splitlines():
        m=DIFF_PATH_RE.match(line)
        if m:
            for p in m.groups():
                if p != "/dev/null": touched.add(_normalize(p))
        elif line.startswith("+++ ") or line.startswith("--- "):
            p=line[4:].strip()
            if p != "/dev/null":
                touched.add(_normalize(p))
    return sorted(touched)


def _matches_any(path: str, patterns: list[str]) -> bool:
    for pattern in patterns or []:
        if not pattern:
            continue
        prefix = pattern if str(pattern).endswith("/") else str(pattern).rstrip("/") + "/"
        if path == str(pattern).rstrip("/") or path.startswith(prefix) or fnmatch.fnmatch(path, str(pattern)):
            return True
    return False


def assert_allowed_paths(touched_files: list[str], policy: dict[str, Any]) -> None:
    allowed = policy.get("allowed_files") or []
    allowed_prefixes = policy.get("allowed_prefixes") or []
    forbidden = set(policy.get("forbidden_files") or [])
    forbidden_prefixes = policy.get("forbidden_prefixes") or []
    max_files = int(policy.get("max_files", 0) or 0)
    if max_files and len(touched_files) > max_files:
        raise PolicyViolation(f"patch touches too many files: {len(touched_files)} > {max_files}")
    for path in touched_files:
        if path in forbidden or _matches_any(path, forbidden_prefixes):
            raise PolicyViolation(f"patch touches forbidden path: {path}")
        if allowed or allowed_prefixes:
            if path not in allowed and not _matches_any(path, allowed_prefixes):
                raise PolicyViolation(f"patch touches path outside allowlist: {path}")


def assert_patch_size_within_limit(patch_text: str, policy: dict[str, Any]) -> None:
    limit=int(policy.get("max_patch_bytes", 0) or 0)
    size=len((patch_text or "").encode("utf-8"))
    if limit and size > limit:
        raise PolicyViolation(f"patch too large: {size} > {limit}")


def assert_no_binary_mode_rename_or_symlink(patch_text: str) -> None:
    for marker in FORBIDDEN_PATCH_MARKERS:
        if marker in (patch_text or ""):
            raise PolicyViolation(f"forbidden patch operation detected: {marker.strip()}")


def assert_no_dangerous_keyword_changes(patch_text: str, policy: dict[str, Any]) -> None:
    keywords=[str(k).lower() for k in policy.get("dangerous_keywords", []) or []]
    for line in (patch_text or "").splitlines():
        if not line.startswith("+") or line.startswith("+++"):
            continue
        lower=line.lower()
        for keyword in keywords:
            if keyword and keyword in lower:
                raise PolicyViolation(f"dangerous keyword added in patch: {keyword}")


def assert_no_public_api_risk(patch_text: str, source_context: dict[str, Any] | None, policy: dict[str, Any]) -> None:
    if policy.get("allow_public_api_changes"):
        return
    risky=["pub fn ", "public ", "export ", "module.exports", "@api", "serde", "signature", "nonce", "signing"]
    for line in (patch_text or "").splitlines():
        if line.startswith("+") and not line.startswith("+++") and any(token in line.lower() for token in risky):
            raise PolicyViolation("public API or signing-related change requires human review")


def git_apply_check(patch_text: str, repo_path: str | Path) -> None:
    if not repo_path:
        return
    p=Path(repo_path)
    if not (p / ".git").exists():
        return
    proc=subprocess.run(["git", "apply", "--check", "-"], input=patch_text, text=True, cwd=p, capture_output=True, env=sanitized_env())
    if proc.returncode != 0:
        raise PolicyViolation(f"git apply --check failed: {proc.stderr.strip()}")


def validate_patch_policy(patch_text: str, policy: dict[str, Any], context: dict[str, Any] | None = None) -> dict[str, Any]:
    context=context or {}
    assert_patch_size_within_limit(patch_text, policy)
    assert_no_binary_mode_rename_or_symlink(patch_text)
    touched=parse_patch_touched_files(patch_text)
    assert_allowed_paths(touched, policy)
    findings=scan_patch_for_secrets(patch_text)
    if findings:
        raise PolicyViolation(f"secret-like material detected in patch: {findings[:3]}")
    assert_no_dangerous_keyword_changes(patch_text, policy)
    assert_no_public_api_risk(patch_text, context.get("source_context"), policy)
    if context.get("repo_path"):
        git_apply_check(patch_text, context["repo_path"])
    return {"ok": True, "touched_files": touched, "patch_bytes": len((patch_text or '').encode('utf-8'))}
