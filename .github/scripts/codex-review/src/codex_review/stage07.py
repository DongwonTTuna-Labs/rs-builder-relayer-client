"""Stage07 trusted push artifact contract logic."""

from __future__ import annotations

import shlex
import subprocess
from pathlib import Path, PurePosixPath
from typing import Any

from .validators import require_keys, require_schema_version


FIX_MERGE_SCHEMA = "codex.stage06.fix_merge.v1"
TRUSTED_PUSH_SCHEMA = "codex.stage07.trusted_push.v1"
PUSH_SCHEMA = "codex.stage07.push.v1"
RUBY_WORKFLOW_YAML_CHECK = (
    'require "yaml"; Dir[".github/workflows/*.yml"].each { |p| YAML.load_file(p) }; puts "yaml ok"'
)
EXACT_VALIDATION_COMMANDS = {
    ("git", "diff", "--check"),
    ("cargo", "fmt", "--all", "--check"),
    ("cargo", "test", "--workspace", "--all-features"),
    ("cargo", "clippy", "--workspace", "--all-targets", "--all-features", "--", "-D", "warnings"),
    ("actionlint", ".github/workflows/codex-pr-review.yml"),
    ("actionlint", "-ignore", 'label "dongwontuna-labs-runner" is unknown', ".github/workflows/codex-pr-review.yml"),
    ("ruby", "-e", RUBY_WORKFLOW_YAML_CHECK),
}
UNITTEST_DISCOVERY_DIRS = {
    ".github/scripts/codex-review/tests",
    ".github/scripts/tests",
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
        ["status", "can_continue", "repository", "pr_number", "head_sha", "touched_files", "validation_commands", "candidate_patch"],
    )
    if _require_string(payload.get("status"), "status") != "ready" or payload.get("can_continue") is not True:
        raise ValueError("stage07 requires ready fix_merge")
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


def _is_allowed_test_filename(value: str) -> bool:
    path = PurePosixPath(value)
    return len(path.parts) == 1 and path.name.startswith("test_") and path.suffix == ".py"


def _is_allowed_unittest_command(argv: list[str]) -> bool:
    if len(argv) not in {6, 8}:
        return False
    if argv[:5] != ["python3", "-m", "unittest", "discover", "-s"]:
        return False
    if argv[5] not in UNITTEST_DISCOVERY_DIRS:
        return False
    if len(argv) == 8:
        return argv[6] == "-p" and _is_allowed_test_filename(argv[7])
    return True


def _is_allowed_python_script_command(argv: list[str]) -> bool:
    if len(argv) != 2 or argv[0] != "python3":
        return False
    path = PurePosixPath(argv[1])
    return (
        not path.is_absolute()
        and ".." not in path.parts
        and len(path.parts) == 4
        and path.parts[:3] == (".github", "scripts", "tests")
        and path.name.startswith("test_")
        and path.suffix == ".py"
    )


def validation_command_argv(command: str) -> list[str]:
    try:
        argv = shlex.split(_require_string(command, "validation command"), posix=True)
    except ValueError as exc:
        _unsupported_validation_command(command)
        raise AssertionError("unreachable") from exc
    if tuple(argv) in EXACT_VALIDATION_COMMANDS:
        return argv
    if _is_allowed_unittest_command(argv) or _is_allowed_python_script_command(argv):
        return argv
    _unsupported_validation_command(command)
    raise AssertionError("unreachable")


def validation_commands_argv(commands: list[str]) -> list[list[str]]:
    return [validation_command_argv(command) for command in commands]


def run_validation_commands(commands: list[str], workspace: Path) -> None:
    argv_commands = validation_commands_argv(commands)
    for argv in argv_commands:
        subprocess.run(argv, cwd=workspace, check=True)


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
