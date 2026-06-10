#!/usr/bin/env bash
set -euo pipefail

readonly DEFAULT_OUTPUT=".omo/ci/review-findings.json"
readonly DEFAULT_MODEL="ai-relay/gpt-5.5"
readonly DEFAULT_VARIANT="xhigh"

mode="${GRIMOIRE_REVIEW_MODE:-real}"
output_path="${GRIMOIRE_REVIEW_OUTPUT:-${DEFAULT_OUTPUT}}"
fixture_path="${GRIMOIRE_REVIEW_FIXTURE:-}"
repo_root="${GITHUB_WORKSPACE:-$(pwd)}"

usage() {
  cat <<'USAGE'
Usage: grimoire-review.sh [options]

Read-only grimoire review stage. Writes a stable JSON artifact for Task 6.

Options:
  --mode MODE       real, mock-clean, or mock-defect. Default: real.
  --fixture PATH    Fixture file or unified diff used by mock-defect.
  --input PATH      Alias for --fixture.
  --output PATH     JSON output path. Default: .omo/ci/review-findings.json.
  --repo-root PATH  Repository root for real-mode opencode execution.
  --help            Show this help text.

JSON contract:
  Root fields: status, approval_signal, read_only, mutation_allowed, findings.
  Finding fields: file, line, severity, lens, title, what, why,
  suggested_fix, evidence.

Modes:
  mock-clean   Deterministic local fixture; emits zero findings and
               GRIMOIRE_REVIEW_APPROVED.
  mock-defect  Deterministic local fixture; scans --fixture for
               GRIMOIRE_REVIEW_DEFECT and emits exact file/line findings.
  real         Fail-closed unless AI_RELAY_API_KEY, opencode, and
               GRIMOIRE_TEAM_MODE_ENABLED=1 are present. The model prompt is
               read-only and forbids source edits, commits, pushes, comments,
               labels, or GitHub mutation.
USAGE
}

fail_usage() {
  printf 'grimoire-review: %s\n\n' "$1" >&2
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
    --fixture|--input)
      [ "$#" -ge 2 ] || fail_usage "$1 requires a value"
      fixture_path="$2"
      shift 2
      ;;
    --output)
      [ "$#" -ge 2 ] || fail_usage "--output requires a value"
      output_path="$2"
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
  real|mock-clean|mock-defect)
    ;;
  *)
    fail_usage "unsupported mode: ${mode}"
    ;;
esac

if ! command -v python3 >/dev/null 2>&1; then
  printf 'grimoire-review: python3 is required\n' >&2
  exit 127
fi

write_contract() {
  python3 - "$@" <<'PY'
import json
import pathlib
import re
import sys
from datetime import datetime, timezone

COMMAND = sys.argv[1]
OUTPUT = pathlib.Path(sys.argv[2])
STAGE = "grimoire-review"
MARKER = "GRIMOIRE_REVIEW_DEFECT"
LENSES = ["security", "correctness", "maintainability", "repo-policy"]
REQUIRED_FINDING_FIELDS = [
    "file",
    "line",
    "severity",
    "lens",
    "title",
    "what",
    "why",
    "suggested_fix",
    "evidence",
]


def write_json(payload):
    payload.setdefault("schema_version", 1)
    payload.setdefault("stage", STAGE)
    payload.setdefault("generated_at", datetime.now(timezone.utc).isoformat().replace("+00:00", "Z"))
    payload.setdefault("lenses", LENSES)
    OUTPUT.parent.mkdir(parents=True, exist_ok=True)
    OUTPUT.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def base_payload(mode, status, approval_signal, findings):
    return {
        "status": status,
        "approval_signal": approval_signal,
        "read_only": True,
        "mutation_allowed": False,
        "mode": mode,
        "review_contract": "Task 5 stable review findings contract for Task 6",
        "findings": findings,
    }


def blocked(mode, reasons):
    payload = base_payload(mode, "blocked", "GRIMOIRE_REVIEW_BLOCKED", [])
    payload["blocked_reason"] = "; ".join(reasons)
    payload["blockers"] = reasons
    payload["real_mode_attempted"] = mode == "real"
    write_json(payload)


def clean():
    payload = base_payload("mock-clean", "approved", "GRIMOIRE_REVIEW_APPROVED", [])
    payload["real_mode_attempted"] = False
    payload["review_contract"] = "deterministic local clean fixture; no model or GitHub calls were made"
    write_json(payload)


def diff_findings(path):
    text = pathlib.Path(path).read_text(encoding="utf-8", errors="replace")
    findings = []
    current_file = None
    new_line = None
    saw_diff_shape = False
    for index, raw_line in enumerate(text.splitlines(), start=1):
        if raw_line.startswith("+++ "):
            saw_diff_shape = True
            target = raw_line[4:].strip()
            if target == "/dev/null":
                current_file = None
            elif target.startswith("b/"):
                current_file = target[2:]
            else:
                current_file = target
            continue
        if raw_line.startswith("@@ "):
            saw_diff_shape = True
            match = re.search(r"\+(\d+)(?:,\d+)?", raw_line)
            new_line = int(match.group(1)) if match else None
            continue
        if new_line is None:
            continue
        if raw_line.startswith("+") and not raw_line.startswith("+++"):
            if MARKER in raw_line:
                findings.append(make_finding(current_file or str(path), new_line, raw_line[1:].strip()))
            new_line += 1
        elif raw_line.startswith("-") and not raw_line.startswith("---"):
            continue
        elif raw_line.startswith(" ") or raw_line == "":
            new_line += 1
    if findings or saw_diff_shape:
        return findings

    for line_number, line in enumerate(text.splitlines(), start=1):
        if MARKER in line:
            findings.append(make_finding(str(path), line_number, line.strip()))
    return findings


def make_finding(file_name, line_number, line_text):
    evidence = f"{MARKER} marker observed at {file_name}:{line_number}: {line_text}"
    return {
        "file": file_name,
        "line": line_number,
        "severity": "medium",
        "lens": "correctness",
        "title": "Deterministic review defect marker present",
        "what": f"The local review fixture contains {MARKER}.",
        "why": "Task 5 uses this marker as a deterministic defect fixture to prove exact file/line finding emission before live team-mode is enabled.",
        "suggested_fix": f"Remove the {MARKER} marker or replace the intentionally defective fixture line with the corrected code before requesting approval.",
        "evidence": evidence,
    }


def mock_defect(path):
    if not path:
        blocked("mock-defect", ["--fixture or GRIMOIRE_REVIEW_FIXTURE is required for mock-defect"])
        return 1
    fixture = pathlib.Path(path)
    if not fixture.exists():
        blocked("mock-defect", [f"fixture does not exist: {path}"])
        return 1
    findings = diff_findings(fixture)
    if not findings:
        blocked("mock-defect", [f"fixture contains no {MARKER} marker: {path}"])
        return 1
    payload = base_payload("mock-defect", "findings", "GRIMOIRE_REVIEW_FINDINGS_PRESENT", findings)
    payload["real_mode_attempted"] = False
    payload["review_contract"] = "deterministic local defect fixture; no model or GitHub calls were made"
    write_json(payload)
    return 0


def validate_review_object(candidate):
    if not isinstance(candidate, dict):
        return False, "candidate is not a JSON object"
    for field in ["status", "approval_signal", "read_only", "mutation_allowed", "findings"]:
        if field not in candidate:
            return False, f"missing root field: {field}"
    if candidate.get("read_only") is not True:
        return False, "read_only must be true"
    if candidate.get("mutation_allowed") is not False:
        return False, "mutation_allowed must be false"
    if not isinstance(candidate.get("findings"), list):
        return False, "findings must be an array"
    for index, finding in enumerate(candidate["findings"]):
        if not isinstance(finding, dict):
            return False, f"finding {index} is not an object"
        for field in REQUIRED_FINDING_FIELDS:
            if field not in finding:
                return False, f"finding {index} missing field: {field}"
        if finding.get("lens") not in LENSES:
            return False, f"finding {index} has unsupported lens: {finding.get('lens')}"
    return True, "ok"


def walk(value):
    if isinstance(value, dict):
        yield value
        for child in value.values():
            yield from walk(child)
    elif isinstance(value, list):
        for child in value:
            yield from walk(child)


def possible_json_strings(raw):
    yield raw
    for line in raw.splitlines():
        stripped = line.strip()
        if stripped:
            yield stripped
        try:
            event = json.loads(stripped)
        except json.JSONDecodeError:
            continue
        for obj in walk(event):
            for key in ("text", "content", "message", "output"):
                value = obj.get(key)
                if isinstance(value, str):
                    yield value


def extract_real(raw_path):
    raw = pathlib.Path(raw_path).read_text(encoding="utf-8", errors="replace")
    last_error = "no JSON candidate found"
    for text in possible_json_strings(raw):
        text = text.strip()
        if not text:
            continue
        try:
            candidate = json.loads(text)
        except json.JSONDecodeError:
            start = text.find("{")
            end = text.rfind("}")
            if start < 0 or end <= start:
                continue
            try:
                candidate = json.loads(text[start : end + 1])
            except json.JSONDecodeError as exc:
                last_error = str(exc)
                continue
        ok, reason = validate_review_object(candidate)
        if not ok:
            last_error = reason
            continue
        candidate["schema_version"] = 1
        candidate["stage"] = STAGE
        candidate.setdefault("mode", "real")
        candidate.setdefault("lenses", LENSES)
        candidate.setdefault("review_contract", "real read-only opencode review output")
        candidate["real_mode_attempted"] = True
        write_json(candidate)
        return 0
    blocked("real", [f"opencode did not return a valid review JSON object: {last_error}"])
    return 1


if COMMAND == "blocked":
    blocked(sys.argv[3], sys.argv[4:])
    sys.exit(0)
if COMMAND == "mock-clean":
    clean()
    sys.exit(0)
if COMMAND == "mock-defect":
    sys.exit(mock_defect(sys.argv[3] if len(sys.argv) > 3 else ""))
if COMMAND == "extract-real":
    sys.exit(extract_real(sys.argv[3]))
raise SystemExit(f"unknown command: {COMMAND}")
PY
}

write_blocked() {
  write_contract blocked "$output_path" "$mode" "$@"
}

run_mock_clean() {
  write_contract mock-clean "$output_path"
  printf 'grimoire-review: wrote clean review artifact to %s\n' "$output_path"
}

run_mock_defect() {
  if write_contract mock-defect "$output_path" "$fixture_path"; then
    printf 'grimoire-review: wrote defect review artifact to %s\n' "$output_path"
  else
    printf 'grimoire-review: mock-defect failed closed; artifact written to %s\n' "$output_path" >&2
    exit 1
  fi
}

readonly_permission_json() {
  cat <<'JSON'
{"read":{"*":"allow","*.env":"deny","*.env.*":"deny","*.env.example":"allow"},"edit":"deny","bash":"deny","task":"deny","webfetch":"deny","external_directory":"deny","question":"deny","plan_enter":"deny","plan_exit":"deny"}
JSON
}

real_review_prompt() {
  cat <<'PROMPT'
You are running the grimoire Task 5 review stage for this repository.

Operate in read-only review mode:
- Do not edit files.
- Do not run write, commit, push, pull request comment, label, workflow dispatch, or GitHub mutation commands.
- Do not reveal, transform, list, or summarize secret values.
- Return exactly one JSON object and no prose.

Use four lenses:
1. security: secret hygiene, auth boundary, supply-chain, unsafe credential/log handling.
2. correctness: concrete runtime or contract breakages with reachable paths.
3. maintainability: focused maintainability defects that block safe operation.
4. repo-policy: AGENTS.md and grimoire scope rules, including wire-format caution, auth identity separation, no branch-pin production dependency, no venue-facing guesswork, and read-only review boundaries.

Severity discipline:
- No severity without a concrete attack or failure path.
- Do not report generic hardening advice as a finding.
- Every finding must include exact evidence and a specific suggested fix.

Return this schema:
{
  "status": "approved" | "findings" | "blocked",
  "approval_signal": "GRIMOIRE_REVIEW_APPROVED" | "GRIMOIRE_REVIEW_FINDINGS_PRESENT" | "GRIMOIRE_REVIEW_BLOCKED",
  "read_only": true,
  "mutation_allowed": false,
  "findings": [
    {
      "file": "path/from/repo/root",
      "line": 1,
      "severity": "low" | "medium" | "high" | "critical",
      "lens": "security" | "correctness" | "maintainability" | "repo-policy",
      "title": "short concrete title",
      "what": "what is wrong",
      "why": "why it matters with exploitability or failure path",
      "suggested_fix": "smallest specific remediation",
      "evidence": "exact evidence, including file:line or command output summary"
    }
  ]
}

If there are no concrete findings, return status "approved", approval_signal "GRIMOIRE_REVIEW_APPROVED", and an empty findings array.
PROMPT
}

run_real() {
  blockers=()
  if [ -z "${AI_RELAY_API_KEY:-}" ]; then
    blockers+=("AI_RELAY_API_KEY is not set")
  fi
  if [ "${GRIMOIRE_TEAM_MODE_ENABLED:-}" != "1" ]; then
    blockers+=("GRIMOIRE_TEAM_MODE_ENABLED=1 readiness gate is not set")
  fi
  if ! command -v opencode >/dev/null 2>&1; then
    blockers+=("opencode CLI is not available")
  fi
  if [ "${#blockers[@]}" -gt 0 ]; then
    write_blocked "${blockers[@]}"
    printf 'grimoire-review: real mode blocked; artifact written to %s\n' "$output_path" >&2
    exit 1
  fi

  tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/grimoire-review.XXXXXX")"
  prompt_file="${tmp_dir}/prompt.txt"
  opencode_output="${tmp_dir}/opencode.jsonl"
  real_review_prompt > "$prompt_file"

  OPENCODE_PERMISSION="$(readonly_permission_json)"
  export OPENCODE_PERMISSION
  export OPENCODE_MODEL="${OPENCODE_MODEL:-${DEFAULT_MODEL}}"
  export OPENCODE_VARIANT="${OPENCODE_VARIANT:-${DEFAULT_VARIANT}}"

  if ! opencode run \
    --format json \
    --dir "$repo_root" \
    --model "$OPENCODE_MODEL" \
    --variant "$OPENCODE_VARIANT" \
    "$(cat "$prompt_file")" > "$opencode_output" 2>&1; then
    write_blocked "opencode run exited nonzero in read-only real mode"
    printf 'grimoire-review: real mode failed closed; artifact written to %s\n' "$output_path" >&2
    exit 1
  fi

  if write_contract extract-real "$output_path" "$opencode_output"; then
    printf 'grimoire-review: wrote real review artifact to %s\n' "$output_path"
  else
    printf 'grimoire-review: real mode output failed contract validation; artifact written to %s\n' "$output_path" >&2
    exit 1
  fi
}

case "$mode" in
  mock-clean)
    run_mock_clean
    ;;
  mock-defect)
    run_mock_defect
    ;;
  real)
    run_real
    ;;
esac
