#!/usr/bin/env bash
set -euo pipefail

readonly DEFAULT_MODE="dry-run"
readonly DEFAULT_STATUS_OUTPUT=".omo/ci/grimoire-label-status.json"
readonly DEFAULT_STATE_FILE=".omo/ci/grimoire-label-state.txt"

mode="${GRIMOIRE_LABEL_MODE:-$DEFAULT_MODE}"
transition="${GRIMOIRE_LABEL_STATE:-}"
repo_root="${GITHUB_WORKSPACE:-$(pwd)}"
repo="${GRIMOIRE_BASE_REPO:-${GITHUB_REPOSITORY:-}}"
pr_number="${GRIMOIRE_PR_NUMBER:-}"
state_file="${GRIMOIRE_LABEL_STATE_FILE:-$DEFAULT_STATE_FILE}"
state_output="${GRIMOIRE_LABEL_STATE_OUTPUT:-}"
status_output="${GRIMOIRE_LABEL_STATUS_OUTPUT:-$DEFAULT_STATUS_OUTPUT}"

usage() {
  cat <<'USAGE'
Usage: grimoire-labels.sh [options] [running|done|fizzled]

Display-only PR label lifecycle helper for grimoire Task 19.

Options:
  --state STATE          running, done, or fizzled. May also be positional.
  --mode MODE            dry-run, local, or live. Default: dry-run.
  --repo-root PATH       Repository or artifact root for local outputs.
  --repo OWNER/REPO      GitHub repository for live mode.
  --pr-number NUMBER     Pull request number for live mode.
  --state-file PATH      Newline-delimited local input labels. Default: .omo/ci/grimoire-label-state.txt.
  --state-output PATH    Newline-delimited local output labels. Defaults to state-file in local mode.
  --status-output PATH   JSON transition report. Default: .omo/ci/grimoire-label-status.json.
  --help                 Show this help text.

Live mode uses only GRIMOIRE_PAT or CODEX_LOOP_PAT, derives GH_TOKEN for each gh
call from that PAT, ensures the three grimoire labels exist, and manages only
those three labels on the pull request. Labels are display-only and are never a
loop state source.
USAGE
}

fail_usage() {
  printf 'grimoire-labels: %s\n\n' "$1" >&2
  usage >&2
  exit 2
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --help|-h)
      usage
      exit 0
      ;;
    --state)
      [ "$#" -ge 2 ] || fail_usage "--state requires a value"
      transition="$2"
      shift 2
      ;;
    --mode)
      [ "$#" -ge 2 ] || fail_usage "--mode requires a value"
      mode="$2"
      shift 2
      ;;
    --repo-root)
      [ "$#" -ge 2 ] || fail_usage "--repo-root requires a value"
      repo_root="$2"
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
    --state-file)
      [ "$#" -ge 2 ] || fail_usage "--state-file requires a value"
      state_file="$2"
      shift 2
      ;;
    --state-output)
      [ "$#" -ge 2 ] || fail_usage "--state-output requires a value"
      state_output="$2"
      shift 2
      ;;
    --status-output)
      [ "$#" -ge 2 ] || fail_usage "--status-output requires a value"
      status_output="$2"
      shift 2
      ;;
    --*)
      fail_usage "unknown option: $1"
      ;;
    *)
      if [ -n "$transition" ]; then
        fail_usage "unexpected argument: $1"
      fi
      transition="$1"
      shift
      ;;
  esac
done

case "$transition" in
  running|done|fizzled)
    ;;
  '')
    fail_usage "state is required"
    ;;
  *)
    fail_usage "state must be running, done, or fizzled"
    ;;
esac

case "$mode" in
  dry-run|local|live)
    ;;
  '')
    mode="$DEFAULT_MODE"
    ;;
  *)
    fail_usage "--mode must be dry-run, local, or live"
    ;;
esac

if ! command -v python3 >/dev/null 2>&1; then
  printf 'grimoire-labels: python3 is required\n' >&2
  exit 127
fi

cd "$repo_root"

if [ -z "$state_output" ] && [ "$mode" = "local" ]; then
  state_output="$state_file"
fi

resolve_path() {
  local path="$1"
  if [ -z "$path" ]; then
    return 0
  fi
  case "$path" in
    /*) printf '%s\n' "$path" ;;
    *) printf '%s/%s\n' "$repo_root" "$path" ;;
  esac
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
  case "$1" in
    GRIMOIRE_PAT) printf '%s\n' "${GRIMOIRE_PAT}" ;;
    CODEX_LOOP_PAT) printf '%s\n' "${CODEX_LOOP_PAT}" ;;
    *) return 1 ;;
  esac
}

ensure_live_preconditions() {
  if [ -z "$repo" ]; then
    printf 'grimoire-labels: --repo or GRIMOIRE_BASE_REPO/GITHUB_REPOSITORY is required for live mode\n' >&2
    exit 1
  fi
  if [ -z "$pr_number" ]; then
    printf 'grimoire-labels: --pr-number or GRIMOIRE_PR_NUMBER is required for live mode\n' >&2
    exit 1
  fi
  if ! selected_pat_source="$(select_pat_source)"; then
    printf 'grimoire-labels: GRIMOIRE_PAT or CODEX_LOOP_PAT is required for live mode\n' >&2
    exit 1
  fi
  if ! command -v gh >/dev/null 2>&1; then
    printf 'grimoire-labels: gh is required for live mode\n' >&2
    exit 127
  fi
}

ensure_live_labels() {
  local token="$1"
  local label_snapshot
  label_snapshot="$(mktemp "${RUNNER_TEMP:-/tmp}/grimoire-label-definitions.XXXXXX")"
  GH_TOKEN="$token" gh label list --repo "$repo" --limit 1000 --json name,color,description > "$label_snapshot"
  while IFS=$'\t' read -r name color description; do
    [ -n "$name" ] || continue
    GH_TOKEN="$token" gh label create "$name" --repo "$repo" --color "$color" --description "$description" --force >/dev/null
  done < <(python3 - "$label_snapshot" <<'PY'
import json
import pathlib
import sys

snapshot = json.loads(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))
existing = {}
for item in snapshot:
    if isinstance(item, dict) and isinstance(item.get("name"), str):
        existing[item["name"]] = {
            "color": str(item.get("color") or "").lstrip("#").lower(),
            "description": str(item.get("description") or ""),
        }

desired = [
    ("🔮 Casting…", "7c3aed", "Grimoire review/autofix loop is running."),
    ("✨ Cast", "10b981", "Grimoire review/autofix loop completed cleanly."),
    ("💨 Fizzled", "6b7280", "Grimoire review/autofix loop halted or failed closed."),
]
for name, color, description in desired:
    current = existing.get(name)
    if current is None or current["color"] != color or current["description"] != description:
        print(f"{name}\t{color}\t{description}")
PY
  )
}

read_live_labels() {
  local token="$1"
  local output="$2"
  GH_TOKEN="$token" gh pr view "$pr_number" --repo "$repo" --json labels --jq '.labels[].name' > "$output"
}

compute_transition() {
  local input_path="$1"
  local output_path="$2"
  local report_path="$3"
  local auth_source="$4"
  local ensure_attempted="$5"
  python3 - "$input_path" "$output_path" "$report_path" "$transition" "$mode" "$repo" "$pr_number" "$auth_source" "$ensure_attempted" <<'PY'
import json
import pathlib
import sys
from datetime import datetime, timezone

(
    input_arg,
    output_arg,
    report_arg,
    transition,
    mode,
    repo,
    pr_number,
    auth_source,
    ensure_attempted,
) = sys.argv[1:]

LABELS = {
    "casting": {
        "name": "🔮 Casting…",
        "color": "#7c3aed",
        "description": "Grimoire review/autofix loop is running.",
    },
    "cast": {
        "name": "✨ Cast",
        "color": "#10b981",
        "description": "Grimoire review/autofix loop completed cleanly.",
    },
    "fizzled": {
        "name": "💨 Fizzled",
        "color": "#6b7280",
        "description": "Grimoire review/autofix loop halted or failed closed.",
    },
}
MANAGED = [LABELS["casting"]["name"], LABELS["cast"]["name"], LABELS["fizzled"]["name"]]

input_path = pathlib.Path(input_arg) if input_arg else None
output_path = pathlib.Path(output_arg) if output_arg else None
report_path = pathlib.Path(report_arg)

current = []
if input_path and input_path.exists():
    for line in input_path.read_text(encoding="utf-8", errors="replace").splitlines():
        label = line.strip()
        if label and label not in current:
            current.append(label)

final = list(current)
operations = []
notes = []


def remove(label, reason):
    if label in final:
        final.remove(label)
        operations.append({"action": "remove", "label": label, "reason": reason})
    else:
        notes.append(f"remove skipped for absent label: {label}")


def add(label, reason):
    if label in final:
        notes.append(f"add skipped for existing label: {label}")
    else:
        final.append(label)
        operations.append({"action": "add", "label": label, "reason": reason})

if transition == "running":
    if LABELS["cast"]["name"] in final or LABELS["fizzled"]["name"] in final:
        notes.append("running skipped because a terminal grimoire label is already present")
    elif LABELS["casting"]["name"] in final:
        notes.append("running skipped because Casting is already present")
    else:
        add(LABELS["casting"]["name"], "running transition adds Casting")
elif transition == "done":
    remove(LABELS["casting"]["name"], "done transition removes running label")
    remove(LABELS["fizzled"]["name"], "done transition removes halted label")
    add(LABELS["cast"]["name"], "done transition adds Cast")
elif transition == "fizzled":
    remove(LABELS["casting"]["name"], "fizzled transition removes running label")
    remove(LABELS["cast"]["name"], "fizzled transition removes success label")
    add(LABELS["fizzled"]["name"], "fizzled transition adds Fizzled")
else:
    raise SystemExit(f"unsupported transition: {transition}")

if output_path:
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text("\n".join(final) + ("\n" if final else ""), encoding="utf-8")

report = {
    "schema_version": 1,
    "stage": "grimoire-labels",
    "generated_at": datetime.now(timezone.utc).isoformat().replace("+00:00", "Z"),
    "mode": mode,
    "transition": transition,
    "repo": repo,
    "pr_number": pr_number,
    "auth_source": auth_source,
    "uses_default_actions_token": False,
    "labels_are_display_only": True,
    "durable_loop_state_source": False,
    "managed_labels": [LABELS["casting"], LABELS["cast"], LABELS["fizzled"]],
    "unrelated_labels_preserved": sorted(label for label in final if label not in MANAGED),
    "current_labels": current,
    "final_labels": final,
    "operations": operations,
    "operation_count": len(operations),
    "changed": bool(operations),
    "repository_label_ensure_attempted": ensure_attempted == "true",
    "github_pr_edit_commands_planned": 1 if mode == "live" and operations else 0,
    "github_pr_label_mutation_attempted": mode == "live" and bool(operations),
    "notes": notes,
}
report_path.parent.mkdir(parents=True, exist_ok=True)
report_path.write_text(json.dumps(report, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY
}

apply_live_operations() {
  local token="$1"
  local report_path="$2"
  local -a gh_args
  gh_args=(pr edit "$pr_number" --repo "$repo")
  while IFS=$'\t' read -r action label; do
    [ -n "$action" ] || continue
    case "$action" in
      add)
        gh_args+=(--add-label "$label")
        ;;
      remove)
        gh_args+=(--remove-label "$label")
        ;;
      *)
        printf 'grimoire-labels: unsupported live operation: %s\n' "$action" >&2
        exit 1
        ;;
    esac
  done < <(python3 - "$report_path" <<'PY'
import json
import pathlib
import sys
payload = json.loads(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))
for item in payload.get("operations", []):
    print(f"{item['action']}\t{item['label']}")
PY
  )
  if [ "${#gh_args[@]}" -gt 5 ]; then
    GH_TOKEN="$token" gh "${gh_args[@]}" >/dev/null
  fi
}

abs_state_file="$(resolve_path "$state_file")"
abs_state_output="$(resolve_path "$state_output")"
abs_status_output="$(resolve_path "$status_output")"
selected_pat_source=""
selected_pat=""
input_labels="$abs_state_file"
computed_state_output="$abs_state_output"
status_tmp="$abs_status_output"
ensure_attempted="false"

if [ "$mode" = "live" ]; then
  ensure_live_preconditions
  selected_pat="$(pat_value_for_source "$selected_pat_source")"
  if [ -n "${GITHUB_ACTIONS:-}" ]; then
    printf '::add-mask::%s\n' "$selected_pat"
  fi
  ensure_live_labels "$selected_pat"
  ensure_attempted="true"
  input_labels="$(mktemp "${RUNNER_TEMP:-/tmp}/grimoire-labels-current.XXXXXX")"
  read_live_labels "$selected_pat" "$input_labels"
  if [ -z "$computed_state_output" ]; then
    computed_state_output="$(mktemp "${RUNNER_TEMP:-/tmp}/grimoire-labels-final.XXXXXX")"
  fi
  status_tmp="$(mktemp "${RUNNER_TEMP:-/tmp}/grimoire-labels-status.XXXXXX")"
fi

compute_transition "$input_labels" "$computed_state_output" "$status_tmp" "$selected_pat_source" "$ensure_attempted"

if [ "$mode" = "live" ]; then
  apply_live_operations "$selected_pat" "$status_tmp"
  mkdir -p "$(dirname -- "$abs_status_output")"
  cp "$status_tmp" "$abs_status_output"
fi

printf 'grimoire-labels: transition=%s mode=%s status=%s\n' "$transition" "$mode" "$status_output"
