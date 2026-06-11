#!/usr/bin/env bash
set -euo pipefail

readonly DEFAULT_OUTPUT=".omo/ci/trusted-controller-status.json"
readonly DEFAULT_PROTECTED_ACTION="halt"

repo_root="${GITHUB_WORKSPACE:-$(pwd)}"
output_path="${GRIMOIRE_TRUSTED_CONTROLLER_OUTPUT:-${DEFAULT_OUTPUT}}"
base_controller_path="${GRIMOIRE_BASE_CONTROLLER:-${GRIMOIRE_BASE_CONTROLLER_PATH:-}}"
protected_action="${GRIMOIRE_PROTECTED_ACTION:-${DEFAULT_PROTECTED_ACTION}}"
diff_base="${GRIMOIRE_DIFF_BASE:-}"
diff_head="${GRIMOIRE_DIFF_HEAD:-HEAD}"
changed_list_paths=()
changed_files=()

usage() {
  cat <<'USAGE'
Usage: grimoire-trusted-controller.sh [options]

Deterministic trusted-controller guard for grimoire Task 12. It inspects the
PR-changed file list, validates that controller material came from a trusted
base copy, and writes .omo/ci/trusted-controller-status.json before any model,
comment, commit, push, or write-mutation path may run.

Options:
  --repo-root PATH             PR-head workspace. Default: GITHUB_WORKSPACE or cwd.
  --base-controller PATH       Trusted base-controller copy. Alias for --base-controller-path.
  --base-controller-path PATH  Trusted base-controller copy. Default: GRIMOIRE_BASE_CONTROLLER or GRIMOIRE_BASE_CONTROLLER_PATH.
  --changed-files PATH         Newline-delimited changed-file list. Repeatable.
  --changed-file PATH          Single changed path. Repeatable.
  --diff-from-git REV          Derive changed files with git diff --name-only REV...HEAD.
  --diff-to-git REV            Head rev for --diff-from-git. Default: HEAD.
  --protected-action ACTION    halt or read-only. Default: halt.
  --output PATH                Status JSON path. Default: .omo/ci/trusted-controller-status.json.
  --help                       Show this help text.

Env fallbacks:
  GRIMOIRE_CHANGED_FILES_FILE  Newline-delimited changed-file list.
  GRIMOIRE_CHANGED_FILES       Newline-delimited changed-file list.
  GRIMOIRE_BASE_CONTROLLER      trusted controller root.
  GRIMOIRE_BASE_CONTROLLER_PATH trusted controller root legacy alias.

Protected paths:
  .github/**, .opencode/**, opencode.json, root/nested AGENTS.md, and
  docs/SECURITY.md, docs/REVIEW_CHECKLIST.md, docs/FORKED_RELAYER_CRATE.md,
  docs/PUBLISHING_DISABLED.md.
USAGE
}

fail_usage() {
  printf 'grimoire-trusted-controller: %s\n\n' "$1" >&2
  usage >&2
  exit 2
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --help|-h)
      usage
      exit 0
      ;;
    --repo-root)
      [ "$#" -ge 2 ] || fail_usage "--repo-root requires a value"
      repo_root="$2"
      shift 2
      ;;
    --base-controller|--base-controller-path)
      [ "$#" -ge 2 ] || fail_usage "$1 requires a value"
      base_controller_path="$2"
      shift 2
      ;;
    --changed-files)
      [ "$#" -ge 2 ] || fail_usage "--changed-files requires a value"
      changed_list_paths+=("$2")
      shift 2
      ;;
    --changed-file)
      [ "$#" -ge 2 ] || fail_usage "--changed-file requires a value"
      changed_files+=("$2")
      shift 2
      ;;
    --diff-from-git)
      [ "$#" -ge 2 ] || fail_usage "--diff-from-git requires a value"
      diff_base="$2"
      shift 2
      ;;
    --diff-to-git)
      [ "$#" -ge 2 ] || fail_usage "--diff-to-git requires a value"
      diff_head="$2"
      shift 2
      ;;
    --protected-action)
      [ "$#" -ge 2 ] || fail_usage "--protected-action requires a value"
      protected_action="$2"
      shift 2
      ;;
    --output)
      [ "$#" -ge 2 ] || fail_usage "--output requires a value"
      output_path="$2"
      shift 2
      ;;
    --*)
      fail_usage "unknown option: $1"
      ;;
    *)
      fail_usage "unexpected argument: $1"
      ;;
  esac
done

case "$protected_action" in
  halt|read-only)
    ;;
  *)
    fail_usage "--protected-action must be halt or read-only"
    ;;
esac

if ! command -v python3 >/dev/null 2>&1; then
  printf 'grimoire-trusted-controller: python3 is required\n' >&2
  exit 127
fi

if [ -z "$base_controller_path" ]; then
  base_controller_path="$repo_root"
fi

python_args=(
  --repo-root "$repo_root"
  --base-controller-path "$base_controller_path"
  --output "$output_path"
  --protected-action "$protected_action"
  --diff-head "$diff_head"
)

if [ -n "$diff_base" ]; then
  python_args+=(--diff-base "$diff_base")
fi
if [ "${#changed_list_paths[@]}" -gt 0 ]; then
  for value in "${changed_list_paths[@]}"; do
    python_args+=(--changed-list "$value")
  done
fi
if [ "${#changed_files[@]}" -gt 0 ]; then
  for value in "${changed_files[@]}"; do
    python_args+=(--changed-file "$value")
  done
fi
if [ -n "${GRIMOIRE_CHANGED_FILES_FILE:-}" ]; then
  python_args+=(--changed-list "$GRIMOIRE_CHANGED_FILES_FILE")
fi
if [ -n "${GRIMOIRE_CHANGED_FILES:-}" ]; then
  python_args+=(--changed-env "$GRIMOIRE_CHANGED_FILES")
fi

python3 - "${python_args[@]}" <<'PY'
import argparse
import json
import pathlib
import posixpath
import subprocess
import sys
from datetime import datetime, timezone

PROTECTED_DOCS = {
    "docs/SECURITY.md",
    "docs/REVIEW_CHECKLIST.md",
    "docs/FORKED_RELAYER_CRATE.md",
    "docs/PUBLISHING_DISABLED.md",
}
REQUIRED_STATUS_FIELDS = [
    "schema_version",
    "stage",
    "status",
    "action",
    "protected_paths",
    "read_only",
    "model_execution_allowed",
    "write_allowed",
    "commit_allowed",
    "push_allowed",
    "github_mutation_allowed",
    "base_controller_path",
    "reason",
    "push_attempts",
    "trusted_protected_comment_path",
    "protected_comment_required",
    "protected_comment_artifact",
]


def utc_now():
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def normalize_path(raw):
    text = str(raw).strip().replace("\\", "/")
    while text.startswith("./"):
        text = text[2:]
    text = posixpath.normpath(text) if text else ""
    if text == ".":
        return ""
    return text


def reject_path(raw):
    path = normalize_path(raw)
    if not path:
        return None
    if path.startswith("/"):
        return None
    if ".." in pathlib.PurePosixPath(path).parts:
        return None
    return path


def read_changed_list(path):
    source = pathlib.Path(path)
    if not source.exists():
        raise FileNotFoundError(f"changed-file list does not exist: {path}")
    return source.read_text(encoding="utf-8", errors="replace").splitlines()


def derive_git_diff(repo_root, base, head):
    if not base:
        return []
    try:
        output = subprocess.check_output(
            ["git", "-C", str(repo_root), "diff", "--name-only", f"{base}...{head}"],
            stderr=subprocess.STDOUT,
        )
    except subprocess.CalledProcessError as exc:
        text = exc.output.decode("utf-8", errors="replace").strip()
        raise RuntimeError(f"git diff --name-only {base}...{head} failed: {text}") from exc
    return output.decode("utf-8", errors="replace").splitlines()


def protected_reason(path):
    if path == ".github" or path.startswith(".github/"):
        return ".github/** controller/workflow path"
    if path == ".opencode" or path.startswith(".opencode/"):
        return ".opencode/** model/controller config path"
    if path == "opencode.json":
        return "opencode.json model/controller config"
    if pathlib.PurePosixPath(path).name == "AGENTS.md":
        return "root or nested AGENTS.md instruction path"
    if path in PROTECTED_DOCS:
        return "security-critical documentation path"
    return ""


def controller_checks(base):
    scripts = base / ".github" / "scripts"
    checks = {
        "base_controller_exists": base.exists() and base.is_dir(),
        "trusted_controller_exists": (scripts / "grimoire-trusted-controller.sh").exists(),
        "trusted_controller_executable": (scripts / "grimoire-trusted-controller.sh").exists()
        and (scripts / "grimoire-trusted-controller.sh").stat().st_mode & 0o111 != 0,
        "protected_comment_helper_exists": (scripts / "grimoire-protected-comment.sh").exists(),
        "protected_comment_helper_executable": (scripts / "grimoire-protected-comment.sh").exists()
        and (scripts / "grimoire-protected-comment.sh").stat().st_mode & 0o111 != 0,
        "cast_driver_exists": (scripts / "grimoire-cast.sh").exists(),
        "cast_driver_executable": (scripts / "grimoire-cast.sh").exists()
        and (scripts / "grimoire-cast.sh").stat().st_mode & 0o111 != 0,
        "opencode_config_exists": (base / "opencode.json").exists(),
        "omo_config_exists": (base / ".opencode" / "oh-my-openagent.jsonc").exists(),
    }
    blockers = [name for name, ok in checks.items() if not ok]
    return checks, blockers


def path_for_output(path, repo_root):
    try:
        return path.resolve().relative_to(repo_root.resolve()).as_posix()
    except (OSError, ValueError):
        return str(path)


def main():
    parser = argparse.ArgumentParser(add_help=False)
    parser.add_argument("--repo-root", required=True)
    parser.add_argument("--base-controller-path", required=True)
    parser.add_argument("--output", required=True)
    parser.add_argument("--protected-action", choices=["halt", "read-only"], required=True)
    parser.add_argument("--changed-list", action="append", default=[])
    parser.add_argument("--changed-file", action="append", default=[])
    parser.add_argument("--changed-env", action="append", default=[])
    parser.add_argument("--diff-base", default="")
    parser.add_argument("--diff-head", default="HEAD")
    args = parser.parse_args()

    repo_root = pathlib.Path(args.repo_root)
    output = pathlib.Path(args.output)
    if not output.is_absolute():
        output = repo_root / output
    base = pathlib.Path(args.base_controller_path)

    raw_paths = []
    sources = []
    for list_path in args.changed_list:
        raw_paths.extend(read_changed_list(list_path))
        sources.append({"type": "file", "value": list_path})
    for changed_env in args.changed_env:
        raw_paths.extend(changed_env.splitlines())
        sources.append({"type": "env", "value": "GRIMOIRE_CHANGED_FILES"})
    raw_paths.extend(args.changed_file)
    if args.changed_file:
        sources.append({"type": "argv", "value": "--changed-file"})
    if args.diff_base:
        raw_paths.extend(derive_git_diff(repo_root, args.diff_base, args.diff_head))
        sources.append({"type": "git", "value": f"{args.diff_base}...{args.diff_head}"})

    changed = []
    invalid = []
    seen = set()
    for raw in raw_paths:
        path = reject_path(raw)
        if path is None:
            if str(raw).strip():
                invalid.append(str(raw))
            continue
        if path not in seen:
            changed.append(path)
            seen.add(path)

    matches = []
    for path in changed:
        reason = protected_reason(path)
        if reason:
            matches.append({"path": path, "reason": reason})

    checks, material_blockers = controller_checks(base)
    protected_paths = [item["path"] for item in matches]
    material_ok = not material_blockers

    if material_ok and not protected_paths:
        status = "ok"
        action = "continue"
        read_only = False
        model_allowed = True
        write_allowed = True
        commit_allowed = True
        push_allowed = True
        github_allowed = True
        reason = "no protected paths touched; trusted base controller material is complete"
        exit_code = 0
    elif material_ok:
        status = "protected"
        action = args.protected_action
        read_only = True
        model_allowed = False
        write_allowed = False
        commit_allowed = False
        push_allowed = False
        github_allowed = False
        reason = "protected path change requires trusted-controller halt/read-only: " + ", ".join(protected_paths)
        exit_code = 0
    else:
        status = "blocked"
        action = "halt"
        read_only = True
        model_allowed = False
        write_allowed = False
        commit_allowed = False
        push_allowed = False
        github_allowed = False
        reason = "trusted base controller material incomplete: " + ", ".join(material_blockers)
        exit_code = 1

    if invalid and status == "ok":
        status = "blocked"
        action = "halt"
        read_only = True
        model_allowed = False
        write_allowed = False
        commit_allowed = False
        push_allowed = False
        github_allowed = False
        reason = "changed-file list contained invalid paths: " + ", ".join(invalid)
        exit_code = 1

    payload = {
        "schema_version": 1,
        "stage": "grimoire-trusted-controller",
        "generated_at": utc_now(),
        "status": status,
        "action": action,
        "reason": reason,
        "protected_paths": protected_paths,
        "protected_path_matches": matches,
        "changed_files": changed,
        "changed_file_sources": sources,
        "invalid_changed_paths": invalid,
        "read_only": read_only,
        "model_execution_allowed": model_allowed,
        "write_allowed": write_allowed,
        "write_mutation_allowed": write_allowed,
        "commit_allowed": commit_allowed,
        "push_allowed": push_allowed,
        "github_mutation_allowed": github_allowed,
        "comment_allowed": github_allowed,
        "push_attempts": 0,
        "base_controller_path": str(base.resolve()),
        "trusted_controller_path": str((base / ".github" / "scripts" / "grimoire-trusted-controller.sh").resolve()),
        "trusted_protected_comment_path": str((base / ".github" / "scripts" / "grimoire-protected-comment.sh").resolve()),
        "trusted_cast_path": str((base / ".github" / "scripts" / "grimoire-cast.sh").resolve()),
        "trusted_opencode_config_path": str((base / "opencode.json").resolve()),
        "trusted_omo_config_path": str((base / ".opencode" / "oh-my-openagent.jsonc").resolve()),
        "status_path": path_for_output(output, repo_root),
        "protected_comment_required": status == "protected",
        "protected_comment_artifact": ".omo/ci/trusted-controller-comment.md",
        "controller_checks": checks,
        "material_blockers": material_blockers,
        "protected_patterns": [
            ".github/**",
            ".opencode/**",
            "opencode.json",
            "**/AGENTS.md",
            "docs/SECURITY.md",
            "docs/REVIEW_CHECKLIST.md",
            "docs/FORKED_RELAYER_CRATE.md",
            "docs/PUBLISHING_DISABLED.md",
        ],
    }

    missing = [field for field in REQUIRED_STATUS_FIELDS if field not in payload]
    if missing:
        raise RuntimeError("internal status missing fields: " + ", ".join(missing))

    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(f"grimoire-trusted-controller: status={status} action={action} protected_paths={len(protected_paths)} output={path_for_output(output, repo_root)}")
    return exit_code


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as exc:
        print(f"grimoire-trusted-controller: {exc}", file=sys.stderr)
        sys.exit(1)
PY
