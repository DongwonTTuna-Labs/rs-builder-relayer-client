#!/usr/bin/env bash
set -euo pipefail

readonly DEFAULT_OUTPUT=".omo/grimoire/verdict.json"
readonly DEFAULT_SPEC_SUFFICIENCY=".omo/ci/spec-sufficiency.json"
readonly DEFAULT_SPEC_GAP_STATUS=".omo/ci/spec-gap-status.json"
readonly DEFAULT_FIX_STATUS=".omo/ci/fix-status.json"
readonly DEFAULT_MODEL="ai-relay/gpt-5.5"
readonly DEFAULT_VARIANT="xhigh"
readonly JQ_APPROVE_EXPR='type == "object" and .schema_version == 1 and .stage == "grimoire-verify" and (.notes | type == "object") and (.notes.f1_oracle | type == "object") and (.notes.f2_quality | type == "object") and (.notes.f3_real_qa | type == "object") and (.notes.f4_scope | type == "object") and .f1_oracle == "APPROVE" and .f2_quality == "APPROVE" and .f3_real_qa == "APPROVE" and .f4_scope == "APPROVE" and .approved == true'

mode="${GRIMOIRE_VERIFY_MODE:-real}"
output_path="${GRIMOIRE_VERIFY_OUTPUT:-${DEFAULT_OUTPUT}}"
input_path="${GRIMOIRE_VERIFY_INPUT:-}"
repo_root="${GITHUB_WORKSPACE:-$(pwd)}"
spec_sufficiency_path="${GRIMOIRE_SPEC_SUFFICIENCY:-${DEFAULT_SPEC_SUFFICIENCY}}"
spec_gap_status_path="${GRIMOIRE_SPEC_GAP_STATUS:-${DEFAULT_SPEC_GAP_STATUS}}"
fix_status_path="${GRIMOIRE_FIX_STATUS:-${DEFAULT_FIX_STATUS}}"

usage() {
  cat <<'USAGE'
Usage: grimoire-verify.sh [options]

F1-F4 grimoire verification stage. Writes and validates the machine-readable
Task 9 verdict JSON used by the driver termination gate.

Options:
  --mode MODE                real, mock-approve, mock-reject, mock-invalid,
                             or validate. Default: real.
  --output PATH              Verdict output path. Default: .omo/grimoire/verdict.json.
  --input PATH               Verdict input path for validate mode. Default: --output.
  --validate PATH            Validate PATH with the driver all-APPROVE predicate.
  --jq-expression            Print the exact jq predicate used for termination.
  --spec-sufficiency PATH    Task 6 JSON prerequisite. Default: .omo/ci/spec-sufficiency.json.
  --spec-gap-status PATH     Task 7 JSON prerequisite. Default: .omo/ci/spec-gap-status.json.
  --fix-status PATH          Task 8 JSON prerequisite. Default: .omo/ci/fix-status.json.
  --repo-root PATH           Repository root for real-mode opencode execution.
  --help                     Show this help text.

Verdict JSON contract:
  Required root fields: schema_version, stage, approved, f1_oracle,
  f2_quality, f3_real_qa, f4_scope, and notes. Each F field is an enum:
  APPROVE or REJECT. The script derives approved=true only when all four
  F fields are exactly APPROVE.

Driver jq predicate:
  Use --jq-expression to print the stable predicate. The predicate requires
  schema_version=1, stage="grimoire-verify", a notes object with per-lens
  objects, all four F fields exactly APPROVE, and approved=true. Missing files,
  malformed JSON, missing keys, invalid verdicts, and any REJECT fail closed.

Modes:
  mock-approve  Deterministic local mode. Writes all F fields APPROVE and exits 0.
  mock-reject   Deterministic local mode. Writes a valid REJECT verdict and exits nonzero.
  mock-invalid  Deterministic local negative fixture. Writes malformed contract JSON and exits nonzero.
  validate      Does not write. Validates --input/--output using jq when available,
                otherwise a Python equivalent of the same all-APPROVE predicate.
  real          Validates Task 6/7/8 machine prerequisites before any model call,
                then requires AI_RELAY_API_KEY, opencode, and
                GRIMOIRE_VERIFY_READY=1. Missing or malformed prerequisites write
                a REJECT/blocked artifact and exit nonzero.
USAGE
}

fail_usage() {
  printf 'grimoire-verify: %s\n\n' "$1" >&2
  usage >&2
  exit 2
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --help|-h)
      usage
      exit 0
      ;;
    --jq-expression)
      printf '%s\n' "$JQ_APPROVE_EXPR"
      exit 0
      ;;
    --mode)
      [ "$#" -ge 2 ] || fail_usage "--mode requires a value"
      mode="$2"
      shift 2
      ;;
    --output)
      [ "$#" -ge 2 ] || fail_usage "--output requires a value"
      output_path="$2"
      shift 2
      ;;
    --input)
      [ "$#" -ge 2 ] || fail_usage "--input requires a value"
      input_path="$2"
      shift 2
      ;;
    --validate)
      [ "$#" -ge 2 ] || fail_usage "--validate requires a value"
      mode="validate"
      input_path="$2"
      shift 2
      ;;
    --spec-sufficiency)
      [ "$#" -ge 2 ] || fail_usage "--spec-sufficiency requires a value"
      spec_sufficiency_path="$2"
      shift 2
      ;;
    --spec-gap-status)
      [ "$#" -ge 2 ] || fail_usage "--spec-gap-status requires a value"
      spec_gap_status_path="$2"
      shift 2
      ;;
    --fix-status)
      [ "$#" -ge 2 ] || fail_usage "--fix-status requires a value"
      fix_status_path="$2"
      shift 2
      ;;
    --repo-root)
      [ "$#" -ge 2 ] || fail_usage "--repo-root requires a value"
      repo_root="$2"
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
  real|mock-approve|mock-reject|mock-invalid|validate)
    ;;
  *)
    fail_usage "unsupported mode: ${mode}"
    ;;
esac

if ! command -v python3 >/dev/null 2>&1; then
  printf 'grimoire-verify: python3 is required\n' >&2
  exit 127
fi

write_contract() {
  python3 - "$@" <<'PY'
import json
import pathlib
import sys
from datetime import datetime, timezone

COMMAND = sys.argv[1]
OUTPUT = pathlib.Path(sys.argv[2])
REPO_ROOT = pathlib.Path(sys.argv[3])
SPEC_SUFFICIENCY = pathlib.Path(sys.argv[4])
SPEC_GAP_STATUS = pathlib.Path(sys.argv[5])
FIX_STATUS = pathlib.Path(sys.argv[6])
EXTRA = sys.argv[7:]
STAGE = "grimoire-verify"
LENSES = ["f1_oracle", "f2_quality", "f3_real_qa", "f4_scope"]
ENUM_VALUES = {"APPROVE", "REJECT"}


def utc_now():
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def path_for_output(path):
    try:
        resolved = path.resolve()
        root = REPO_ROOT.resolve()
        return resolved.relative_to(root).as_posix()
    except (OSError, ValueError):
        return path.as_posix()


def prereq_paths():
    return {
        "spec_sufficiency": path_for_output(SPEC_SUFFICIENCY),
        "spec_gap_status": path_for_output(SPEC_GAP_STATUS),
        "fix_status": path_for_output(FIX_STATUS),
    }


def derive_approved(statuses):
    return all(statuses.get(lens) == "APPROVE" for lens in LENSES)


def note(status, summary, evidence, reviewer="grimoire-verify"):
    return {
        "status": status,
        "summary": summary,
        "evidence": evidence,
        "reviewer": reviewer,
    }


def normalize_note(raw, lens, status):
    if isinstance(raw, dict):
        result = dict(raw)
        result.setdefault("status", status)
        result.setdefault("summary", f"{lens} returned {status}")
        result.setdefault("evidence", [])
        result.setdefault("reviewer", "grimoire-real")
        if not isinstance(result["evidence"], list):
            result["evidence"] = [str(result["evidence"])]
        return result
    return note(status, f"{lens} returned {status}", [], "grimoire-real")


def make_payload(mode, statuses, notes, blocked_reason="", blockers=None, model_called=False, real_mode_attempted=False):
    payload = {
        "schema_version": 1,
        "stage": STAGE,
        "generated_at": utc_now(),
        "mode": mode,
        "approved": derive_approved(statuses),
        "f1_oracle": statuses["f1_oracle"],
        "f2_quality": statuses["f2_quality"],
        "f3_real_qa": statuses["f3_real_qa"],
        "f4_scope": statuses["f4_scope"],
        "notes": notes,
        "prerequisites": prereq_paths(),
        "blocked_reason": blocked_reason,
        "blockers": blockers or [],
        "model_called": model_called,
        "real_mode_attempted": real_mode_attempted,
    }
    return payload


def write_json(payload):
    OUTPUT.parent.mkdir(parents=True, exist_ok=True)
    OUTPUT.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def mock_approve():
    statuses = {lens: "APPROVE" for lens in LENSES}
    notes = {
        "f1_oracle": note("APPROVE", "Mock Oracle compliance accepts the fixture.", ["deterministic mock-approve fixture"]),
        "f2_quality": note("APPROVE", "Mock quality lens accepts the fixture.", ["deterministic mock-approve fixture"]),
        "f3_real_qa": note("APPROVE", "Mock real QA lens accepts the fixture.", ["deterministic mock-approve fixture"]),
        "f4_scope": note("APPROVE", "Mock scope lens accepts the fixture.", ["deterministic mock-approve fixture"]),
    }
    write_json(make_payload("mock-approve", statuses, notes))
    return 0


def mock_reject():
    statuses = {
        "f1_oracle": "APPROVE",
        "f2_quality": "REJECT",
        "f3_real_qa": "REJECT",
        "f4_scope": "APPROVE",
    }
    notes = {
        "f1_oracle": note("APPROVE", "Mock Oracle compliance accepts the fixture.", ["deterministic mock-reject fixture"]),
        "f2_quality": note("REJECT", "Mock quality rejects the fixture to prove non-termination.", ["deterministic REJECT fixture"]),
        "f3_real_qa": note("REJECT", "Mock real QA rejects because live proof is absent.", ["deterministic REJECT fixture"]),
        "f4_scope": note("APPROVE", "Mock scope accepts the fixture.", ["deterministic mock-reject fixture"]),
    }
    write_json(make_payload("mock-reject", statuses, notes, blocked_reason="mock rejection proves fail-closed non-termination"))
    return 0


def mock_invalid():
    OUTPUT.parent.mkdir(parents=True, exist_ok=True)
    payload = {
        "schema_version": 1,
        "stage": STAGE,
        "approved": True,
        "f1_oracle": "APPROVE",
        "f2_quality": "MAYBE",
        "notes": "invalid notes shape",
    }
    OUTPUT.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return 0


def split_blockers(values):
    blockers = []
    for value in values:
        for line in str(value).splitlines():
            line = line.strip()
            if line:
                blockers.append(line)
    return blockers


def blocked(values):
    blockers = split_blockers(values) or ["real verification prerequisites are unavailable"]
    statuses = {lens: "REJECT" for lens in LENSES}
    joined = "; ".join(blockers)
    notes = {
        "f1_oracle": note("REJECT", "Oracle compliance was not run because verification is blocked.", blockers),
        "f2_quality": note("REJECT", "Quality review was not run because verification is blocked.", blockers),
        "f3_real_qa": note("REJECT", "Real QA was not run because verification is blocked.", blockers),
        "f4_scope": note("REJECT", "Scope review was not run because verification is blocked.", blockers),
    }
    write_json(make_payload("real", statuses, notes, blocked_reason=joined, blockers=blockers, model_called=False, real_mode_attempted=True))
    return 0


def load_json(path, label):
    if not path.exists():
        raise ValueError(f"{label} missing: {path}")
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        raise ValueError(f"{label} is not valid JSON: {exc}") from exc


def validate_contract(candidate):
    errors = []
    if not isinstance(candidate, dict):
        return ["verdict is not a JSON object"]
    if candidate.get("schema_version") != 1:
        errors.append("schema_version must equal 1")
    if candidate.get("stage") != STAGE:
        errors.append(f"stage must equal {STAGE}")
    statuses = {}
    for lens in LENSES:
        value = candidate.get(lens)
        if value not in ENUM_VALUES:
            errors.append(f"{lens} must be APPROVE or REJECT")
        else:
            statuses[lens] = value
    notes = candidate.get("notes")
    if not isinstance(notes, dict):
        errors.append("notes must be an object")
    else:
        for lens in LENSES:
            if not isinstance(notes.get(lens), dict):
                errors.append(f"notes.{lens} must be an object")
    if not isinstance(candidate.get("approved"), bool):
        errors.append("approved must be boolean")
    elif len(statuses) == len(LENSES) and candidate.get("approved") is not derive_approved(statuses):
        errors.append("approved must be derived from exact all-APPROVE F fields")
    return errors


def validate_approval():
    try:
        candidate = load_json(OUTPUT, "verdict")
    except ValueError as exc:
        print(str(exc), file=sys.stderr)
        return 1
    errors = validate_contract(candidate)
    if errors:
        for error in errors:
            print(error, file=sys.stderr)
        return 1
    if not derive_approved({lens: candidate[lens] for lens in LENSES}):
        print("verdict is valid but not all-APPROVE", file=sys.stderr)
        return 1
    print(f"grimoire-verify: {OUTPUT} satisfies all-APPROVE predicate")
    return 0


def check_prerequisites():
    blockers = []
    try:
        spec = load_json(SPEC_SUFFICIENCY, "Task 6 spec sufficiency artifact")
        if spec.get("schema_version") != 1:
            blockers.append("Task 6 spec sufficiency schema_version must equal 1")
        if spec.get("stage") != "grimoire-design":
            blockers.append("Task 6 spec sufficiency stage must equal grimoire-design")
        if spec.get("spec_sufficient") is not True:
            blockers.append("Task 6 spec_sufficient must be true")
        if spec.get("halt_reason") not in ("", None):
            blockers.append("Task 6 halt_reason must be empty for verification")
        if not isinstance(spec.get("bindings"), list):
            blockers.append("Task 6 bindings must be an array")
    except ValueError as exc:
        blockers.append(str(exc))

    try:
        gap = load_json(SPEC_GAP_STATUS, "Task 7 spec-gap status artifact")
        if gap.get("schema_version") != 1:
            blockers.append("Task 7 spec-gap status schema_version must equal 1")
        if gap.get("should_halt") is not False:
            blockers.append("Task 7 should_halt must be false")
        if gap.get("should_comment") is not False:
            blockers.append("Task 7 should_comment must be false before verification")
    except ValueError as exc:
        blockers.append(str(exc))

    try:
        fix = load_json(FIX_STATUS, "Task 8 fix status artifact")
        if fix.get("schema_version") != 1:
            blockers.append("Task 8 fix status schema_version must equal 1")
        if fix.get("scope_ok") is not True:
            blockers.append("Task 8 scope_ok must be true")
        if fix.get("status") not in {"fixed", "clear-noop"}:
            blockers.append("Task 8 status must be fixed or clear-noop")
        if fix.get("blocked_reason") not in ("", None):
            blockers.append("Task 8 blocked_reason must be empty")
        if "changed_files" in fix and not isinstance(fix.get("changed_files"), list):
            blockers.append("Task 8 changed_files must be an array when present")
        if "should_commit" in fix and not isinstance(fix.get("should_commit"), bool):
            blockers.append("Task 8 should_commit must be boolean when present")
        if "should_push" in fix and not isinstance(fix.get("should_push"), bool):
            blockers.append("Task 8 should_push must be boolean when present")
    except ValueError as exc:
        blockers.append(str(exc))

    if blockers:
        for blocker in blockers:
            print(blocker, file=sys.stderr)
        return 1
    return 0


def normalize_real():
    try:
        candidate = load_json(OUTPUT, "real verdict candidate")
    except ValueError as exc:
        blocked([str(exc)])
        return 1
    errors = validate_contract(candidate)
    if errors:
        blocked(errors)
        return 1
    statuses = {lens: candidate[lens] for lens in LENSES}
    source_notes = candidate.get("notes", {})
    notes = {lens: normalize_note(source_notes.get(lens), lens, statuses[lens]) for lens in LENSES}
    payload = make_payload(
        "real",
        statuses,
        notes,
        blocked_reason=candidate.get("blocked_reason", ""),
        blockers=candidate.get("blockers", []),
        model_called=True,
        real_mode_attempted=True,
    )
    write_json(payload)
    return 0 if payload["approved"] else 1


if COMMAND == "mock-approve":
    raise SystemExit(mock_approve())
if COMMAND == "mock-reject":
    raise SystemExit(mock_reject())
if COMMAND == "mock-invalid":
    raise SystemExit(mock_invalid())
if COMMAND == "blocked":
    raise SystemExit(blocked(EXTRA))
if COMMAND == "validate":
    raise SystemExit(validate_approval())
if COMMAND == "check-prerequisites":
    raise SystemExit(check_prerequisites())
if COMMAND == "normalize-real":
    raise SystemExit(normalize_real())
print(f"unknown command: {COMMAND}", file=sys.stderr)
raise SystemExit(2)
PY
}

write_blocked() {
  write_contract blocked "$output_path" "$repo_root" "$spec_sufficiency_path" "$spec_gap_status_path" "$fix_status_path" "$@"
}

check_machine_prerequisites() {
  write_contract check-prerequisites "$output_path" "$repo_root" "$spec_sufficiency_path" "$spec_gap_status_path" "$fix_status_path"
}

validate_with_python() {
  local target_path="$1"
  write_contract validate "$target_path" "$repo_root" "$spec_sufficiency_path" "$spec_gap_status_path" "$fix_status_path"
}

validate_approval() {
  local target_path="$1"
  if command -v jq >/dev/null 2>&1; then
    jq -e "$JQ_APPROVE_EXPR" "$target_path" >/dev/null
  else
    validate_with_python "$target_path"
  fi
}

verify_permission_json() {
  cat <<'JSON'
{"bash":"allow","read":"allow","write":"allow","edit":"allow","webfetch":"deny"}
JSON
}

real_verification_prompt() {
  cat <<PROMPT
You are running the grimoire Task 9 F1-F4 verification stage for this repo.

Write exactly one JSON file at:
${output_path}

Do not use GitHub API mutation, gh comment/label/edit commands, push, merge, or token-bearing output.
Do not print, infer, or store secret values. Do not parse free-form text for APPROVE.
Only the JSON file controls the result; the driver validates it with this jq predicate:
${JQ_APPROVE_EXPR}

Machine prerequisites already checked by the shell stage:
- Task 6 spec sufficiency: ${spec_sufficiency_path}
- Task 7 spec-gap status: ${spec_gap_status_path}
- Task 8 fix status: ${fix_status_path}

Return REJECT unless each lens has direct evidence:
- f1_oracle: Oracle plan/spec compliance.
- f2_quality: code/script quality and maintainability.
- f3_real_qa: actual commands/tests/manual surface evidence, not assumptions.
- f4_scope: requested scope fidelity and no forbidden mutation/auth behavior.

Required JSON shape:
{
  "schema_version": 1,
  "stage": "grimoire-verify",
  "approved": false,
  "f1_oracle": "APPROVE or REJECT",
  "f2_quality": "APPROVE or REJECT",
  "f3_real_qa": "APPROVE or REJECT",
  "f4_scope": "APPROVE or REJECT",
  "notes": {
    "f1_oracle": {"status": "APPROVE or REJECT", "summary": "...", "evidence": ["..."]},
    "f2_quality": {"status": "APPROVE or REJECT", "summary": "...", "evidence": ["..."]},
    "f3_real_qa": {"status": "APPROVE or REJECT", "summary": "...", "evidence": ["..."]},
    "f4_scope": {"status": "APPROVE or REJECT", "summary": "...", "evidence": ["..."]}
  }
}

Set approved=true only when all four F fields are exactly APPROVE. If any evidence is missing, use REJECT and explain it in notes.
PROMPT
}

run_mock_approve() {
  write_contract mock-approve "$output_path" "$repo_root" "$spec_sufficiency_path" "$spec_gap_status_path" "$fix_status_path"
  printf 'grimoire-verify: wrote mock APPROVE verdict to %s\n' "$output_path"
}

run_mock_reject() {
  write_contract mock-reject "$output_path" "$repo_root" "$spec_sufficiency_path" "$spec_gap_status_path" "$fix_status_path"
  printf 'grimoire-verify: wrote mock REJECT verdict to %s\n' "$output_path" >&2
  exit 1
}

run_mock_invalid() {
  write_contract mock-invalid "$output_path" "$repo_root" "$spec_sufficiency_path" "$spec_gap_status_path" "$fix_status_path"
  printf 'grimoire-verify: wrote invalid verdict fixture to %s\n' "$output_path" >&2
  exit 1
}

run_validate() {
  local target_path="${input_path:-${output_path}}"
  if validate_approval "$target_path"; then
    printf 'grimoire-verify: %s satisfies all-APPROVE predicate\n' "$target_path"
    exit 0
  fi
  printf 'grimoire-verify: %s does not satisfy all-APPROVE predicate\n' "$target_path" >&2
  exit 1
}

run_real() {
  blockers=()
  if [ -z "${AI_RELAY_API_KEY:-}" ]; then
    blockers+=("AI_RELAY_API_KEY is not set")
  fi
  if [ "${GRIMOIRE_VERIFY_READY:-}" != "1" ]; then
    blockers+=("GRIMOIRE_VERIFY_READY=1 readiness gate is not set")
  fi
  if ! command -v opencode >/dev/null 2>&1; then
    blockers+=("opencode CLI is not available")
  fi

  prerequisite_blockers=""
  if ! prerequisite_blockers="$(check_machine_prerequisites 2>&1)"; then
    blockers+=("${prerequisite_blockers}")
  fi

  if [ "${#blockers[@]}" -gt 0 ]; then
    write_blocked "${blockers[@]}"
    printf 'grimoire-verify: real mode blocked; REJECT artifact written to %s\n' "$output_path" >&2
    exit 1
  fi

  rm -f -- "$output_path"
  tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/grimoire-verify.XXXXXX")"
  prompt_file="${tmp_dir}/prompt.txt"
  opencode_output="${tmp_dir}/opencode.jsonl"
  real_verification_prompt > "$prompt_file"

  OPENCODE_PERMISSION="$(verify_permission_json)"
  export OPENCODE_PERMISSION
  export OPENCODE_MODEL="${OPENCODE_MODEL:-${DEFAULT_MODEL}}"
  export OPENCODE_VARIANT="${OPENCODE_VARIANT:-${DEFAULT_VARIANT}}"
  prompt_text="$(<"$prompt_file")"

  if ! opencode run \
    --format json \
    --dir "$repo_root" \
    --agent atlas \
    --model "$OPENCODE_MODEL" \
    --variant "$OPENCODE_VARIANT" \
    "$prompt_text" > "$opencode_output" 2>&1; then
    write_blocked "opencode run exited nonzero in real F1-F4 verification mode"
    printf 'grimoire-verify: real mode failed closed; REJECT artifact written to %s\n' "$output_path" >&2
    exit 1
  fi

  if write_contract normalize-real "$output_path" "$repo_root" "$spec_sufficiency_path" "$spec_gap_status_path" "$fix_status_path"; then
    if validate_approval "$output_path"; then
      printf 'grimoire-verify: real mode wrote all-APPROVE verdict to %s\n' "$output_path"
      exit 0
    fi
  fi

  printf 'grimoire-verify: real mode wrote non-approval or invalid verdict to %s\n' "$output_path" >&2
  exit 1
}

case "$mode" in
  mock-approve)
    run_mock_approve
    ;;
  mock-reject)
    run_mock_reject
    ;;
  mock-invalid)
    run_mock_invalid
    ;;
  validate)
    run_validate
    ;;
  real)
    run_real
    ;;
esac
