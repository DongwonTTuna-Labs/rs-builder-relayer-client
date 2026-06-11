#!/usr/bin/env bash
set -euo pipefail

readonly DEFAULT_STATUS=".omo/ci/trusted-controller-status.json"
readonly DEFAULT_OUTPUT=".omo/ci/trusted-controller-comment.md"
readonly DEFAULT_MODE="render"

repo_root="${GITHUB_WORKSPACE:-$(pwd)}"
status_path="${GRIMOIRE_TRUSTED_CONTROLLER_STATUS:-${DEFAULT_STATUS}}"
output_path="${GRIMOIRE_PROTECTED_COMMENT_OUTPUT:-${DEFAULT_OUTPUT}}"
mode="${GRIMOIRE_PROTECTED_COMMENT_MODE:-${DEFAULT_MODE}}"
repo="${GRIMOIRE_BASE_REPO:-${GITHUB_REPOSITORY:-}}"
pr_number="${GRIMOIRE_PR_NUMBER:-}"

usage() {
  cat <<'USAGE'
Usage: grimoire-protected-comment.sh [options]

Trusted-base helper for Task 12 protected-path halt/read-only comments. It reads
trusted-controller status JSON, writes a deterministic markdown reason comment
artifact, and optionally posts it only when explicitly requested.

Options:
  --repo-root PATH      PR workspace. Default: GITHUB_WORKSPACE or cwd.
  --status PATH         trusted-controller status JSON. Default: .omo/ci/trusted-controller-status.json.
  --output PATH         Comment artifact path. Default: .omo/ci/trusted-controller-comment.md.
  --mode MODE           render, dry-run, or post. Default: render.
  --repo OWNER/REPO     GitHub repository for post mode. Default: GRIMOIRE_BASE_REPO or GITHUB_REPOSITORY.
  --pr-number NUMBER    Pull request number for post mode. Default: GRIMOIRE_PR_NUMBER.
  --help                Show this help text.

Auth for post mode:
  Uses GRIMOIRE_PAT first, then CODEX_LOOP_PAT. The helper derives GH_TOKEN
  only for the gh pr comment subprocess after selecting one of those PAT
  sources; it does not read ambient GH_TOKEN as an input credential.
USAGE
}

fail_usage() {
  printf 'grimoire-protected-comment: %s\n\n' "$1" >&2
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
    --status)
      [ "$#" -ge 2 ] || fail_usage "--status requires a value"
      status_path="$2"
      shift 2
      ;;
    --output)
      [ "$#" -ge 2 ] || fail_usage "--output requires a value"
      output_path="$2"
      shift 2
      ;;
    --mode)
      [ "$#" -ge 2 ] || fail_usage "--mode requires a value"
      mode="$2"
      shift 2
      ;;
    --repo)
      [ "$#" -ge 2 ] || fail_usage "--repo requires a value"
      repo="$2"
      shift 2
      ;;
    --pr-number)
      [ "$#" -ge 2 ] || fail_usage "--pr-number requires a value"
      pr_number="$2"
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

case "$mode" in
  render|dry-run|post)
    ;;
  '')
    mode="render"
    ;;
  *)
    fail_usage "--mode must be render, dry-run, or post"
    ;;
esac

if ! command -v python3 >/dev/null 2>&1; then
  printf 'grimoire-protected-comment: python3 is required\n' >&2
  exit 127
fi

cd "$repo_root"

select_pat_source() {
  if [ -n "${GRIMOIRE_PAT:-}" ]; then
    printf 'GRIMOIRE_PAT\n'
  elif [ -n "${CODEX_LOOP_PAT:-}" ]; then
    printf 'CODEX_LOOP_PAT\n'
  else
    return 1
  fi
}

pat_value_for_source() {
  case "$1" in
    GRIMOIRE_PAT) printf '%s\n' "${GRIMOIRE_PAT}" ;;
    CODEX_LOOP_PAT) printf '%s\n' "${CODEX_LOOP_PAT}" ;;
    *) return 1 ;;
  esac
}

comment_required="$(python3 - "$status_path" "$output_path" "$mode" <<'PY'
import json
import pathlib
import sys
from datetime import datetime, timezone

status_arg, output_arg, mode = sys.argv[1:4]
status_path = pathlib.Path(status_arg)
output_path = pathlib.Path(output_arg)

if not status_path.is_absolute():
    status_path = pathlib.Path.cwd() / status_path
if not output_path.is_absolute():
    output_path = pathlib.Path.cwd() / output_path

payload = json.loads(status_path.read_text(encoding="utf-8"))
required = [
    "schema_version",
    "stage",
    "status",
    "action",
    "protected_paths",
    "protected_path_matches",
    "read_only",
    "model_execution_allowed",
    "write_allowed",
    "commit_allowed",
    "push_allowed",
    "github_mutation_allowed",
    "reason",
    "push_attempts",
    "status_path",
    "trusted_protected_comment_path",
    "protected_comment_required",
    "protected_comment_artifact",
]
missing = [field for field in required if field not in payload]
if missing:
    raise SystemExit("trusted-controller status missing fields: " + ", ".join(missing))
if payload.get("schema_version") != 1:
    raise SystemExit("trusted-controller status has wrong schema_version")
if payload.get("stage") != "grimoire-trusted-controller":
    raise SystemExit("trusted-controller status has wrong stage")

status = payload.get("status")
action = payload.get("action")
protected_paths = payload.get("protected_paths")
protected_matches = payload.get("protected_path_matches")
if not isinstance(protected_paths, list):
    raise SystemExit("trusted-controller protected_paths must be an array")
if not isinstance(protected_matches, list):
    raise SystemExit("trusted-controller protected_path_matches must be an array")

output_path.parent.mkdir(parents=True, exist_ok=True)
if status == "ok" and action == "continue" and not protected_paths:
    if payload.get("protected_comment_required") is not False:
        raise SystemExit("trusted-controller protected_comment_required must be false for normal paths")
    output_path.write_text("", encoding="utf-8")
    print("false")
    raise SystemExit(0)

if status != "protected" or action not in {"halt", "read-only"} or not protected_paths:
    raise SystemExit(f"trusted-controller status does not require a protected-path comment: status={status} action={action}")
if payload.get("protected_comment_required") is not True:
    raise SystemExit("trusted-controller protected_comment_required must be true for protected paths")
if "grimoire-protected-comment.sh" not in str(payload.get("trusted_protected_comment_path") or ""):
    raise SystemExit("trusted-controller trusted protected-comment helper path missing from status")

disabled = [
    "model_execution_allowed",
    "write_allowed",
    "commit_allowed",
    "push_allowed",
    "github_mutation_allowed",
]
for field in disabled:
    if payload.get(field) is not False:
        raise SystemExit(f"trusted-controller did not disable protected capability: {field}")
if payload.get("read_only") is not True:
    raise SystemExit("trusted-controller read_only must be true for protected paths")
if payload.get("push_attempts") != 0:
    raise SystemExit("trusted-controller push_attempts must remain 0 for protected paths")

def inline_code(value):
    return "`" + str(value).replace("`", "\\`") + "`"

match_reasons = {}
for item in protected_matches:
    if isinstance(item, dict) and isinstance(item.get("path"), str):
        match_reasons[item["path"]] = str(item.get("reason") or "protected path")

protected_lines = []
for path in protected_paths:
    protected_lines.append(f"- {inline_code(path)} - {match_reasons.get(path, 'protected path')}")

capability_lines = []
for field in disabled:
    capability_lines.append(f"- {inline_code(field + '=false')}")
capability_lines.append(f"- {inline_code('read_only=true')}")
capability_lines.append(f"- {inline_code('push_attempts=0')}")

status_artifact = str(payload.get("status_path") or status_arg)
comment_artifact = output_path
try:
    comment_artifact_text = comment_artifact.relative_to(pathlib.Path.cwd()).as_posix()
except ValueError:
    comment_artifact_text = str(comment_artifact)

body = "\n".join([
    "<!-- grimoire-trusted-controller-protected-path -->",
    "## Summary",
    "Grimoire halted this PR before model execution, writes, commits, pushes, or general GitHub mutations because trusted-controller detected protected controller or security paths.",
    "",
    "## Protected Paths",
    *protected_lines,
    "",
    "## Disabled Capabilities",
    *capability_lines,
    "",
    "## How To Proceed",
    "- Review the protected controller/security changes manually before trusting this PR's automation changes.",
    "- If the automation or policy change is intended, merge or otherwise update the trusted base controller first, then rerun grimoire from that trusted base.",
    "- This halt comment is generated from trusted base-controller material and does not require model or relay credentials.",
    "",
    "## Source/Status Artifact",
    f"- Status artifact: {inline_code(status_artifact)}",
    f"- Comment artifact: {inline_code(comment_artifact_text)}",
    f"- Comment mode: {inline_code(mode)}",
    f"- Generated at: {inline_code(datetime.now(timezone.utc).isoformat().replace('+00:00', 'Z'))}",
    "",
])
output_path.write_text(body, encoding="utf-8")
print("true")
PY
)"

if [ "$comment_required" != "true" ]; then
  printf 'grimoire-protected-comment: no protected-path comment required; wrote noop artifact %s\n' "$output_path"
  exit 0
fi

case "$mode" in
  render|dry-run)
    printf 'grimoire-protected-comment: rendered protected-path reason comment to %s\n' "$output_path"
    exit 0
    ;;
  post)
    ;;
esac

if [ -z "$repo" ] || [ -z "$pr_number" ]; then
  printf 'grimoire-protected-comment: --repo and --pr-number are required for post mode\n' >&2
  exit 2
fi
if ! command -v gh >/dev/null 2>&1; then
  printf 'grimoire-protected-comment: gh is required for post mode\n' >&2
  exit 127
fi
if ! selected_pat_source="$(select_pat_source)"; then
  printf 'grimoire-protected-comment: GRIMOIRE_PAT or CODEX_LOOP_PAT is required for post mode\n' >&2
  exit 1
fi
if ! selected_pat="$(pat_value_for_source "$selected_pat_source")"; then
  printf 'grimoire-protected-comment: internal error selecting post-mode PAT source\n' >&2
  exit 1
fi
if [ -n "${GITHUB_ACTIONS:-}" ]; then
  printf '::add-mask::%s\n' "$selected_pat"
fi
GH_TOKEN="$selected_pat" gh pr comment "$pr_number" --repo "$repo" --body-file "$output_path"
printf 'grimoire-protected-comment: posted protected-path reason comment to PR %s in %s using %s auth\n' "$pr_number" "$repo" "$selected_pat_source"
