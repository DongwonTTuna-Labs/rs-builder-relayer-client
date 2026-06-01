"""Push helper."""
from __future__ import annotations
import os
import subprocess
import urllib.parse
from pathlib import Path
from typing import Any
from codex_review.artifacts import write_json
from codex_review.errors import ValidationError
from codex_review.github.pull_requests import get_current_head_sha
from .safe_subprocess import sanitized_env


def _server_url() -> str:
    return os.environ.get("GITHUB_SERVER_URL", "https://github.com").rstrip("/")


def _authenticated_url(owner: str, repo: str, token: str) -> str:
    if not owner or not repo or not token:
        raise ValidationError("owner, repo and token are required to prepare authenticated remote")
    quoted = urllib.parse.quote(token, safe="")
    return f"{_server_url()}/{owner}/{repo}.git".replace("https://", f"https://x-access-token:{quoted}@", 1)


def prepare_authenticated_remote(repo_path: str | Path, owner: str, repo: str, token: str | None) -> dict[str, Any]:
    if not token:
        raise ValidationError("actual push requires GitHub App installation token")
    proc = subprocess.run(["git", "remote", "get-url", "origin"], cwd=Path(repo_path), capture_output=True, text=True, env=sanitized_env())
    original = proc.stdout.strip() if proc.returncode == 0 else "origin"
    subprocess.run(["git", "remote", "set-url", "origin", _authenticated_url(owner, repo, token)], cwd=Path(repo_path), check=True, env=sanitized_env())
    return {"configured": True, "original_url": original}


def restore_remote(repo_path: str | Path, original_url: str | None) -> None:
    if original_url:
        subprocess.run(["git", "remote", "set-url", "origin", original_url], cwd=Path(repo_path), check=False, env=sanitized_env())


def push_commit(repo_path: str | Path, head_ref: str, owner: str, repo: str, token: str | None) -> dict[str, Any]:
    remote = prepare_authenticated_remote(repo_path, owner, repo, token)
    try:
        proc=subprocess.run(["git","push","origin",f"HEAD:{head_ref}"], cwd=Path(repo_path), capture_output=True, text=True, env=sanitized_env())
        return {"pushed": proc.returncode == 0, "returncode": proc.returncode, "stderr": proc.stderr[-2000:]}
    finally:
        restore_remote(repo_path, remote.get("original_url"))


def verify_pushed_head(owner: str, repo: str, pr_number: int, expected_sha: str, token: str | None) -> bool:
    return get_current_head_sha(owner, repo, pr_number, token) == expected_sha


def write_push_result(result: dict[str, Any], out_path: str | Path) -> Path:
    return write_json(out_path, result, "stage07-push-result.v1")
