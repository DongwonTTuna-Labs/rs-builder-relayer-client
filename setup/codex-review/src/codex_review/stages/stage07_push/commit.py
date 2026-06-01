"""Commit helper for trusted push."""
from __future__ import annotations
import subprocess
from pathlib import Path
from typing import Any
from codex_review.errors import ValidationError
from codex_review.security.patch_policy import validate_patch_policy
from .safe_subprocess import sanitized_env


def build_commit_message(merged_fix: dict[str, Any], design_plan_hash: str, old_head_sha: str) -> str:
    return f"Codex Review Autofix\n\nDesign-plan-hash: {design_plan_hash}\nPrevious-head-sha: {old_head_sha}\nMarker: codex-review:autofix\n"


def configure_git_author(policy: dict[str, Any]) -> None:
    name=policy.get("git_author_name", "Codex Review Bot"); email=policy.get("git_author_email", "codex-review@example.invalid")
    subprocess.run(["git","config","user.name",name], check=True, env=sanitized_env())
    subprocess.run(["git","config","user.email",email], check=True, env=sanitized_env())


def create_commit(repo_path: str | Path, message: str) -> str:
    subprocess.run(["git", "add", "-A"], cwd=Path(repo_path), check=True, env=sanitized_env())
    proc = subprocess.run(["git", "commit", "--no-verify", "-F", "-"], input=message, cwd=Path(repo_path), capture_output=True, text=True, env=sanitized_env())
    if proc.returncode != 0:
        raise ValidationError(f"git commit failed: {proc.stderr.strip()}")
    sha = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=Path(repo_path), text=True, env=sanitized_env()).strip()
    return sha


def validate_commit_diff(repo_path: str | Path, commit_sha: str, policy: dict[str, Any]) -> dict[str, Any]:
    patch=subprocess.check_output(["git","show","--format=","--binary",commit_sha], cwd=Path(repo_path), text=True, env=sanitized_env())
    return validate_patch_policy(patch, policy, {})
