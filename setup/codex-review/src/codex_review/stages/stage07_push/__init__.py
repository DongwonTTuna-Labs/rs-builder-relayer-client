"""Stage07 trusted push artifact contract logic."""

from __future__ import annotations

import shlex
import subprocess
from pathlib import Path, PurePosixPath
from typing import Any

from ...schema import require_keys, require_schema_version


FIX_MERGE_SCHEMA = "codex.stage06.fix_merge.v1"
TRUSTED_PUSH_SCHEMA = "codex.stage07.trusted_push.v1"
PUSH_SCHEMA = "codex.stage07.push.v1"
RUBY_WORKFLOW_YAML_CHECK = (
    'require "yaml"; Dir[".github/workflows/*.yml"].each { |p| YAML.load_file(p) }; puts "yaml ok"'
)
EXACT_VALIDATION_COMMANDS = {
    ("git", "diff", "--check"),
    ("cargo", "fmt", "--all", "--check"),
    ("actionlint", ".github/workflows/codex-review-orchestrator.yml"),
    ("actionlint", "-ignore", 'label "dongwontuna-labs-runner" is unknown', ".github/workflows/codex-review-orchestrator.yml"),
    ("ruby", "-e", RUBY_WORKFLOW_YAML_CHECK),
}


def _require_string(value: Any, field: str) -> str:
    text = str(value or "").strip()
    if not text:
        raise ValueError(f"{field} is required")
    return text


def _require_sha(value: Any, field: str, length: int) -> str:
    text = _require_string(value, field)
    if len(text) != length or any(char not in "0123456789abcdef" for char in text.lower()):
        raise ValueError(f"{field} must be a sha{length * 4} hex digest")
    return text


def _string_list(value: Any, field: str, *, allow_empty: bool = True) -> list[str]:
    if not isinstance(value, list):
        raise ValueError(f"{field} must be an array")
    items = [_require_string(item, f"{field} entry") for item in value]
    if not allow_empty and not items:
        raise ValueError(f"{field} must be a non-empty array")
    if len(set(items)) != len(items):
        raise ValueError(f"{field} must not contain duplicates")
    return items


def validate_fix_merge(payload: dict[str, Any]) -> dict[str, Any]:
    require_schema_version(payload, FIX_MERGE_SCHEMA)
    require_keys(
        payload,
        [
            "status",
            "can_continue",
            "repository",
            "pr_number",
            "head_sha",
            "touched_files",
            "validation_commands",
            "deferred_validation_commands",
            "candidate_patch",
        ],
    )
    if _require_string(payload.get("status"), "status") != "ready" or payload.get("can_continue") is not True:
        raise ValueError("stage07 requires ready fix_merge")
    deferred_validation_commands = _string_list(payload.get("deferred_validation_commands"), "deferred_validation_commands")
    if deferred_validation_commands:
        raise ValueError("stage07 requires no deferred validation commands")
    if not str(payload.get("candidate_patch") or "").strip():
        raise ValueError("candidate_patch is required")
    return {
        "repository": _require_string(payload.get("repository"), "repository"),
        "pr_number": _require_string(payload.get("pr_number"), "pr_number"),
        "head_sha": _require_sha(payload.get("head_sha"), "head_sha", 40),
        "touched_files": _string_list(payload.get("touched_files"), "touched_files", allow_empty=False),
        "validation_commands": _string_list(payload.get("validation_commands"), "validation_commands", allow_empty=False),
    }


def _unsupported_validation_command(command: str) -> None:
    raise ValueError(f"unsupported validation command: {command}")


def validation_command_argv(command: str) -> list[str]:
    try:
        argv = shlex.split(_require_string(command, "validation command"), posix=True)
    except ValueError as exc:
        _unsupported_validation_command(command)
        raise AssertionError("unreachable") from exc
    if tuple(argv) in EXACT_VALIDATION_COMMANDS:
        return argv
    _unsupported_validation_command(command)
    raise AssertionError("unreachable")


def validation_commands_argv(commands: list[str]) -> list[list[str]]:
    return [validation_command_argv(command) for command in commands]


def run_validation_commands(commands: list[str], workspace: Path) -> None:
    argv_commands = validation_commands_argv(commands)
    for argv in argv_commands:
        subprocess.run(argv, cwd=workspace, check=True)


def _git_output_lines(workspace: Path, args: list[str]) -> list[str]:
    result = subprocess.run(["git", *args], cwd=workspace, check=True, text=True, stdout=subprocess.PIPE)
    return [line for line in result.stdout.splitlines() if line]


def workspace_changed_files(workspace: Path) -> list[str]:
    changed = set(_git_output_lines(workspace, ["diff", "--name-only"]))
    changed.update(_git_output_lines(workspace, ["diff", "--cached", "--name-only"]))
    changed.update(_git_output_lines(workspace, ["ls-files", "--others", "--exclude-standard"]))
    return sorted(changed)


def assert_workspace_changes_match(workspace: Path, expected_files: list[str]) -> list[str]:
    """Validate staged changes against Stage06/Stage07 post-apply path names.

    The expected file list must use the same contract as parse_unified_diff_paths:
    rename/copy destinations, deleted paths for deletes, and changed paths for
    add/modify/binary patches.
    """
    expected = sorted(set(expected_files))
    staged = _git_output_lines(workspace, ["diff", "--cached", "--name-only"])
    unstaged = _git_output_lines(workspace, ["diff", "--name-only"])
    untracked = _git_output_lines(workspace, ["ls-files", "--others", "--exclude-standard"])
    if staged != expected or unstaged or untracked:
        raise ValueError(
            "workspace changed files mismatch: "
            f"expected staged {expected}, got staged {staged}, unstaged {unstaged}, untracked {untracked}"
        )
    return staged


def stage_workspace_files(workspace: Path, files: list[str]) -> None:
    paths = sorted(set(files))
    if not paths:
        raise ValueError("files must be non-empty")
    for path in paths:
        parsed = PurePosixPath(path)
        if parsed.is_absolute() or ".." in parsed.parts:
            raise ValueError(f"invalid workspace path: {path}")
    subprocess.run(["git", "add", "--all", "--", *paths], cwd=workspace, check=True)


def validate_trusted_push(payload: dict[str, Any]) -> dict[str, Any]:
    require_schema_version(payload, TRUSTED_PUSH_SCHEMA)
    require_keys(
        payload,
        [
            "trusted_job",
            "trusted_ref",
            "actor",
            "target_branch",
            "pushed_head_sha",
            "applied_patch_sha256",
            "merge_commit",
            "pr_merged",
        ],
    )
    if payload.get("trusted_job") is not True:
        raise ValueError("trusted_job must be true")
    if payload.get("pr_merged") is True:
        raise ValueError("stage07 must not merge PRs")
    return {
        "trusted_ref": _require_string(payload.get("trusted_ref"), "trusted_ref"),
        "actor": _require_string(payload.get("actor"), "actor"),
        "target_branch": _require_string(payload.get("target_branch"), "target_branch"),
        "pushed_head_sha": _require_sha(payload.get("pushed_head_sha"), "pushed_head_sha", 40),
        "applied_patch_sha256": _require_sha(payload.get("applied_patch_sha256"), "applied_patch_sha256", 64),
        "merge_commit": bool(payload.get("merge_commit")),
        "pr_merged": False,
    }


def build_push_result(fix_merge_payload: dict[str, Any], trusted_push_payload: dict[str, Any]) -> dict[str, Any]:
    fix_merge = validate_fix_merge(fix_merge_payload)
    trusted_push = validate_trusted_push(trusted_push_payload)
    return {
        "schema_version": PUSH_SCHEMA,
        "stage": "stage07-push",
        "status": "pushed",
        "can_continue": True,
        "repository": fix_merge["repository"],
        "pr_number": fix_merge["pr_number"],
        "previous_head_sha": fix_merge["head_sha"],
        "pushed_head_sha": trusted_push["pushed_head_sha"],
        "target_branch": trusted_push["target_branch"],
        "trusted_ref": trusted_push["trusted_ref"],
        "actor": trusted_push["actor"],
        "applied_patch_sha256": trusted_push["applied_patch_sha256"],
        "merge_commit": trusted_push["merge_commit"],
        "pr_merged": trusted_push["pr_merged"],
        "touched_files": fix_merge["touched_files"],
        "validation_commands": fix_merge["validation_commands"],
    }
