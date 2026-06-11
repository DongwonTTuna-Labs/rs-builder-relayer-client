#!/usr/bin/env bash
set -euo pipefail

script_dir="$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
script_controller_root="$(CDPATH='' cd -- "${script_dir}/../.." && pwd -P)"
trusted_controller_root="${GRIMOIRE_BASE_CONTROLLER:-${GRIMOIRE_BASE_CONTROLLER_PATH:-${GRIMOIRE_TRUSTED_CONTROLLER_ROOT:-$script_controller_root}}}"
if [ -d "$trusted_controller_root" ]; then
  trusted_controller_root="$(CDPATH='' cd -- "$trusted_controller_root" && pwd -P)"
fi

readonly TRUSTED_CONTROLLER_ROOT="$trusted_controller_root"

readonly REVIEW_SCRIPT="${TRUSTED_CONTROLLER_ROOT}/.github/scripts/grimoire-review.sh"
readonly DESIGN_SCRIPT="${TRUSTED_CONTROLLER_ROOT}/.github/scripts/grimoire-design.sh"
readonly SPEC_GAP_SCRIPT="${TRUSTED_CONTROLLER_ROOT}/.github/scripts/grimoire-spec-gap-comment.sh"
readonly FIX_SCRIPT="${TRUSTED_CONTROLLER_ROOT}/.github/scripts/grimoire-fix.sh"
readonly VERIFY_SCRIPT="${TRUSTED_CONTROLLER_ROOT}/.github/scripts/grimoire-verify.sh"
readonly LABEL_SCRIPT="${TRUSTED_CONTROLLER_ROOT}/.github/scripts/grimoire-labels.sh"

readonly REVIEW_JSON=".omo/ci/review-findings.json"
readonly SPEC_JSON=".omo/ci/spec-sufficiency.json"
readonly DESIGN_PLAN=".omo/ci/design-plan.md"
readonly SPEC_GAP_STATUS=".omo/ci/spec-gap-status.json"
readonly SPEC_GAP_COMMENT=".omo/ci/spec-gap-comment.md"
readonly FIX_STATUS=".omo/ci/fix-status.json"
readonly VERDICT_JSON=".omo/grimoire/verdict.json"
readonly REAL_BOULDER_JSON=".omo/boulder.json"
readonly MOCK_BOULDER_JSON=".omo/ci/grimoire-cast-mock-boulder.json"
readonly DECISION_JSON=".omo/ci/grimoire-cast-decision.json"
readonly LOOP_METADATA_JSON=".omo/ci/grimoire-loop-metadata.json"
readonly TRUSTED_STATUS_JSON="${GRIMOIRE_TRUSTED_CONTROLLER_STATUS:-.omo/ci/trusted-controller-status.json}"
readonly LABEL_STATE_FILE=".omo/ci/grimoire-label-state.txt"
readonly LABEL_STATUS_JSON=".omo/ci/grimoire-label-status.json"
readonly BOT_COMMIT_MESSAGE="chore(review): autofix [grimoire]"
readonly BOT_AUTHOR_NAME="grimoire-autofix[bot]"
readonly BOT_AUTHOR_EMAIL="grimoire-autofix@users.noreply.github.com"

mode="${GRIMOIRE_CAST_MODE:-real}"
dry_run="${GRIMOIRE_CAST_DRY_RUN:-0}"
repo_root="${GITHUB_WORKSPACE:-$(pwd)}"
liveness_timeout_seconds="${GRIMOIRE_LIVENESS_TIMEOUT_SECONDS:-2700}"
heartbeat_seconds="${GRIMOIRE_HEARTBEAT_SECONDS:-30}"
mock_session_id="ses_mock_grimoire_task11"
trusted_status_json="$TRUSTED_STATUS_JSON"

usage() {
  cat <<'USAGE'
Usage: grimoire-cast.sh [options]

Headless grimoire review-loop driver for Task 11. Reuses the existing stage
contracts instead of replacing them:
  review(5) -> design(6) -> spec-gap halt(7) -> fix(8)
  -> Atlas/OMO boulder completion -> verify(9) -> jq all-APPROVE decision.

Options:
  --mode MODE        real, mock-noop, mock-fixed, mock-spec-insufficient,
                     mock-stage-failure, mock-missing-verdict, mock-reject,
                     mock-boulder-missing, mock-boulder-malformed,
                     mock-boulder-active, or mock-boulder-session-mismatch.
                     Default: real.
  --dry-run          Do not commit or push; write intended mutation decision only.
  --repo-root PATH   Repository root. Default: GITHUB_WORKSPACE or current dir.
  --trusted-status PATH
                     Trusted-controller status JSON. Default: .omo/ci/trusted-controller-status.json.
  --liveness-timeout-seconds N
                     Timeout for persistent non-approval/non-convergence.
                     This is a wall-clock liveness guard, not a semantic
                     iteration cap.
  --heartbeat-seconds N
                     Heartbeat interval while waiting/continuing.
  --help             Show this help text.

Real mode fail-closed prerequisites:
  AI_RELAY_API_KEY, GRIMOIRE_PAT or CODEX_LOOP_PAT, GRIMOIRE_FIX_READY=1,
  GRIMOIRE_BOULDER_READY=1, GRIMOIRE_VERIFY_READY=1, opencode CLI, and grounded
  PR metadata: GRIMOIRE_PR_NUMBER, GRIMOIRE_HEAD_REPO, GRIMOIRE_HEAD_REF,
  GRIMOIRE_HEAD_SHA, GRIMOIRE_BASE_REPO, and GRIMOIRE_BASE_REF.

Mutation and loop policy:
  clear-noop with all F1-F4 APPROVE is terminal success and exits without
  commit or push.
  fixed with all F1-F4 APPROVE is nonterminal: it prepares exactly one
  scoped non-empty bot commit/push path using the message
  "chore(review): autofix [grimoire]", then relies on the GitHub
  pull_request.synchronize event for a fresh re-review. Bot-authored
  grimoire autofix commits are detected for metadata only; they are never
  skipped before review or F1-F4 verification.
USAGE
}

fail_usage() {
  printf 'grimoire-cast: %s\n\n' "$1" >&2
  usage >&2
  exit 2
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --help|-h)
      usage
      exit 0
      ;;
    --mode)
      [ "$#" -ge 2 ] || fail_usage "--mode requires a value"
      mode="$2"
      shift 2
      ;;
    --dry-run)
      dry_run="1"
      shift
      ;;
    --repo-root)
      [ "$#" -ge 2 ] || fail_usage "--repo-root requires a value"
      repo_root="$2"
      shift 2
      ;;
    --trusted-status)
      [ "$#" -ge 2 ] || fail_usage "--trusted-status requires a value"
      trusted_status_json="$2"
      shift 2
      ;;
    --liveness-timeout-seconds)
      [ "$#" -ge 2 ] || fail_usage "--liveness-timeout-seconds requires a value"
      liveness_timeout_seconds="$2"
      shift 2
      ;;
    --heartbeat-seconds)
      [ "$#" -ge 2 ] || fail_usage "--heartbeat-seconds requires a value"
      heartbeat_seconds="$2"
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
  real|mock-noop|mock-fixed|mock-spec-insufficient|mock-stage-failure|mock-missing-verdict|mock-reject|mock-boulder-missing|mock-boulder-malformed|mock-boulder-active|mock-boulder-session-mismatch)
    ;;
  *)
    fail_usage "unsupported mode: ${mode}"
    ;;
esac

case "$dry_run" in
  0|1|true|false)
    ;;
  *)
    fail_usage "GRIMOIRE_CAST_DRY_RUN must be 0, 1, true, or false"
    ;;
esac

case "$liveness_timeout_seconds" in
  ''|*[!0-9]*) fail_usage "--liveness-timeout-seconds must be a positive integer" ;;
esac
case "$heartbeat_seconds" in
  ''|*[!0-9]*) fail_usage "--heartbeat-seconds must be a positive integer" ;;
esac
if [ "$liveness_timeout_seconds" -le 0 ]; then
  fail_usage "--liveness-timeout-seconds must be greater than zero"
fi
if [ "$heartbeat_seconds" -le 0 ]; then
  fail_usage "--heartbeat-seconds must be greater than zero"
fi

case "$mode" in
  mock-*)
    boulder_json="${GRIMOIRE_BOULDER_JSON:-$MOCK_BOULDER_JSON}"
    ;;
  *)
    boulder_json="${GRIMOIRE_BOULDER_JSON:-$REAL_BOULDER_JSON}"
    ;;
esac

cd "$repo_root"

require_python() {
  if ! command -v python3 >/dev/null 2>&1; then
    printf 'grimoire-cast: python3 is required\n' >&2
    exit 127
  fi
}

require_stage_scripts() {
  local script
  for script in "$REVIEW_SCRIPT" "$DESIGN_SCRIPT" "$SPEC_GAP_SCRIPT" "$FIX_SCRIPT" "$VERIFY_SCRIPT" "$LABEL_SCRIPT"; do
    if [ ! -x "$script" ]; then
      printf 'grimoire-cast: required stage script missing or not executable: %s\n' "$script" >&2
      exit 1
    fi
  done
}

ensure_dirs() {
  mkdir -p .omo/ci .omo/grimoire
}

mask_secret_sources() {
  if [ "${GITHUB_ACTIONS:-}" != "true" ]; then
    return 0
  fi
  if [ -n "${GRIMOIRE_PAT:-}" ]; then
    printf '::add-mask::%s\n' "$GRIMOIRE_PAT"
  fi
  if [ -n "${CODEX_LOOP_PAT:-}" ]; then
    printf '::add-mask::%s\n' "$CODEX_LOOP_PAT"
  fi
  if [ -n "${AI_RELAY_API_KEY:-}" ]; then
    printf '::add-mask::%s\n' "$AI_RELAY_API_KEY"
  fi
}

utc_now() {
  python3 - <<'PY'
from datetime import datetime, timezone
print(datetime.now(timezone.utc).isoformat().replace('+00:00', 'Z'))
PY
}

json_get() {
  local path="$1"
  local expr="$2"
  python3 - "$path" "$expr" <<'PY'
import json
import sys
path, expr = sys.argv[1:3]
with open(path, encoding='utf-8') as handle:
    value = json.load(handle)
for part in expr.split('.'):
    if not part:
        continue
    value = value[part]
if isinstance(value, bool):
    print('true' if value else 'false')
elif value is None:
    print('')
else:
    print(value)
PY
}

write_loop_metadata() {
  python3 - "$LOOP_METADATA_JSON" "$repo_root" "$mode" "$dry_run" "$liveness_timeout_seconds" "$BOT_COMMIT_MESSAGE" "$BOT_AUTHOR_NAME" "$BOT_AUTHOR_EMAIL" <<'PY'
import json
import os
import pathlib
import subprocess
import sys
from datetime import datetime, timezone

path, repo_root, mode, dry_run, timeout, bot_message, bot_name, bot_email = sys.argv[1:]


def utc_now():
    return datetime.now(timezone.utc).isoformat().replace('+00:00', 'Z')


def env_first(*names):
    for name in names:
        value = os.environ.get(name, '').strip()
        if value:
            return value
    return ''


def git_head():
    try:
        raw = subprocess.check_output(
            ['git', '-C', repo_root, 'log', '-1', '--pretty=%H%x00%an%x00%ae%x00%s'],
            stderr=subprocess.DEVNULL,
        )
    except (OSError, subprocess.CalledProcessError):
        return {}
    parts = raw.decode('utf-8', 'replace').rstrip('\n').split('\x00', 3)
    if len(parts) != 4:
        return {}
    return {
        'sha': parts[0],
        'author_name': parts[1],
        'author_email': parts[2],
        'message': parts[3],
    }

head = git_head()
event_action = env_first('GRIMOIRE_EVENT_ACTION', 'GITHUB_EVENT_ACTION')
head_sha = env_first('GRIMOIRE_HEAD_SHA') or head.get('sha', '')
author_name = env_first('GRIMOIRE_HEAD_AUTHOR_NAME', 'GRIMOIRE_HEAD_COMMIT_AUTHOR_NAME') or head.get('author_name', '')
author_email = env_first('GRIMOIRE_HEAD_AUTHOR_EMAIL', 'GRIMOIRE_HEAD_COMMIT_AUTHOR_EMAIL') or head.get('author_email', '')
author_login = env_first('GRIMOIRE_HEAD_AUTHOR_LOGIN', 'GRIMOIRE_HEAD_ACTOR', 'GITHUB_ACTOR')
message = env_first('GRIMOIRE_HEAD_COMMIT_MESSAGE', 'GRIMOIRE_HEAD_COMMIT_SUBJECT') or head.get('message', '')
by_marker = bot_message in message if message else False
by_author = author_name == bot_name or author_email == bot_email or author_login == bot_name
bot_commit = bool(by_marker or by_author)
payload = {
    'schema_version': 1,
    'stage': 'grimoire-loop-metadata',
    'generated_at': utc_now(),
    'mode': mode,
    'dry_run': dry_run in {'1', 'true'},
    'event_action': event_action,
    'synchronize_event': event_action == 'synchronize',
    'head_sha': head_sha,
    'head_commit': {
        'message': message,
        'author_name': author_name,
        'author_email': author_email,
        'author_login': author_login,
    },
    'bot_marker': bot_message,
    'bot_author': f'{bot_name} <{bot_email}>',
    'bot_commit': bot_commit,
    'bot_commit_detection': {
        'by_marker': by_marker,
        'by_author': by_author,
    },
    'review_required': True,
    're_review_bot_commits': True,
    'skip_review_for_bot_commit': False,
    'verification_required': True,
    'empty_commit_allowed': False,
    'semantic_iteration_cap': False,
    'liveness_guard': 'wall-clock-timeout',
    'liveness_timeout_seconds': int(timeout),
}
pathlib.Path(path).parent.mkdir(parents=True, exist_ok=True)
pathlib.Path(path).write_text(json.dumps(payload, indent=2, sort_keys=True) + '\n', encoding='utf-8')
PY
}

write_decision() {
  local decision="$1"
  local reason="$2"
  local exit_code="$3"
  python3 - "$DECISION_JSON" "$LOOP_METADATA_JSON" "$mode" "$decision" "$reason" "$exit_code" "$dry_run" "$BOT_COMMIT_MESSAGE" "$BOT_AUTHOR_NAME" "$BOT_AUTHOR_EMAIL" <<'PY'
import json
import pathlib
import sys
from datetime import datetime, timezone

path, metadata_path, mode, decision, reason, exit_code, dry_run, bot_message, bot_name, bot_email = sys.argv[1:]
metadata_file = pathlib.Path(metadata_path)
metadata = {}
if metadata_file.exists():
    try:
        loaded = json.loads(metadata_file.read_text(encoding='utf-8'))
        if isinstance(loaded, dict):
            metadata = loaded
    except json.JSONDecodeError:
        metadata = {}

fixed_nonterminal = decision in {'fixed-dry-run', 'fixed-pushed'}
noop_terminal = decision == 'noop-approved'
commit_attempted = decision == 'fixed-pushed'
push_attempted = decision == 'fixed-pushed'
loop_phase_by_decision = {
    'fixed-dry-run': 'fixed-nonterminal-dry-run',
    'fixed-pushed': 'fixed-nonterminal-pushed',
    'noop-approved': 'clear-noop-terminal-approved',
    'empty-commit-refused': 'empty-commit-refused',
    'mutation-scope-blocked': 'mutation-blocked-before-commit',
    'verdict-not-approved': 'nonterminal-verdict-rejected',
    'missing-verdict': 'nonterminal-verdict-missing',
    'liveness-timeout': 'wall-clock-timeout',
    'spec-insufficient-halt': 'spec-insufficient-halt',
}
loop_phase = loop_phase_by_decision.get(decision, 'fail-closed' if int(exit_code) != 0 else 'nonterminal')
payload = {
    'schema_version': 1,
    'stage': 'grimoire-cast',
    'generated_at': datetime.now(timezone.utc).isoformat().replace('+00:00', 'Z'),
    'mode': mode,
    'decision': decision,
    'reason': reason,
    'exit_code': int(exit_code),
    'dry_run': dry_run in {'1', 'true'},
    'terminal': noop_terminal,
    'terminal_success': noop_terminal,
    'loop_phase': loop_phase,
    'synchronize_expected': fixed_nonterminal,
    'fixed_push_nonterminal': fixed_nonterminal,
    'clear_noop_terminal': noop_terminal,
    're_review_required': fixed_nonterminal,
    're_review_on_synchronize': fixed_nonterminal,
    'review_required': True,
    'verification_required': True,
    'skip_review_for_bot_commit': False,
    'bot_commit': bool(metadata.get('bot_commit', False)),
    'bot_commit_re_reviewed': bool(metadata.get('bot_commit', False)),
    'bot_commit_intended': decision == 'fixed-dry-run',
    'bot_commit_created': decision == 'fixed-pushed',
    'bot_commit_message': bot_message,
    'bot_author': f'{bot_name} <{bot_email}>',
    'commit_attempted': commit_attempted,
    'push_attempted': push_attempted,
    'commit_intended': fixed_nonterminal,
    'push_intended': fixed_nonterminal,
    'empty_commit_allowed': False,
    'semantic_iteration_cap': False,
    'liveness_guard': 'wall-clock-timeout',
    'task9_all_approve_observed': decision in {'fixed-dry-run', 'fixed-pushed', 'noop-approved'},
    'decision_artifact': path,
    'loop_metadata_artifact': metadata_path,
    'loop_metadata': metadata,
}
pathlib.Path(path).parent.mkdir(parents=True, exist_ok=True)
pathlib.Path(path).write_text(json.dumps(payload, indent=2, sort_keys=True) + '\n', encoding='utf-8')
PY
}

label_mode_for_transition() {
  if [ -n "${GRIMOIRE_LABEL_MODE:-}" ]; then
    printf '%s\n' "$GRIMOIRE_LABEL_MODE"
  elif [ "$mode" = "real" ] && [ "$dry_run" != "1" ] && [ "$dry_run" != "true" ]; then
    printf 'live\n'
  else
    printf 'local\n'
  fi
}

run_label_transition() {
  local transition="$1"
  local reason="$2"
  local selected_label_mode
  local label_state_file
  local label_status_output
  local -a args

  if [ ! -x "$LABEL_SCRIPT" ]; then
    printf 'grimoire-cast: label helper missing or not executable: %s\n' "$LABEL_SCRIPT" >&2
    return 1
  fi

  selected_label_mode="$(label_mode_for_transition)"
  label_state_file="${GRIMOIRE_LABEL_STATE_FILE:-$LABEL_STATE_FILE}"
  label_status_output="${GRIMOIRE_LABEL_STATUS_OUTPUT:-$LABEL_STATUS_JSON}"
  args=(
    --mode "$selected_label_mode"
    --state "$transition"
    --repo-root "$repo_root"
    --state-file "$label_state_file"
    --status-output "$label_status_output"
  )
  if [ "$selected_label_mode" = "live" ]; then
    args+=(
      --repo "${GRIMOIRE_BASE_REPO:-${GITHUB_REPOSITORY:-}}"
      --pr-number "${GRIMOIRE_PR_NUMBER:-}"
    )
  fi
  printf 'grimoire-cast: label transition %s (%s)\n' "$transition" "$reason"
  bash "$LABEL_SCRIPT" "${args[@]}"
}

write_label_failure_decision() {
  local transition="$1"
  write_decision "label-transition-failed" "grimoire label transition failed: ${transition}" 38
  exit 38
}

mark_label_running() {
  run_label_transition running "cycle start" || write_label_failure_decision running
}

mark_label_done() {
  run_label_transition "done" "terminal clear-noop success" || write_label_failure_decision "done"
}

mark_label_fizzled() {
  local reason="$1"
  run_label_transition fizzled "$reason" || write_label_failure_decision fizzled
}

mark_label_fizzled_best_effort() {
  local reason="$1"
  if ! run_label_transition fizzled "$reason"; then
    printf 'grimoire-cast: fizzled label transition failed after %s\n' "$reason" >&2
  fi
}

fail_closed() {
  local decision="$1"
  local reason="$2"
  local exit_code="${3:-1}"
  mark_label_fizzled_best_effort "$decision"
  write_decision "$decision" "$reason" "$exit_code"
  printf 'grimoire-cast: %s: %s\n' "$decision" "$reason" >&2
  exit "$exit_code"
}

validate_trusted_controller_status() {
  local purpose="$1"
  if [ ! -s "$trusted_status_json" ]; then
    if [ "$mode" = "real" ]; then
      fail_closed "trusted-controller-missing" "trusted-controller status missing before ${purpose}: ${trusted_status_json}" 32
    fi
    return 0
  fi

  local validation
  if ! validation="$(python3 - "$trusted_status_json" "$purpose" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
purpose = sys.argv[2]
required = [
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
]
try:
    payload = json.loads(path.read_text(encoding="utf-8"))
except FileNotFoundError:
    print(f"trusted-controller status missing: {path}")
    sys.exit(1)
except json.JSONDecodeError as exc:
    print(f"trusted-controller status malformed: {exc}")
    sys.exit(1)
if not isinstance(payload, dict):
    print("trusted-controller status must be a JSON object")
    sys.exit(1)
missing = [field for field in required if field not in payload]
if missing:
    print("trusted-controller status missing fields: " + ", ".join(missing))
    sys.exit(1)
if payload.get("schema_version") != 1:
    print("trusted-controller status has wrong schema_version")
    sys.exit(1)
if payload.get("stage") != "grimoire-trusted-controller":
    print("trusted-controller status has wrong stage")
    sys.exit(1)
status = payload.get("status")
action = payload.get("action")
reason = str(payload.get("reason") or "trusted-controller blocked")
protected_paths = payload.get("protected_paths")
if not isinstance(protected_paths, list):
    print("trusted-controller protected_paths must be an array")
    sys.exit(1)
if status not in {"ok", "protected", "blocked"}:
    print(f"trusted-controller status has unsupported status: {status}")
    sys.exit(1)
if action not in {"continue", "halt", "read-only"}:
    print(f"trusted-controller status has unsupported action: {action}")
    sys.exit(1)
for field in [
    "read_only",
    "model_execution_allowed",
    "write_allowed",
    "commit_allowed",
    "push_allowed",
    "github_mutation_allowed",
]:
    if not isinstance(payload.get(field), bool):
        print(f"trusted-controller status field must be boolean: {field}")
        sys.exit(1)
if not isinstance(payload.get("push_attempts"), int) or payload.get("push_attempts") < 0:
    print("trusted-controller push_attempts must be a non-negative integer")
    sys.exit(1)
if status != "ok" or action != "continue" or protected_paths:
    print(reason)
    sys.exit(1)
checks = {
    "startup": ["model_execution_allowed", "write_allowed", "commit_allowed", "push_allowed", "github_mutation_allowed"],
    "model": ["model_execution_allowed"],
    "write": ["write_allowed"],
    "commit-push": ["write_allowed", "commit_allowed", "push_allowed", "github_mutation_allowed"],
}
for field in checks.get(purpose, checks["startup"]):
    if payload.get(field) is not True:
        print(f"trusted-controller denied {purpose}: {field}=false; {reason}")
        sys.exit(1)
print(reason)
PY
  )"; then
    fail_closed "trusted-controller-blocked" "$validation" 32
  fi
  printf 'grimoire-cast: trusted-controller allows %s: %s\n' "$purpose" "$validation"
}

run_stage() {
  local name="$1"
  shift
  printf 'grimoire-cast: stage %s start\n' "$name"
  if "$@"; then
    printf 'grimoire-cast: stage %s complete\n' "$name"
    return 0
  fi
  printf 'grimoire-cast: stage %s failed closed\n' "$name" >&2
  return 1
}

write_mock_boulder() {
  local session_id="$1"
  local status="$2"
  python3 - "$boulder_json" "$session_id" "$status" <<'PY'
import json
import pathlib
import sys
from datetime import datetime, timezone
path, session_id, status = sys.argv[1:]
work_id = 'grimoire-task11-mock'
now = datetime.now(timezone.utc).isoformat().replace('+00:00', 'Z')
work = {
    'work_id': work_id,
    'active_plan': '.omo/plans/grimoire.md',
    'plan_name': 'grimoire',
    'status': status,
    'started_at': now,
    'updated_at': now,
    'ended_at': now if status == 'completed' else '',
    'elapsed_ms': 1234 if status == 'completed' else None,
    'session_ids': [f'opencode:{session_id}'],
}
if status != 'completed':
    work.pop('ended_at')
    work.pop('elapsed_ms')
payload = {
    'schema_version': 2,
    'active_work_id': work_id,
    'works': {work_id: work},
    'active_plan': '.omo/plans/grimoire.md',
    'plan_name': 'grimoire',
    'status': status,
    'started_at': now,
    'updated_at': now,
    'ended_at': now if status == 'completed' else '',
    'elapsed_ms': 1234 if status == 'completed' else None,
    'session_ids': [f'opencode:{session_id}'],
}
if status != 'completed':
    payload.pop('ended_at')
    payload.pop('elapsed_ms')
pathlib.Path(path).parent.mkdir(parents=True, exist_ok=True)
pathlib.Path(path).write_text(json.dumps(payload, indent=2, sort_keys=True) + '\n', encoding='utf-8')
PY
}

validate_boulder_completion() {
  local session_id="$1"
  python3 - "$boulder_json" "$session_id" <<'PY'
import json
import pathlib
import sys
path = pathlib.Path(sys.argv[1])
session_id = sys.argv[2]
if not session_id.startswith('ses_'):
    print('boulder session id is missing or invalid', file=sys.stderr)
    sys.exit(1)
if not path.exists():
    print('boulder file missing', file=sys.stderr)
    sys.exit(1)
try:
    payload = json.loads(path.read_text(encoding='utf-8'))
except json.JSONDecodeError as exc:
    print(f'boulder file malformed: {exc}', file=sys.stderr)
    sys.exit(1)
if not isinstance(payload, dict):
    print('boulder payload is not an object', file=sys.stderr)
    sys.exit(1)
if payload.get('schema_version') != 2:
    print('boulder schema_version must be 2', file=sys.stderr)
    sys.exit(1)
works = payload.get('works')
active_work_id = payload.get('active_work_id')
work = None
if isinstance(works, dict) and active_work_id:
    work = works.get(active_work_id)
    if not isinstance(work, dict):
        print('active_work_id does not select a work object', file=sys.stderr)
        sys.exit(1)
elif isinstance(works, dict):
    candidates = [item for item in works.values() if isinstance(item, dict)]
    if len(candidates) != 1:
        print('boulder active work is ambiguous', file=sys.stderr)
        sys.exit(1)
    work = candidates[0]
else:
    work = payload
if active_work_id and work.get('work_id') not in (None, active_work_id):
    print('boulder work_id mismatches active_work_id', file=sys.stderr)
    sys.exit(1)
if work.get('status') != 'completed':
    print('boulder active work is not completed', file=sys.stderr)
    sys.exit(1)
plan_name = str(work.get('plan_name') or payload.get('plan_name') or '').lower()
active_plan = str(work.get('active_plan') or payload.get('active_plan') or '').lower()
work_id = str(work.get('work_id') or active_work_id or '').lower()
if plan_name and 'grimoire' not in plan_name:
    print('boulder completed work is not grimoire plan_name', file=sys.stderr)
    sys.exit(1)
if active_plan and 'grimoire' not in active_plan:
    print('boulder completed work is not grimoire active_plan', file=sys.stderr)
    sys.exit(1)
if not plan_name and not active_plan and 'grimoire' not in work_id:
    print('boulder completed work lacks grimoire identity', file=sys.stderr)
    sys.exit(1)
session_ids = work.get('session_ids') or payload.get('session_ids')
if not isinstance(session_ids, list):
    print('boulder session_ids missing', file=sys.stderr)
    sys.exit(1)
allowed_sessions = {session_id, f'opencode:{session_id}'}
if not any(str(item) in allowed_sessions for item in session_ids):
    print('boulder completed work does not include continued session id', file=sys.stderr)
    sys.exit(1)
elapsed = work.get('elapsed_ms', payload.get('elapsed_ms'))
if not isinstance(elapsed, (int, float)) or elapsed < 0:
    print('boulder completed work missing elapsed metadata', file=sys.stderr)
    sys.exit(1)
print('grimoire-cast: boulder completion validated')
PY
}

validate_fix_status() {
  python3 - "$FIX_STATUS" <<'PY'
import json
import pathlib
import sys
path = pathlib.Path(sys.argv[1])
if not path.exists():
    print('fix status missing', file=sys.stderr)
    sys.exit(1)
try:
    payload = json.loads(path.read_text(encoding='utf-8'))
except json.JSONDecodeError as exc:
    print(f'fix status malformed: {exc}', file=sys.stderr)
    sys.exit(1)
status = payload.get('status')
if status not in {'fixed', 'clear-noop'}:
    print('fix status must be fixed or clear-noop', file=sys.stderr)
    sys.exit(1)
if payload.get('scope_ok') is not True:
    print('fix status scope_ok must be true', file=sys.stderr)
    sys.exit(1)
if payload.get('blocked_reason') not in ('', None):
    print('fix status blocked_reason must be empty', file=sys.stderr)
    sys.exit(1)
if status == 'clear-noop':
    if payload.get('should_commit') is not False or payload.get('should_push') is not False:
        print('clear-noop must disable commit and push', file=sys.stderr)
        sys.exit(1)
    if payload.get('changed_files') not in ([], None):
        print('clear-noop must not carry changed files', file=sys.stderr)
        sys.exit(1)
else:
    if payload.get('should_commit') is not True or payload.get('should_push') is not True:
        print('fixed must request commit and push before driver guards', file=sys.stderr)
        sys.exit(1)
print(status)
PY
}

emit_scoped_mutation_paths() {
  python3 - "$FIX_STATUS" <<'PY'
import json
import pathlib
import subprocess
import sys


def reject(message):
    print(message, file=sys.stderr)
    sys.exit(1)


fix_status_path = pathlib.Path(sys.argv[1])
try:
    payload = json.loads(fix_status_path.read_text(encoding='utf-8'))
except FileNotFoundError:
    reject('fix status missing')
except json.JSONDecodeError as exc:
    reject(f'fix status malformed: {exc}')

declared = payload.get('changed_files')
if not isinstance(declared, list):
    reject('fix status changed_files must be an array')

normalized_declared = []
for item in declared:
    if not isinstance(item, str) or not item:
        reject('fix status changed_files entries must be non-empty strings')
    normalized = item[2:] if item.startswith('./') else item
    if normalized in {'', '.'} or normalized.startswith('/'):
        reject(f'fix status changed_files entry is not repo-relative: {item}')
    parts = pathlib.PurePosixPath(normalized).parts
    if '..' in parts:
        reject(f'fix status changed_files entry escapes repo: {item}')
    normalized_declared.append(normalized)

if not normalized_declared:
    reject('fixed status changed_files is empty')

declared_set = set(normalized_declared)
try:
    status_bytes = subprocess.check_output(
        ['git', 'status', '--porcelain=v1', '-z', '--untracked-files=all'],
    )
except subprocess.CalledProcessError as exc:
    reject(f'git status failed: {exc}')

fields = status_bytes.decode('utf-8', 'surrogateescape').split('\0')
if fields and fields[-1] == '':
    fields.pop()

actual_paths = []
field_index = 0
while field_index < len(fields):
    entry = fields[field_index]
    if len(entry) < 4 or entry[2] != ' ':
        reject(f'unexpected git status entry: {entry!r}')
    status = entry[:2]
    path = entry[3:]
    if path:
        actual_paths.append(path)
    field_index += 1
    if 'R' in status or 'C' in status:
        if field_index >= len(fields):
            reject('git status rename/copy entry missing source path')
        original_path = fields[field_index]
        if original_path:
            actual_paths.append(original_path)
        field_index += 1

actual_set = set(actual_paths)
if not actual_set:
    reject('fixed status has no working-tree changes to commit')

extra_paths = sorted(actual_set - declared_set)
if extra_paths:
    reject('working-tree changes outside Task 8 changed_files: ' + ', '.join(extra_paths))

emitted = set()
for path in actual_paths:
    if path in emitted:
        continue
    sys.stdout.buffer.write(path.encode('utf-8', 'surrogateescape') + b'\0')
    emitted.add(path)
PY
}

termination_approved() {
  if [ ! -s "$VERDICT_JSON" ]; then
    printf 'grimoire-cast: verdict JSON missing: %s\n' "$VERDICT_JSON" >&2
    return 1
  fi
  if ! command -v jq >/dev/null 2>&1; then
    printf 'grimoire-cast: jq is required for Task 9 termination expression\n' >&2
    return 1
  fi
  local jq_expr
  jq_expr="$(bash "$VERIFY_SCRIPT" --jq-expression)"
  jq -e "$jq_expr" "$VERDICT_JSON" >/dev/null
}

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
  local pat_source="$1"
  case "$pat_source" in
    GRIMOIRE_PAT)
      [ -n "${GRIMOIRE_PAT:-}" ] || return 1
      printf '%s' "$GRIMOIRE_PAT"
      ;;
    CODEX_LOOP_PAT)
      [ -n "${CODEX_LOOP_PAT:-}" ] || return 1
      printf '%s' "$CODEX_LOOP_PAT"
      ;;
    *)
      return 1
      ;;
  esac
}

mask_github_actions_value() {
  local value="$1"
  if [ "${GITHUB_ACTIONS:-}" = "true" ] && [ -n "$value" ]; then
    printf '::add-mask::%s\n' "$value"
  fi
}

push_with_pat_source() {
  local pat_source="$1"
  local refspec="$2"
  local pat_value
  local encoded_auth
  local extraheader

  if ! pat_value="$(pat_value_for_source "$pat_source")"; then
    return 1
  fi
  if ! encoded_auth="$(GRIMOIRE_SELECTED_PAT_VALUE="$pat_value" python3 - <<'PY'
import base64
import os
import sys

token = os.environ.get("GRIMOIRE_SELECTED_PAT_VALUE", "")
if not token:
    sys.exit(1)
sys.stdout.write(base64.b64encode(f"x-access-token:{token}".encode("utf-8")).decode("ascii"))
PY
  )"; then
    return 1
  fi
  extraheader="AUTHORIZATION: basic ${encoded_auth}"
  mask_github_actions_value "$pat_value"
  mask_github_actions_value "$encoded_auth"
  mask_github_actions_value "$extraheader"

  GIT_CONFIG_COUNT=1 \
    GIT_CONFIG_KEY_0="http.https://github.com/.extraheader" \
    GIT_CONFIG_VALUE_0="$extraheader" \
    git push origin "$refspec"
}

preflight_real() {
  local blockers=()
  [ -n "${AI_RELAY_API_KEY:-}" ] || blockers+=("AI_RELAY_API_KEY is not set")
  if ! select_pat_source >/dev/null 2>&1; then
    blockers+=("PAT source is unavailable")
  fi
  [ "${GRIMOIRE_FIX_READY:-}" = "1" ] || blockers+=("GRIMOIRE_FIX_READY=1 readiness gate is not set")
  [ "${GRIMOIRE_BOULDER_READY:-}" = "1" ] || blockers+=("GRIMOIRE_BOULDER_READY=1 readiness gate is not set")
  [ "${GRIMOIRE_VERIFY_READY:-}" = "1" ] || blockers+=("GRIMOIRE_VERIFY_READY=1 readiness gate is not set")
  command -v opencode >/dev/null 2>&1 || blockers+=("opencode CLI is not available")
  [ -n "${GRIMOIRE_PR_NUMBER:-}" ] || blockers+=("GRIMOIRE_PR_NUMBER is not set")
  [ -n "${GRIMOIRE_HEAD_REPO:-}" ] || blockers+=("GRIMOIRE_HEAD_REPO is not set")
  [ -n "${GRIMOIRE_HEAD_REF:-}" ] || blockers+=("GRIMOIRE_HEAD_REF is not set")
  [ -n "${GRIMOIRE_HEAD_SHA:-}" ] || blockers+=("GRIMOIRE_HEAD_SHA is not set")
  [ -n "${GRIMOIRE_BASE_REPO:-}" ] || blockers+=("GRIMOIRE_BASE_REPO is not set")
  [ -n "${GRIMOIRE_BASE_REF:-}" ] || blockers+=("GRIMOIRE_BASE_REF is not set")
  if [ "${#blockers[@]}" -gt 0 ]; then
    local joined
    joined="$(printf '%s; ' "${blockers[@]}")"
    fail_closed "real-preflight-blocked" "$joined" 31
  fi
}

opencode_output_has_error() {
  local output_path="$1"
  python3 - "$output_path" <<'PY'
import json
import pathlib
import sys
path = pathlib.Path(sys.argv[1])
for line in path.read_text(encoding='utf-8', errors='replace').splitlines():
    stripped = line.strip()
    if not stripped:
        continue
    try:
        event = json.loads(stripped)
    except json.JSONDecodeError:
        lowered = stripped.lower()
        if 'error' in lowered or 'missing api key' in lowered:
            print('non-json error text found', file=sys.stderr)
            sys.exit(1)
        continue
    stack = [event]
    while stack:
        value = stack.pop()
        if isinstance(value, dict):
            typ = str(value.get('type', '')).lower()
            level = str(value.get('level', '')).lower()
            message = str(value.get('message') or value.get('error') or '').lower()
            if typ == 'error' or level == 'error' or 'missing api key' in message:
                print('opencode JSON error event found', file=sys.stderr)
                sys.exit(1)
            stack.extend(value.values())
        elif isinstance(value, list):
            stack.extend(value)
sys.exit(0)
PY
}

extract_session_id() {
  local output_path="$1"
  python3 - "$output_path" <<'PY'
import pathlib
import re
import sys
text = pathlib.Path(sys.argv[1]).read_text(encoding='utf-8', errors='replace')
match = re.search(r'\bses_[A-Za-z0-9_-]+\b', text)
if not match:
    print('session id not found', file=sys.stderr)
    sys.exit(1)
print(match.group(0))
PY
}

real_start_boulder() {
  local output_path="$1"
  local prompt
  prompt="/start-work\n\nContinue the grimoire Task 11 fix handoff from ${FIX_STATUS}. Complete only the grimoire boulder work for this PR. Do not commit, push, comment, label, merge, dispatch workflows, or reveal secrets."
  if ! opencode run \
    --format json \
    --dir "$repo_root" \
    --agent atlas \
    --model "${OPENCODE_MODEL:-ai-relay/gpt-5.5}" \
    --variant "${OPENCODE_VARIANT:-xhigh}" \
    "$prompt" > "$output_path" 2>&1; then
    fail_closed "boulder-start-failed" "opencode run exited nonzero while starting boulder" 32
  fi
  if ! opencode_output_has_error "$output_path"; then
    fail_closed "boulder-start-failed" "opencode start output contained error events" 32
  fi
  extract_session_id "$output_path"
}

real_continue_boulder_until_complete() {
  local session_id="$1"
  local started
  local now
  local output_path
  started="$(date +%s)"
  while :; do
    if validate_boulder_completion "$session_id" >/dev/null 2>&1; then
      validate_boulder_completion "$session_id"
      return 0
    fi
    now="$(date +%s)"
    if [ $((now - started)) -ge "$liveness_timeout_seconds" ]; then
      fail_closed "boulder-liveness-timeout" "boulder did not complete before wall-clock timeout" 33
    fi
    output_path=".omo/ci/grimoire-cast-opencode-continue-${now}.jsonl"
    if ! opencode run \
      --format json \
      --dir "$repo_root" \
      --agent atlas \
      --session "$session_id" \
      --model "${OPENCODE_MODEL:-ai-relay/gpt-5.5}" \
      --variant "${OPENCODE_VARIANT:-xhigh}" \
      "Continue grimoire boulder work until .omo/boulder.json records completed status for this session. Do not commit, push, comment, label, merge, dispatch workflows, or reveal secrets." > "$output_path" 2>&1; then
      fail_closed "boulder-continue-failed" "opencode run --session exited nonzero" 34
    fi
    if ! opencode_output_has_error "$output_path"; then
      fail_closed "boulder-continue-failed" "opencode continue output contained error events" 34
    fi
    sleep "$heartbeat_seconds"
  done
}

run_review_design_for_mock_clean() {
  run_stage review bash "$REVIEW_SCRIPT" --mode mock-clean --output "$REVIEW_JSON" || fail_closed "review-failed" "mock clean review failed" 11
  run_stage design bash "$DESIGN_SCRIPT" --mode mock-sufficient --input "$REVIEW_JSON" --output "$SPEC_JSON" --plan "$DESIGN_PLAN" || fail_closed "design-failed" "mock sufficient design failed" 12
  run_stage spec-gap bash "$SPEC_GAP_SCRIPT" --mode render --input "$SPEC_JSON" --comment-output "$SPEC_GAP_COMMENT" --status-output "$SPEC_GAP_STATUS" || fail_closed "spec-gap-failed" "spec-gap clear render failed" 13
}

run_spec_insufficient() {
  local fixture=".omo/ci/grimoire-cast-mock-defect.diff"
  cat > "$fixture" <<'EOF_FIXTURE'
diff --git a/src/demo.rs b/src/demo.rs
--- a/src/demo.rs
+++ b/src/demo.rs
@@ -1 +1 @@
+let issue = "GRIMOIRE_REVIEW_DEFECT";
EOF_FIXTURE
  run_stage review bash "$REVIEW_SCRIPT" --mode mock-defect --fixture "$fixture" --output "$REVIEW_JSON" || fail_closed "review-failed" "mock defect review failed" 11
  if bash "$DESIGN_SCRIPT" --mode mock-insufficient --input "$REVIEW_JSON" --spec ".omo/ci/grimoire-cast-missing-spec.md" --output "$SPEC_JSON" --plan "$DESIGN_PLAN"; then
    fail_closed "design-contract-failed" "mock-insufficient unexpectedly succeeded" 12
  fi
  run_stage spec-gap bash "$SPEC_GAP_SCRIPT" --mode render --input "$SPEC_JSON" --comment-output "$SPEC_GAP_COMMENT" --status-output "$SPEC_GAP_STATUS" || fail_closed "spec-gap-failed" "spec-gap halt render failed" 13
  mark_label_fizzled "spec-insufficient halt"
  write_decision "spec-insufficient-halt" "Task 6 marked specs insufficient and Task 7 rendered halt artifact" 20
  printf 'grimoire-cast: spec insufficient halt recorded\n' >&2
  exit 20
}

run_fix_stage() {
  local fix_mode="$1"
  run_stage fix bash "$FIX_SCRIPT" --mode "$fix_mode" --output "$FIX_STATUS" || fail_closed "fix-failed" "fix stage failed" 14
  validate_fix_status >/dev/null
}

run_boulder_mock() {
  local session_id="$1"
  local boulder_mode="$2"
  case "$boulder_mode" in
    completed)
      write_mock_boulder "$session_id" completed
      ;;
    missing)
      rm -f -- "$boulder_json"
      ;;
    malformed)
      mkdir -p "$(dirname -- "$boulder_json")"
      printf '{not-json\n' > "$boulder_json"
      ;;
    active)
      write_mock_boulder "$session_id" active
      ;;
    session-mismatch)
      write_mock_boulder "ses_other_grimoire_task11" completed
      ;;
  esac
  validate_boulder_completion "$session_id"
}

run_verify_approve() {
  run_stage verify bash "$VERIFY_SCRIPT" --mode mock-approve --output "$VERDICT_JSON" --spec-sufficiency "$SPEC_JSON" --spec-gap-status "$SPEC_GAP_STATUS" --fix-status "$FIX_STATUS" || fail_closed "verify-failed" "mock approve verify failed" 15
  termination_approved || fail_closed "verdict-not-approved" "Task 9 jq all-APPROVE predicate rejected verdict" 16
}

run_verify_missing() {
  run_stage verify bash "$VERIFY_SCRIPT" --mode mock-approve --output "$VERDICT_JSON" --spec-sufficiency "$SPEC_JSON" --spec-gap-status "$SPEC_GAP_STATUS" --fix-status "$FIX_STATUS" || fail_closed "verify-failed" "mock approve verify setup failed" 15
  rm -f -- "$VERDICT_JSON"
  termination_approved || fail_closed "missing-verdict" "verdict missing after verification stage" 16
  fail_closed "missing-verdict-contract-broken" "missing verdict unexpectedly approved" 16
}

run_verify_reject_until_timeout() {
  local started
  local now
  started="$(date +%s)"
  while :; do
    if bash "$VERIFY_SCRIPT" --mode mock-reject --output "$VERDICT_JSON" --spec-sufficiency "$SPEC_JSON" --spec-gap-status "$SPEC_GAP_STATUS" --fix-status "$FIX_STATUS"; then
      :
    fi
    if termination_approved; then
      fail_closed "reject-contract-broken" "reject verdict unexpectedly approved" 16
    fi
    now="$(date +%s)"
    if [ $((now - started)) -ge "$liveness_timeout_seconds" ]; then
      fail_closed "liveness-timeout" "persistent REJECT did not converge before wall-clock timeout" 18
    fi
    sleep "$heartbeat_seconds"
  done
}

commit_and_push_if_allowed() {
  local fix_status="$1"
  if [ "$fix_status" = "clear-noop" ]; then
    mark_label_done
    write_decision "noop-approved" "Task 8 clear-noop and Task 9 all-APPROVE; commit_attempted=false push_attempted=false" 0
    printf 'grimoire-cast: no-op approved; no commit or push attempted\n'
    return 0
  fi
  if [ "$fix_status" != "fixed" ]; then
    fail_closed "invalid-fix-status" "fix status cannot reach mutation path" 17
  fi
  if [ "$dry_run" = "1" ] || [ "$dry_run" = "true" ]; then
    write_decision "fixed-dry-run" "Task 8 fixed and Task 9 all-APPROVE; dry-run recorded one intended bot commit/push path" 0
    printf 'grimoire-cast: fixed dry-run; intended bot commit message: %s\n' "$BOT_COMMIT_MESSAGE"
    return 0
  fi
  validate_trusted_controller_status "commit-push"
  local pat_source
  if ! pat_source="$(select_pat_source)"; then
    fail_closed "mutation-auth-blocked" "PAT source unavailable for fixed mutation path" 35
  fi
  if [ -z "${GRIMOIRE_HEAD_REPO:-}" ] || [ -z "${GRIMOIRE_HEAD_REF:-}" ]; then
    fail_closed "mutation-metadata-blocked" "PR head repo/ref metadata unavailable for fixed mutation path" 35
  fi
  local scoped_paths=()
  if ! while IFS= read -r -d '' scoped_path; do scoped_paths+=("$scoped_path"); done < <(emit_scoped_mutation_paths); then
    fail_closed "mutation-scope-blocked" "working-tree changes must be a non-empty subset of Task 8 changed_files" 37
  fi
  if [ "${#scoped_paths[@]}" -eq 0 ]; then
    fail_closed "mutation-scope-blocked" "working-tree changes must be a non-empty subset of Task 8 changed_files" 37
  fi
  printf 'grimoire-cast: PAT source selected for mutation: %s\n' "$pat_source"
  git config user.name "$BOT_AUTHOR_NAME"
  git config user.email "$BOT_AUTHOR_EMAIL"
  git add -- "${scoped_paths[@]}"
  if git diff --cached --quiet --exit-code; then
    fail_closed "empty-commit-refused" "scoped mutation paths produced no staged diff; refusing empty commit/push" 36
  fi
  git commit -m "$BOT_COMMIT_MESSAGE" --author "$BOT_AUTHOR_NAME <$BOT_AUTHOR_EMAIL>"
  push_with_pat_source "$pat_source" "HEAD:${GRIMOIRE_HEAD_REF}"
  write_decision "fixed-pushed" "Task 8 fixed and Task 9 all-APPROVE; one bot commit/push path completed" 0
}

run_mock_common_until_fix() {
  local fix_mode="$1"
  run_review_design_for_mock_clean
  run_fix_stage "$fix_mode"
}

run_real() {
  validate_trusted_controller_status "model"
  preflight_real
  mask_secret_sources
  run_stage review bash "$REVIEW_SCRIPT" --mode real --output "$REVIEW_JSON" || fail_closed "review-failed" "real review failed closed" 11
  if ! bash "$DESIGN_SCRIPT" --mode real --input "$REVIEW_JSON" --output "$SPEC_JSON" --plan "$DESIGN_PLAN"; then
    run_stage spec-gap bash "$SPEC_GAP_SCRIPT" --mode render --input "$SPEC_JSON" --comment-output "$SPEC_GAP_COMMENT" --status-output "$SPEC_GAP_STATUS" || fail_closed "spec-gap-failed" "real spec-gap render failed" 13
    fail_closed "spec-insufficient-halt" "real design marked specs insufficient; Task 7 artifact rendered" 20
  fi
  run_stage spec-gap bash "$SPEC_GAP_SCRIPT" --mode render --input "$SPEC_JSON" --comment-output "$SPEC_GAP_COMMENT" --status-output "$SPEC_GAP_STATUS" || fail_closed "spec-gap-failed" "real spec-gap clear render failed" 13
  validate_trusted_controller_status "write"
  run_stage fix bash "$FIX_SCRIPT" --mode real --output "$FIX_STATUS" || fail_closed "fix-failed" "real fix status failed closed" 14
  local fix_status
  fix_status="$(validate_fix_status)"
  local start_output=".omo/ci/grimoire-cast-opencode-start.jsonl"
  local session_id
  validate_trusted_controller_status "model"
  session_id="$(real_start_boulder "$start_output")"
  real_continue_boulder_until_complete "$session_id"
  validate_trusted_controller_status "model"
  run_stage verify bash "$VERIFY_SCRIPT" --mode real --output "$VERDICT_JSON" --spec-sufficiency "$SPEC_JSON" --spec-gap-status "$SPEC_GAP_STATUS" --fix-status "$FIX_STATUS" || fail_closed "verify-failed" "real verification failed closed" 15
  termination_approved || fail_closed "verdict-not-approved" "real verdict did not satisfy Task 9 jq all-APPROVE predicate" 16
  commit_and_push_if_allowed "$fix_status"
}

require_python
require_stage_scripts
ensure_dirs
write_loop_metadata
mask_secret_sources
validate_trusted_controller_status "startup"
mark_label_running

case "$mode" in
  real)
    run_real
    ;;
  mock-noop)
    dry_run="1"
    run_mock_common_until_fix mock-noop
    fix_status="$(validate_fix_status)"
    run_boulder_mock "$mock_session_id" completed
    run_verify_approve
    commit_and_push_if_allowed "$fix_status"
    ;;
  mock-fixed)
    dry_run="1"
    run_mock_common_until_fix mock-fix
    fix_status="$(validate_fix_status)"
    run_boulder_mock "$mock_session_id" completed
    run_verify_approve
    commit_and_push_if_allowed "$fix_status"
    ;;
  mock-spec-insufficient)
    dry_run="1"
    run_spec_insufficient
    ;;
  mock-stage-failure)
    dry_run="1"
    if bash "$REVIEW_SCRIPT" --mode mock-defect --output "$REVIEW_JSON"; then
      fail_closed "stage-failure-contract-broken" "stage failure fixture unexpectedly succeeded" 11
    fi
    fail_closed "stage-failure" "review stage failed closed before downstream stages" 11
    ;;
  mock-missing-verdict)
    dry_run="1"
    run_mock_common_until_fix mock-noop >/dev/null
    run_boulder_mock "$mock_session_id" completed
    run_verify_missing
    ;;
  mock-reject)
    dry_run="1"
    run_mock_common_until_fix mock-noop >/dev/null
    run_boulder_mock "$mock_session_id" completed
    run_verify_reject_until_timeout
    ;;
  mock-boulder-missing)
    dry_run="1"
    run_mock_common_until_fix mock-noop >/dev/null
    run_boulder_mock "$mock_session_id" missing || fail_closed "boulder-validation-failed" "missing boulder file failed closed as expected" 19
    fail_closed "boulder-contract-broken" "missing boulder unexpectedly validated" 19
    ;;
  mock-boulder-malformed)
    dry_run="1"
    run_mock_common_until_fix mock-noop >/dev/null
    run_boulder_mock "$mock_session_id" malformed || fail_closed "boulder-validation-failed" "malformed boulder file failed closed as expected" 19
    fail_closed "boulder-contract-broken" "malformed boulder unexpectedly validated" 19
    ;;
  mock-boulder-active)
    dry_run="1"
    run_mock_common_until_fix mock-noop >/dev/null
    run_boulder_mock "$mock_session_id" active || fail_closed "boulder-validation-failed" "non-completed boulder status failed closed as expected" 19
    fail_closed "boulder-contract-broken" "active boulder unexpectedly validated" 19
    ;;
  mock-boulder-session-mismatch)
    dry_run="1"
    run_mock_common_until_fix mock-noop >/dev/null
    run_boulder_mock "$mock_session_id" session-mismatch || fail_closed "boulder-validation-failed" "session-mismatched boulder failed closed as expected" 19
    fail_closed "boulder-contract-broken" "session-mismatched boulder unexpectedly validated" 19
    ;;
esac
