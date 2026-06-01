"""Path helpers for trusted setup, artifacts, prompts and schemas."""
from __future__ import annotations

import os
from pathlib import Path

from .errors import ValidationError

PACKAGE_DIR = Path(__file__).resolve().parent
SETUP_ROOT = PACKAGE_DIR.parents[1]  # setup/codex-review


def repo_root() -> Path:
    override = os.environ.get("CODEX_REVIEW_REPO_ROOT") or os.environ.get("GITHUB_WORKSPACE")
    if override:
        return Path(override).resolve()
    cwd = Path.cwd().resolve()
    for candidate in [cwd, *cwd.parents]:
        if (candidate / ".github").exists() or (candidate / "setup" / "codex-review").exists():
            return candidate
    return cwd


def setup_root() -> Path:
    return SETUP_ROOT.resolve()


def artifact_root() -> Path:
    value = os.environ.get("CODEX_REVIEW_ARTIFACT_ROOT") or os.environ.get("CODEX_REVIEW_ARTIFACTS")
    path = Path(value).resolve() if value else (repo_root() / "codex-review-artifacts").resolve()
    path.mkdir(parents=True, exist_ok=True)
    return path


def stage_artifact_dir(stage: str) -> Path:
    if ".." in stage or stage.startswith("/"):
        raise ValidationError(f"unsafe stage name: {stage}")
    path = artifact_root() / stage
    path.mkdir(parents=True, exist_ok=True)
    return path


def prompt_path(stage: str, name: str) -> Path:
    path = (setup_root() / "prompts" / stage / name).resolve()
    base = (setup_root() / "prompts").resolve()
    if base not in path.parents and path != base:
        raise ValidationError(f"unsafe prompt path: {stage}/{name}")
    if not path.exists():
        raise FileNotFoundError(path)
    return path


def schema_path(name: str) -> Path:
    filename = name if name.endswith(".schema.json") else f"{name}.schema.json"
    path = (setup_root() / "schemas" / filename).resolve()
    base = (setup_root() / "schemas").resolve()
    if base not in path.parents and path != base:
        raise ValidationError(f"unsafe schema path: {name}")
    if not path.exists():
        raise FileNotFoundError(path)
    return path


def safe_relative_path(path: str | os.PathLike[str]) -> str:
    p = Path(path)
    if p.is_absolute():
        raise ValidationError(f"absolute paths are not allowed: {path}")
    parts = p.as_posix().split("/")
    if any(part in ("..", "") for part in parts):
        raise ValidationError(f"path traversal is not allowed: {path}")
    return p.as_posix()
