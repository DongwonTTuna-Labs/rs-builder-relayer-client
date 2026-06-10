#!/usr/bin/env bash
set -euo pipefail

readonly DEFAULT_SPEC_SUFFICIENCY=".omo/ci/spec-sufficiency.json"
readonly DEFAULT_PLAN=".omo/ci/design-plan.md"
readonly DEFAULT_SPEC_GAP_STATUS=".omo/ci/spec-gap-status.json"
readonly DEFAULT_OUTPUT=".omo/ci/fix-status.json"
readonly DEFAULT_HANDOFF_PROMPT=".omo/ci/fix-handoff-prompt.md"

mode="${GRIMOIRE_FIX_MODE:-real}"
spec_sufficiency_path="${GRIMOIRE_SPEC_SUFFICIENCY:-${DEFAULT_SPEC_SUFFICIENCY}}"
plan_path="${GRIMOIRE_DESIGN_PLAN:-${DEFAULT_PLAN}}"
spec_gap_status_path="${GRIMOIRE_SPEC_GAP_STATUS:-${DEFAULT_SPEC_GAP_STATUS}}"
output_path="${GRIMOIRE_FIX_STATUS:-${DEFAULT_OUTPUT}}"
handoff_prompt_path="${GRIMOIRE_FIX_HANDOFF_PROMPT:-${DEFAULT_HANDOFF_PROMPT}}"
repo_root="${GITHUB_WORKSPACE:-$(pwd)}"
pr_touched_list_paths=()
pr_touched_files=()
direct_extra_list_paths=()
direct_extra_files=()
changed_list_paths=()
changed_files=()

usage() {
  cat <<'USAGE'
Usage: grimoire-fix.sh [options]

Grimoire Task 8 fix-stage control script. It consumes Task 6 spec sufficiency,
Task 6 design plan, and Task 7 spec-gap status artifacts, prepares an
Atlas /start-work handoff prompt, and writes machine-readable fix status for
later driver/verification stages. It does not commit, push, post PR comments,
change labels, merge, dispatch workflows, or use GitHub API mutation.

Options:
  --mode MODE                real, mock-fix, mock-noop, mock-scope-violation,
                             or mock-blocked. Default: real.
  --spec-sufficiency PATH    Task 6 JSON. Default: .omo/ci/spec-sufficiency.json.
  --plan PATH                Task 6 design plan. Default: .omo/ci/design-plan.md.
  --spec-gap-status PATH     Task 7 JSON. Default: .omo/ci/spec-gap-status.json.
  --output PATH              Fix status JSON. Default: .omo/ci/fix-status.json.
  --handoff-prompt PATH      Atlas/executor prompt artifact.
                             Default: .omo/ci/fix-handoff-prompt.md.
  --pr-touched PATH          Newline file of PR-touched paths. Repeatable.
  --pr-file PATH             Single PR-touched path. Repeatable.
  --direct-extras PATH       Newline file of direct test/docs/spec extras.
                             Repeatable.
  --direct-extra PATH        Single direct test/docs/spec extra. Repeatable.
  --changed-files PATH       Newline file of post-fix changed paths. Repeatable.
                             In real mode this declaration is required even when
                             the file is intentionally empty for no-op.
  --changed-file PATH        Single post-fix changed path. Repeatable.
  --repo-root PATH           Repository root for path normalization.
  --help                     Show this help text.

Inputs and blocking rules:
  The script fails closed before any model/code mutation signal if Task 6 has
  spec_sufficient=false, if Task 7 has should_halt=true, if the design plan is
  a halt-only artifact, or if real mode lacks AI_RELAY_API_KEY, opencode,
  GRIMOIRE_FIX_READY=1, a declared non-empty PR-touched file set, and a declared
  post-fix changed-file source.

Scope guard:
  Changed files must be within PR-touched files, explicitly declared direct
  test/docs/spec extras, or OpenSpec spec paths cited by Task 6 bindings. Any
  other changed file is a fail-closed scope violation. Direct extras are limited
  to test, docs, or spec paths.

No-op guard:
  An empty declared post-fix changed-file list writes status=clear-noop,
  noop=true, scope_ok=true, should_commit=false, and should_push=false.

Real mode readiness:
  Real mode prepares the Atlas /start-work handoff prompt and validates the
  supplied post-fix changed-file set. It requires AI_RELAY_API_KEY, opencode,
  and GRIMOIRE_FIX_READY=1 before it can authorize later model/code mutation,
  but this stage itself still performs no commit or push.

Mock modes:
  mock-fix              Deterministic sufficient fixture. PR-touched source,
                        direct test/docs/spec extras, and cited OpenSpec paths
                        all pass scope guard and produce status=fixed.
  mock-noop             Deterministic sufficient fixture with empty changed-file
                        list, proving no commit/push signal.
  mock-scope-violation  Deterministic sufficient fixture with an out-of-scope
                        changed file; exits nonzero with scope_ok=false.
  mock-blocked          Deterministic insufficient/spec-gap fixture; exits
                        nonzero before handoff authorization.
USAGE
}

fail_usage() {
  printf 'grimoire-fix: %s\n\n' "$1" >&2
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
    --spec-sufficiency)
      [ "$#" -ge 2 ] || fail_usage "--spec-sufficiency requires a value"
      spec_sufficiency_path="$2"
      shift 2
      ;;
    --plan)
      [ "$#" -ge 2 ] || fail_usage "--plan requires a value"
      plan_path="$2"
      shift 2
      ;;
    --spec-gap-status)
      [ "$#" -ge 2 ] || fail_usage "--spec-gap-status requires a value"
      spec_gap_status_path="$2"
      shift 2
      ;;
    --output)
      [ "$#" -ge 2 ] || fail_usage "--output requires a value"
      output_path="$2"
      shift 2
      ;;
    --handoff-prompt|--prompt)
      [ "$#" -ge 2 ] || fail_usage "$1 requires a value"
      handoff_prompt_path="$2"
      shift 2
      ;;
    --pr-touched)
      [ "$#" -ge 2 ] || fail_usage "--pr-touched requires a value"
      pr_touched_list_paths+=("$2")
      shift 2
      ;;
    --pr-file)
      [ "$#" -ge 2 ] || fail_usage "--pr-file requires a value"
      pr_touched_files+=("$2")
      shift 2
      ;;
    --direct-extras)
      [ "$#" -ge 2 ] || fail_usage "--direct-extras requires a value"
      direct_extra_list_paths+=("$2")
      shift 2
      ;;
    --direct-extra)
      [ "$#" -ge 2 ] || fail_usage "--direct-extra requires a value"
      direct_extra_files+=("$2")
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
  real|mock-fix|mock-noop|mock-scope-violation|mock-blocked)
    ;;
  *)
    fail_usage "unsupported mode: ${mode}"
    ;;
esac

if ! command -v python3 >/dev/null 2>&1; then
  printf 'grimoire-fix: python3 is required\n' >&2
  exit 127
fi

python_args=()
if [ "${#pr_touched_list_paths[@]}" -gt 0 ]; then
  for value in "${pr_touched_list_paths[@]}"; do
    python_args+=(--pr-list "$value")
  done
fi
if [ "${#pr_touched_files[@]}" -gt 0 ]; then
  for value in "${pr_touched_files[@]}"; do
    python_args+=(--pr-file "$value")
  done
fi
if [ "${#direct_extra_list_paths[@]}" -gt 0 ]; then
  for value in "${direct_extra_list_paths[@]}"; do
    python_args+=(--direct-list "$value")
  done
fi
if [ "${#direct_extra_files[@]}" -gt 0 ]; then
  for value in "${direct_extra_files[@]}"; do
    python_args+=(--direct-file "$value")
  done
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

write_contract() {
  python3 - "$@" <<'PY'
import json
import pathlib
import re
import shutil
import sys
from datetime import datetime, timezone

COMMAND = sys.argv[1]
OUTPUT = pathlib.Path(sys.argv[2])
HANDOFF = pathlib.Path(sys.argv[3])
SPEC = pathlib.Path(sys.argv[4])
PLAN = pathlib.Path(sys.argv[5])
GAP = pathlib.Path(sys.argv[6])
REPO_ROOT = pathlib.Path(sys.argv[7])
EXTRA = sys.argv[8:]
STAGE = "grimoire-fix"
REQUIRED_SPEC_FIELDS = [
    "spec_sufficient",
    "bindings",
    "missing",
    "safety_default_gaps",
    "suggested_spec_patch",
    "plan_path",
    "halt_reason",
]
REQUIRED_GAP_FIELDS = [
    "should_halt",
    "should_comment",
    "github_mutation_performed",
    "no_code_or_push_action",
]


class ContractError(Exception):
    pass


def utc_now():
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def repo_root():
    return REPO_ROOT.resolve()


def path_for_output(path):
    try:
        return pathlib.Path(path).resolve().relative_to(repo_root()).as_posix()
    except (OSError, ValueError):
        return pathlib.Path(path).as_posix()


def normalize_path(raw):
    value = str(raw).strip()
    if not value:
        return ""
    value = value.replace("\\", "/")
    if value.startswith("./"):
        value = value[2:]
    if value.startswith("a/") or value.startswith("b/"):
        value = value[2:]
    path = pathlib.Path(value)
    if path.is_absolute():
        try:
            return path.resolve().relative_to(repo_root()).as_posix()
        except (OSError, ValueError):
            return path.as_posix()
    try:
        return (repo_root() / path).resolve().relative_to(repo_root()).as_posix()
    except (OSError, ValueError):
        return pathlib.PurePosixPath(value).as_posix()


def dedupe(values):
    result = []
    seen = set()
    for value in values:
        normalized = normalize_path(value)
        if not normalized or normalized in seen:
            continue
        seen.add(normalized)
        result.append(normalized)
    return result


def load_lines(path):
    source = pathlib.Path(path)
    if not source.exists():
        raise ContractError(f"path list does not exist: {path}")
    lines = []
    for line in source.read_text(encoding="utf-8", errors="replace").splitlines():
        stripped = line.strip()
        if stripped and not stripped.startswith("#"):
            lines.append(stripped)
    return lines


def parse_cli_sources(args):
    values = {
        "pr": [],
        "direct": [],
        "changed": [],
        "changed_declared": False,
    }
    index = 0
    while index < len(args):
        flag = args[index]
        if index + 1 >= len(args):
            raise ContractError(f"missing value after {flag}")
        value = args[index + 1]
        if flag == "--pr-list":
            values["pr"].extend(load_lines(value))
        elif flag == "--pr-file":
            values["pr"].append(value)
        elif flag == "--direct-list":
            values["direct"].extend(load_lines(value))
        elif flag == "--direct-file":
            values["direct"].append(value)
        elif flag == "--changed-list":
            values["changed"].extend(load_lines(value))
            values["changed_declared"] = True
        elif flag == "--changed-file":
            values["changed"].append(value)
            values["changed_declared"] = True
        else:
            raise ContractError(f"unknown internal argument: {flag}")
        index += 2
    values["pr"] = dedupe(values["pr"])
    values["direct"] = dedupe(values["direct"])
    values["changed"] = dedupe(values["changed"])
    return values


def load_json(path, label):
    if not path.exists():
        raise ContractError(f"{label} missing: {path}")
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        raise ContractError(f"{label} is not valid JSON: {exc}") from exc


def validate_spec(payload):
    if not isinstance(payload, dict):
        raise ContractError("Task 6 spec sufficiency must be a JSON object")
    for field in REQUIRED_SPEC_FIELDS:
        if field not in payload:
            raise ContractError(f"Task 6 spec sufficiency missing root field: {field}")
    if not isinstance(payload.get("spec_sufficient"), bool):
        raise ContractError("Task 6 spec_sufficient must be boolean")
    for field in ("bindings", "missing", "safety_default_gaps"):
        if not isinstance(payload.get(field), list):
            raise ContractError(f"Task 6 {field} must be an array")
    for field in ("suggested_spec_patch", "plan_path", "halt_reason"):
        if not isinstance(payload.get(field), str):
            raise ContractError(f"Task 6 {field} must be a string")
    return payload


def validate_gap(payload):
    if not isinstance(payload, dict):
        raise ContractError("Task 7 spec-gap status must be a JSON object")
    for field in REQUIRED_GAP_FIELDS:
        if field not in payload:
            raise ContractError(f"Task 7 spec-gap status missing root field: {field}")
    for field in REQUIRED_GAP_FIELDS:
        if not isinstance(payload.get(field), bool):
            raise ContractError(f"Task 7 {field} must be boolean")
    return payload


def citation_to_path(value):
    text = str(value).strip()
    if not text:
        return ""
    text = text.split("#", 1)[0]
    match = re.match(r"^(.+?):\d+(?::\d+)?$", text)
    if match:
        text = match.group(1)
    normalized = normalize_path(text)
    if normalized.startswith("openspec/"):
        return normalized
    return ""


def cited_spec_files(spec):
    cited = []
    for binding in spec.get("bindings", []):
        if not isinstance(binding, dict):
            continue
        for key in ("citation", "spec_path"):
            path = citation_to_path(binding.get(key, ""))
            if path:
                cited.append(path)
        citations = binding.get("citations", [])
        if isinstance(citations, list):
            for citation in citations:
                path = citation_to_path(citation)
                if path:
                    cited.append(path)
    return dedupe(cited)


def is_direct_extra_allowed(path):
    lower = path.lower()
    if lower.startswith(("tests/", "test/", "docs/", "openspec/")):
        return True
    if "/tests/" in lower or "/test/" in lower:
        return True
    if lower.endswith(("_test.rs", "_test.py", ".test.ts", ".test.tsx", ".spec.ts", ".spec.tsx")):
        return True
    if lower.endswith((".md", ".rst", ".txt")):
        return True
    return False


def plan_text_required(spec):
    declared = str(spec.get("plan_path", "")).strip()
    if declared and declared != path_for_output(PLAN):
        return f"Task 6 plan_path is {declared}, but this run was given {path_for_output(PLAN)}"
    if not PLAN.exists():
        return f"Task 6 design plan is missing: {PLAN}"
    text = PLAN.read_text(encoding="utf-8", errors="replace")
    lowered = text.lower()
    if "grimoire design halt" in lowered or "non-executable boundary" in lowered:
        return "Task 6 design plan is a halt-only artifact"
    return ""


def mock_spec_sufficient():
    return {
        "schema_version": 1,
        "stage": "grimoire-design",
        "mode": "mock-sufficient",
        "status": "planned",
        "spec_sufficient": True,
        "bindings": [
            {
                "finding_index": 0,
                "binding_key": "src/deposit_wallet/http/read.rs:42",
                "finding_title": "Scope guard fixture",
                "requirement": "Fix-stage scope guard must enforce cited OpenSpec boundaries",
                "citation": "openspec/changes/grimoire-fix/specs/fix-stage/spec.md:12",
                "citations": ["openspec/changes/grimoire-fix/specs/fix-stage/spec.md:12"],
                "evidence": "### Requirement: Fix-stage scope guard must enforce cited OpenSpec boundaries",
            }
        ],
        "missing": [],
        "safety_default_gaps": [],
        "suggested_spec_patch": "No patch required.",
        "plan_path": path_for_output(PLAN),
        "halt_reason": "",
        "review_findings_count": 1,
    }


def mock_spec_insufficient():
    return {
        "schema_version": 1,
        "stage": "grimoire-design",
        "mode": "mock-insufficient",
        "status": "halted",
        "spec_sufficient": False,
        "bindings": [],
        "missing": [
            {
                "finding_index": 0,
                "location": "src/deposit_wallet/http/read.rs:42",
                "finding_title": "Missing fixture evidence",
                "reason": "Deterministic mock-blocked fixture lacks OpenSpec evidence.",
                "required_evidence": "Add cited OpenSpec coverage before fix.",
            }
        ],
        "safety_default_gaps": [],
        "suggested_spec_patch": "Add fixture OpenSpec coverage.",
        "plan_path": path_for_output(PLAN),
        "halt_reason": "OpenSpec evidence is absent for review findings",
        "review_findings_count": 1,
    }


def mock_gap_clear():
    return {
        "schema_version": 1,
        "stage": "grimoire-spec-gap-comment",
        "mode": "mock-sufficient",
        "status": "clear",
        "spec_sufficient": True,
        "should_comment": False,
        "should_halt": False,
        "github_mutation_performed": False,
        "no_code_or_push_action": True,
    }


def mock_gap_halt():
    return {
        "schema_version": 1,
        "stage": "grimoire-spec-gap-comment",
        "mode": "mock-insufficient",
        "status": "halted",
        "spec_sufficient": False,
        "should_comment": True,
        "should_halt": True,
        "github_mutation_performed": False,
        "no_code_or_push_action": True,
        "halt_reason": "mock spec gap halt",
    }


def mock_plan_text():
    return "\n".join(
        [
            "# Grimoire Design Plan",
            "",
            "## Plan",
            "",
            "- [ ] Apply the scoped fixture fix using the cited OpenSpec requirement.",
            "",
            "## Execution Boundary",
            "",
            "Later fix stages must not expand scope beyond cited bindings.",
            "",
        ]
    )


def ensure_mock_plan():
    PLAN.parent.mkdir(parents=True, exist_ok=True)
    PLAN.write_text(mock_plan_text(), encoding="utf-8")


def write_json(payload):
    payload.setdefault("schema_version", 1)
    payload.setdefault("stage", STAGE)
    payload.setdefault("generated_at", utc_now())
    OUTPUT.parent.mkdir(parents=True, exist_ok=True)
    OUTPUT.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def write_handoff(spec, gap, scope):
    HANDOFF.parent.mkdir(parents=True, exist_ok=True)
    allowed_lines = [f"- `{path}` ({reason})" for path, reason in scope["allowed_reasons"].items()]
    if not allowed_lines:
        allowed_lines = ["- None declared; do not edit files."]
    binding_lines = []
    for binding in spec.get("bindings", []):
        if not isinstance(binding, dict):
            continue
        title = str(binding.get("finding_title") or binding.get("requirement") or "binding").strip()
        citation = str(binding.get("citation") or binding.get("spec_path") or "not provided").strip()
        binding_lines.append(f"- {title}: `{citation}`")
    if not binding_lines:
        binding_lines = ["- No review findings required implementation; no-op is valid if there are no post-fix changes."]
    lines = [
        "# Grimoire Task 8 Fix Handoff",
        "",
        "Invoke Atlas with `/start-work` using the approved Task 6 plan.",
        "",
        "## Execution Prompt",
        "",
        "`/start-work`",
        "",
        f"Plan: `{path_for_output(PLAN)}`",
        f"Spec sufficiency: `{path_for_output(SPEC)}`",
        f"Spec-gap status: `{path_for_output(GAP)}`",
        f"Fix status output: `{path_for_output(OUTPUT)}`",
        "",
        "## Required Executor",
        "",
        "Use Atlas to coordinate the fix work. Use Sisyphus or Hephaestus for implementation according to the active OMO model/runtime mapping.",
        "",
        "## Scope Guard",
        "",
        "Allowed files are exactly the PR-touched files, direct test/docs/spec extras, and OpenSpec files cited by Task 6 bindings listed below. Any other changed file must be rejected as a scope violation before commit or push.",
        "",
        *allowed_lines,
        "",
        "## OpenSpec Bindings",
        "",
        *binding_lines,
        "",
        "## No-op Guard",
        "",
        "After the executor returns, collect the post-fix changed-file list. If it is empty, keep `status=clear-noop`, `noop=true`, `scope_ok=true`, `should_commit=false`, and `should_push=false`. Never create an empty commit or push.",
        "",
        "## Mutation Limits",
        "",
        "Do not use GitHub API mutation, PR comments, labels, workflow dispatch, merge, push, or token-bearing output from this stage. The later driver owns commit/push only after Task 8 scope and Task 9 F1-F4 gates pass.",
        "",
        "## Current Machine Signals",
        "",
        f"- Task 6 spec_sufficient: `{spec.get('spec_sufficient')}`",
        f"- Task 7 should_halt: `{gap.get('should_halt')}`",
        f"- Changed files observed by this stage: `{len(scope['changed_files'])}`",
        "",
    ]
    HANDOFF.write_text("\n".join(lines), encoding="utf-8")


def clear_handoff():
    HANDOFF.parent.mkdir(parents=True, exist_ok=True)
    HANDOFF.write_text("", encoding="utf-8")


def scope_from_files(spec, pr_files, direct_files, changed):
    cited = cited_spec_files(spec)
    invalid_direct = [path for path in direct_files if not is_direct_extra_allowed(path)]
    allowed_reasons = {}
    for path in pr_files:
        allowed_reasons[path] = "pr-touched"
    for path in direct_files:
        allowed_reasons[path] = "direct-test-docs-spec-extra" if path not in invalid_direct else "invalid-direct-extra"
    for path in cited:
        allowed_reasons[path] = "openspec-citation"
    allowed = {path for path, reason in allowed_reasons.items() if reason != "invalid-direct-extra"}
    violations = [path for path in changed if path not in allowed]
    return {
        "allowed_reasons": dict(sorted(allowed_reasons.items())),
        "allowed_files": sorted(allowed),
        "pr_touched_files": pr_files,
        "direct_extra_files": direct_files,
        "cited_spec_files": cited,
        "invalid_direct_extras": invalid_direct,
        "changed_files": changed,
        "scope_violations": violations,
        "scope_ok": not invalid_direct and not violations,
    }


def status_payload(mode, status, spec, gap, scope, blocked_reason="", blockers=None, real_mode_attempted=False, model_called=False):
    changed = scope["changed_files"]
    noop = status == "clear-noop"
    fixed = status == "fixed"
    return {
        "schema_version": 1,
        "stage": STAGE,
        "generated_at": utc_now(),
        "mode": mode,
        "status": status,
        "fix_applied": fixed,
        "noop": noop,
        "scope_ok": scope["scope_ok"],
        "changed_files": changed,
        "allowed_files": scope["allowed_files"],
        "allowed_reasons": scope["allowed_reasons"],
        "pr_touched_files": scope["pr_touched_files"],
        "direct_extra_files": scope["direct_extra_files"],
        "cited_spec_files": scope["cited_spec_files"],
        "invalid_direct_extras": scope["invalid_direct_extras"],
        "scope_violations": scope["scope_violations"],
        "should_commit": fixed and scope["scope_ok"],
        "should_push": fixed and scope["scope_ok"],
        "commit_attempted": False,
        "push_attempted": False,
        "github_mutation_performed": False,
        "blocked_reason": blocked_reason,
        "blockers": blockers or [],
        "spec_sufficient": spec.get("spec_sufficient"),
        "spec_sufficiency_path": path_for_output(SPEC),
        "design_plan_path": path_for_output(PLAN),
        "spec_gap_status_path": path_for_output(GAP),
        "handoff_prompt_path": path_for_output(HANDOFF),
        "task7_should_halt": gap.get("should_halt"),
        "task7_should_comment": gap.get("should_comment"),
        "model_called": model_called,
        "real_mode_attempted": real_mode_attempted,
    }


def blocked_payload(mode, spec, gap, blockers, real_mode_attempted=False):
    empty_scope = scope_from_files(spec, [], [], [])
    payload = status_payload(
        mode,
        "blocked",
        spec,
        gap,
        empty_scope,
        blocked_reason="; ".join(blockers),
        blockers=blockers,
        real_mode_attempted=real_mode_attempted,
        model_called=False,
    )
    payload["scope_ok"] = False
    payload["should_commit"] = False
    payload["should_push"] = False
    payload["noop"] = False
    payload["fix_applied"] = False
    return payload


def evaluate(mode, spec, gap, pr_files, direct_files, changed, changed_declared=False, real_mode_attempted=False):
    blockers = []
    if spec.get("spec_sufficient") is not True:
        blockers.append("Task 6 spec_sufficient is false")
    if spec.get("halt_reason") not in ("", None):
        blockers.append("Task 6 halt_reason is not empty")
    if gap.get("should_halt") is True:
        blockers.append("Task 7 should_halt is true")
    if gap.get("github_mutation_performed") is not False:
        blockers.append("Task 7 github_mutation_performed must be false")
    if gap.get("no_code_or_push_action") is not True:
        blockers.append("Task 7 no_code_or_push_action must be true")
    plan_blocker = plan_text_required(spec) if spec.get("spec_sufficient") is True else ""
    if plan_blocker:
        blockers.append(plan_blocker)
    if real_mode_attempted:
        if not pr_files:
            blockers.append("real mode requires a non-empty declared PR-touched file set")
        if not changed_declared:
            blockers.append("real mode requires an explicit post-fix changed-file declaration")
    if blockers:
        clear_handoff()
        write_json(blocked_payload(mode, spec, gap, blockers, real_mode_attempted=real_mode_attempted))
        return 1

    scope = scope_from_files(spec, pr_files, direct_files, changed)
    write_handoff(spec, gap, scope)
    if not scope["scope_ok"]:
        blockers = []
        if scope["invalid_direct_extras"]:
            blockers.append("direct extras include non test/docs/spec paths")
        if scope["scope_violations"]:
            blockers.append("post-fix changed files include out-of-scope paths")
        payload = status_payload(
            mode,
            "scope-violation",
            spec,
            gap,
            scope,
            blocked_reason="; ".join(blockers),
            blockers=blockers,
            real_mode_attempted=real_mode_attempted,
        )
        payload["should_commit"] = False
        payload["should_push"] = False
        write_json(payload)
        return 1

    status = "clear-noop" if not changed else "fixed"
    write_json(status_payload(mode, status, spec, gap, scope, real_mode_attempted=real_mode_attempted))
    return 0


def real_prerequisite_blockers(cli_values):
    blockers = []
    if not bool(cli_values["pr"]):
        blockers.append("real mode requires declared PR-touched file metadata")
    if not cli_values["changed_declared"]:
        blockers.append("real mode requires declared post-fix changed-file metadata")
    if not shutil.which("opencode"):
        blockers.append("opencode CLI is not available")
    if not bool(__import__("os").environ.get("AI_RELAY_API_KEY")):
        blockers.append("AI_RELAY_API_KEY is not set")
    if __import__("os").environ.get("GRIMOIRE_FIX_READY") != "1":
        blockers.append("GRIMOIRE_FIX_READY=1 readiness gate is not set")
    return blockers


def run_mock_fix():
    ensure_mock_plan()
    spec = validate_spec(mock_spec_sufficient())
    gap = validate_gap(mock_gap_clear())
    pr = dedupe(["src/deposit_wallet/http/read.rs"])
    direct = dedupe(["tests/deposit_wallet/http/read_scope_guard.rs", "docs/grimoire-fix-scope.md"])
    changed = dedupe([
        "src/deposit_wallet/http/read.rs",
        "tests/deposit_wallet/http/read_scope_guard.rs",
        "docs/grimoire-fix-scope.md",
        "openspec/changes/grimoire-fix/specs/fix-stage/spec.md",
    ])
    return evaluate("mock-fix", spec, gap, pr, direct, changed)


def run_mock_noop():
    ensure_mock_plan()
    spec = validate_spec(mock_spec_sufficient())
    gap = validate_gap(mock_gap_clear())
    pr = dedupe(["src/deposit_wallet/http/read.rs"])
    direct = dedupe(["tests/deposit_wallet/http/read_scope_guard.rs"])
    return evaluate("mock-noop", spec, gap, pr, direct, [])


def run_mock_scope_violation():
    ensure_mock_plan()
    spec = validate_spec(mock_spec_sufficient())
    gap = validate_gap(mock_gap_clear())
    pr = dedupe(["src/deposit_wallet/http/read.rs"])
    direct = dedupe(["tests/deposit_wallet/http/read_scope_guard.rs"])
    changed = dedupe(["src/deposit_wallet/http/read.rs", ".github/workflows/grimoire.yml"])
    return evaluate("mock-scope-violation", spec, gap, pr, direct, changed)


def run_mock_blocked():
    ensure_mock_plan()
    spec = validate_spec(mock_spec_insufficient())
    gap = validate_gap(mock_gap_halt())
    return evaluate("mock-blocked", spec, gap, [], [], [])


def run_real():
    cli_values = parse_cli_sources(EXTRA)
    spec = validate_spec(load_json(SPEC, "Task 6 spec sufficiency artifact"))
    gap = validate_gap(load_json(GAP, "Task 7 spec-gap status artifact"))
    prereq_blockers = real_prerequisite_blockers(cli_values)
    if prereq_blockers:
        clear_handoff()
        write_json(blocked_payload("real", spec, gap, prereq_blockers, real_mode_attempted=True))
        return 1
    return evaluate(
        "real",
        spec,
        gap,
        cli_values["pr"],
        cli_values["direct"],
        cli_values["changed"],
        changed_declared=cli_values["changed_declared"],
        real_mode_attempted=True,
    )


def main():
    if COMMAND == "mock-fix":
        return run_mock_fix()
    if COMMAND == "mock-noop":
        return run_mock_noop()
    if COMMAND == "mock-scope-violation":
        return run_mock_scope_violation()
    if COMMAND == "mock-blocked":
        return run_mock_blocked()
    if COMMAND == "real":
        return run_real()
    raise ContractError(f"unsupported command: {COMMAND}")


try:
    sys.exit(main())
except ContractError as exc:
    fallback_spec = {"spec_sufficient": False, "halt_reason": str(exc), "bindings": []}
    fallback_gap = {"should_halt": True, "should_comment": False, "github_mutation_performed": False, "no_code_or_push_action": True}
    clear_handoff()
    write_json(blocked_payload(COMMAND, fallback_spec, fallback_gap, [str(exc)], real_mode_attempted=COMMAND == "real"))
    print(f"grimoire-fix: {exc}", file=sys.stderr)
    sys.exit(1)
PY
}

if [ "${#python_args[@]}" -gt 0 ]; then
  contract_exit=0
  write_contract "$mode" "$output_path" "$handoff_prompt_path" "$spec_sufficiency_path" "$plan_path" "$spec_gap_status_path" "$repo_root" "${python_args[@]}" || contract_exit=$?
else
  contract_exit=0
  write_contract "$mode" "$output_path" "$handoff_prompt_path" "$spec_sufficiency_path" "$plan_path" "$spec_gap_status_path" "$repo_root" || contract_exit=$?
fi

if [ "$contract_exit" -eq 0 ]; then
  case "$mode" in
    mock-fix)
      printf 'grimoire-fix: mock scoped fix passed; status written to %s and handoff prompt to %s\n' "$output_path" "$handoff_prompt_path"
      ;;
    mock-noop)
      printf 'grimoire-fix: mock no-op passed; status written to %s and handoff prompt to %s\n' "$output_path" "$handoff_prompt_path"
      ;;
    real)
      printf 'grimoire-fix: real-mode status written to %s and handoff prompt to %s\n' "$output_path" "$handoff_prompt_path"
      ;;
  esac
else
  case "$mode" in
    mock-scope-violation)
      printf 'grimoire-fix: mock scope violation failed closed; status written to %s\n' "$output_path" >&2
      ;;
    mock-blocked)
      printf 'grimoire-fix: mock blocked fixture failed closed; status written to %s\n' "$output_path" >&2
      ;;
    real)
      printf 'grimoire-fix: real mode blocked or failed closed; status written to %s\n' "$output_path" >&2
      ;;
    *)
      printf 'grimoire-fix: %s failed closed; status written to %s\n' "$mode" "$output_path" >&2
      ;;
  esac
  exit 1
fi
