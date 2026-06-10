#!/usr/bin/env bash
set -euo pipefail

readonly DEFAULT_INPUT=".omo/ci/spec-sufficiency.json"
readonly DEFAULT_COMMENT_OUTPUT=".omo/ci/spec-gap-comment.md"
readonly DEFAULT_STATUS_OUTPUT=".omo/ci/spec-gap-status.json"

mode="${GRIMOIRE_SPEC_GAP_MODE:-render}"
input_path="${GRIMOIRE_SPEC_GAP_INPUT:-${DEFAULT_INPUT}}"
comment_output="${GRIMOIRE_SPEC_GAP_COMMENT:-${DEFAULT_COMMENT_OUTPUT}}"
status_output="${GRIMOIRE_SPEC_GAP_STATUS:-${DEFAULT_STATUS_OUTPUT}}"
repo_root="${GITHUB_WORKSPACE:-$(pwd)}"

usage() {
  cat <<'USAGE'
Usage: grimoire-spec-gap-comment.sh [options]

Render the grimoire Task 7 spec-gap PR comment artifact from Task 6
spec-sufficiency JSON. This stage is local-only: it writes configured files,
sets machine-readable halt/comment signals, and performs no live GitHub
mutation, source-code edit, commit, push, label change, or model call.

Options:
  --mode MODE             render, mock-insufficient, or mock-sufficient.
                          Default: render.
  --input PATH            Task 6 spec sufficiency JSON.
                          Default: .omo/ci/spec-sufficiency.json.
  --comment-output PATH   Markdown PR-comment artifact.
                          Default: .omo/ci/spec-gap-comment.md.
  --comment PATH          Alias for --comment-output.
  --status-output PATH    Machine-readable status JSON.
                          Default: .omo/ci/spec-gap-status.json.
  --status PATH           Alias for --status-output.
  --repo-root PATH        Repository root used for relative artifact paths.
  --help                  Show this help text.

Input contract:
  Consumes Task 6 stable fields: spec_sufficient, bindings, missing,
  safety_default_gaps, suggested_spec_patch, plan_path, and halt_reason.

Outputs:
  Insufficient specs write a readable Markdown artifact with exactly five
  top-level sections: Summary, Intended Work, Missing OpenSpec Evidence,
  Suggested Spec Items, and How To Rerun. Status sets should_comment=true,
  should_halt=true, github_mutation_performed=false, and
  no_code_or_push_action=true.

  Sufficient specs empty the configured comment artifact so stale text cannot
  be reused. Status sets should_comment=false and should_halt=false while still
  recording github_mutation_performed=false and no_code_or_push_action=true.

Mock modes:
  mock-insufficient       Deterministic local insufficient fixture; no ai-relay,
                          gh, PR mutation, source edit, commit, or push.
  mock-sufficient         Deterministic local sufficient fixture; clears any
                          stale comment artifact and writes no comment body.
USAGE
}

fail_usage() {
  printf 'grimoire-spec-gap-comment: %s\n\n' "$1" >&2
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
    --input)
      [ "$#" -ge 2 ] || fail_usage "--input requires a value"
      input_path="$2"
      shift 2
      ;;
    --comment-output|--comment)
      [ "$#" -ge 2 ] || fail_usage "$1 requires a value"
      comment_output="$2"
      shift 2
      ;;
    --status-output|--status)
      [ "$#" -ge 2 ] || fail_usage "$1 requires a value"
      status_output="$2"
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
  render|mock-insufficient|mock-sufficient)
    ;;
  *)
    fail_usage "unsupported mode: ${mode}"
    ;;
esac

if ! command -v python3 >/dev/null 2>&1; then
  printf 'grimoire-spec-gap-comment: python3 is required\n' >&2
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
INPUT = pathlib.Path(sys.argv[2])
COMMENT = pathlib.Path(sys.argv[3])
STATUS = pathlib.Path(sys.argv[4])
REPO_ROOT = pathlib.Path(sys.argv[5])
STAGE = "grimoire-spec-gap-comment"
SECTIONS = [
    "Summary",
    "Intended Work",
    "Missing OpenSpec Evidence",
    "Suggested Spec Items",
    "How To Rerun",
]
REQUIRED_FIELDS = [
    "spec_sufficient",
    "bindings",
    "missing",
    "safety_default_gaps",
    "suggested_spec_patch",
    "plan_path",
    "halt_reason",
]


class ContractError(Exception):
    pass


def utc_now():
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def path_for_output(path):
    try:
        resolved = path.resolve()
        root = REPO_ROOT.resolve()
        return resolved.relative_to(root).as_posix()
    except (OSError, ValueError):
        return path.as_posix()


def inline_code(value):
    text = str(value).strip() if value is not None else ""
    if not text:
        text = "not provided"
    return "`" + text.replace("`", "\\`") + "`"


def plain(value, fallback="not provided"):
    text = str(value).strip() if value is not None else ""
    text = re.sub(r"\s+", " ", text)
    return text or fallback


def quote_block(text, fallback="No suggested patch was provided by Task 6."):
    content = str(text).rstrip() if text is not None else ""
    if not content.strip():
        content = fallback
    return "\n".join("> " + line if line else ">" for line in content.splitlines())


def write_json(payload):
    payload.setdefault("schema_version", 1)
    payload.setdefault("stage", STAGE)
    payload.setdefault("generated_at", utc_now())
    STATUS.parent.mkdir(parents=True, exist_ok=True)
    STATUS.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def load_payload():
    if not INPUT.exists():
        raise ContractError(f"input does not exist: {INPUT}")
    try:
        return json.loads(INPUT.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        raise ContractError(f"input is not valid JSON: {exc}") from exc


def validate_payload(payload):
    if not isinstance(payload, dict):
        raise ContractError("Task 6 input must be a JSON object")
    for field in REQUIRED_FIELDS:
        if field not in payload:
            raise ContractError(f"Task 6 input missing root field: {field}")
    if not isinstance(payload.get("spec_sufficient"), bool):
        raise ContractError("spec_sufficient must be boolean")
    for field in ("bindings", "missing", "safety_default_gaps"):
        if not isinstance(payload.get(field), list):
            raise ContractError(f"{field} must be an array")
    for field in ("suggested_spec_patch", "plan_path", "halt_reason"):
        if not isinstance(payload.get(field), str):
            raise ContractError(f"{field} must be a string")
    if payload["spec_sufficient"] and (payload["missing"] or payload["halt_reason"]):
        raise ContractError("sufficient Task 6 input must not include missing items or halt_reason")
    if not payload["spec_sufficient"] and not payload["halt_reason"]:
        raise ContractError("insufficient Task 6 input must include halt_reason")
    return payload


def mock_insufficient_payload():
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
                "finding_file": "src/demo.rs",
                "finding_line": 42,
                "finding_title": "Deterministic review defect marker present",
                "location": "src/demo.rs:42",
                "reason": "No supplied OpenSpec line matched the finding title, binding key, or file:line.",
                "required_evidence": "Add an OpenSpec requirement or scenario that explicitly names the affected file:line or equivalent binding key.",
                "suggested_spec_section": "### Requirement: Deterministic review defect marker present\n- Binding key: src/demo.rs:42\n- Required behavior: describe the intended safe behavior and default halt condition.\n- Acceptance: cite the observable pass/fail condition.",
            }
        ],
        "safety_default_gaps": [
            {
                "scope": "src/demo.rs:42",
                "gap": "Review finding lacks an explicit OpenSpec binding.",
                "required_default": "Halt before creating an executable plan for this finding.",
            }
        ],
        "suggested_spec_patch": "Add or update OpenSpec coverage before rerunning grimoire design:\n- Deterministic review defect marker present (src/demo.rs:42): Add an OpenSpec requirement or scenario that explicitly names the affected file:line or equivalent binding key.",
        "plan_path": ".omo/ci/design-plan.md",
        "halt_reason": "one or more review findings lack OpenSpec citations",
        "review_findings_count": 1,
    }


def mock_sufficient_payload():
    return {
        "schema_version": 1,
        "stage": "grimoire-design",
        "mode": "mock-sufficient",
        "status": "planned",
        "spec_sufficient": True,
        "bindings": [
            {
                "finding_index": 0,
                "binding_key": "src/demo.rs:42",
                "finding_title": "Deterministic review defect marker present",
                "requirement": "Deterministic review defect marker present",
                "citation": "openspec/changes/demo/specs/demo/spec.md:12",
                "citations": ["openspec/changes/demo/specs/demo/spec.md:12"],
                "evidence": "### Requirement: Deterministic review defect marker present",
            }
        ],
        "missing": [],
        "safety_default_gaps": [],
        "suggested_spec_patch": "No patch required.",
        "plan_path": ".omo/ci/design-plan.md",
        "halt_reason": "",
        "review_findings_count": 1,
    }


def missing_title(item):
    return plain(item.get("finding_title") or item.get("title") or item.get("scope"), "Missing OpenSpec evidence")


def missing_location(item):
    return plain(item.get("location") or item.get("binding_key") or item.get("scope"), "unknown location")


def render_comment(payload):
    missing = payload.get("missing", [])
    safety_gaps = payload.get("safety_default_gaps", [])
    halt_reason = plain(payload.get("halt_reason"), "OpenSpec evidence is insufficient")
    plan_path = plain(payload.get("plan_path"), "not provided")
    review_count = payload.get("review_findings_count")
    if not isinstance(review_count, int):
        review_count = len(missing) if missing else len(payload.get("bindings", []))

    lines = []
    lines.extend(
        [
            "## Summary",
            "",
            "- **Decision:** Grimoire stopped before implementation because OpenSpec evidence is not sufficient for the requested work.",
            f"- **Halt reason:** {halt_reason}",
            f"- **Machine signal:** `should_halt=true`, `should_comment=true`, `github_mutation_performed=false`, `no_code_or_push_action=true`.",
            "- **What this means:** no code edit, commit, push, label change, merge, or live GitHub mutation is authorized by this stage.",
            "",
            "## Intended Work",
            "",
            f"- Task 6 tried to turn **{review_count}** review finding(s) into a spec-grounded Prometheus plan at {inline_code(plan_path)}.",
            "- The fix stage can only proceed when each finding has explicit OpenSpec evidence with a stable citation.",
        ]
    )
    if missing:
        lines.append("- Work blocked by these unbound finding(s):")
        for item in missing:
            lines.append(f"  - {missing_title(item)} at {inline_code(missing_location(item))}")
    else:
        lines.append("- Work blocked by a non-finding OpenSpec safety gap recorded by Task 6.")

    lines.extend(["", "## Missing OpenSpec Evidence", ""])
    if missing:
        lines.append("**Finding-level evidence gaps**")
        lines.append("")
        for index, item in enumerate(missing, start=1):
            lines.append(f"- [ ] **Gap {index}: {missing_title(item)}**")
            lines.append(f"  - **Location:** {inline_code(missing_location(item))}")
            lines.append(f"  - **Why it halted:** {plain(item.get('reason'), 'Task 6 did not find explicit OpenSpec evidence.')}")
            lines.append(f"  - **Evidence needed:** {plain(item.get('required_evidence'), 'Add explicit OpenSpec requirement and acceptance evidence.')}")
    else:
        lines.append("- [ ] Task 6 did not record finding-level gaps, but still marked the spec insufficient. Inspect the safety-default gaps below.")
    lines.append("")
    lines.append("**Safety defaults still missing**")
    lines.append("")
    if safety_gaps:
        for index, item in enumerate(safety_gaps, start=1):
            lines.append(f"- [ ] **Safety gap {index}:** {plain(item.get('gap'), 'Safety default missing.')}")
            lines.append(f"  - **Scope:** {inline_code(plain(item.get('scope'), 'OpenSpec'))}")
            lines.append(f"  - **Required default:** {plain(item.get('required_default'), 'Halt until the spec is explicit.')}")
    else:
        lines.append("- [x] No separate safety-default gaps were recorded by Task 6.")

    lines.extend(["", "## Suggested Spec Items", ""])
    if missing:
        lines.append("Add the missing OpenSpec coverage before asking grimoire to plan or fix this PR:")
        lines.append("")
        for index, item in enumerate(missing, start=1):
            lines.append(f"- [ ] **Spec item {index}:** cover {missing_title(item)} at {inline_code(missing_location(item))}.")
            lines.append(f"  - Include the intended behavior, safety default, and an observable acceptance condition.")
            suggested_section = str(item.get("suggested_spec_section", "")).strip()
            if suggested_section:
                lines.append("  - Task 6 suggested section:")
                lines.append(quote_block(suggested_section))
    else:
        lines.append("- [ ] Add an OpenSpec requirement or scenario for the unresolved safety-default gap recorded above.")
    lines.append("")
    lines.append("**Task 6 suggested patch, preserved**")
    lines.append("")
    lines.append(quote_block(payload.get("suggested_spec_patch")))

    lines.extend(
        [
            "",
            "## How To Rerun",
            "",
            "1. Update OpenSpec first. Use `/opsx:propose` for a new change or `/opsx:apply` when extending an existing change, then include the missing requirement, scenario, and acceptance evidence.",
            "2. Push or synchronize the PR after the spec update, or reopen the PR if it was closed and reopened for the correction.",
            "3. Rerun grimoire only after the OpenSpec evidence is present. The requirement cannot be bypassed by a comment, label, approval phrase, merge, or agent guess.",
            "4. Expect this stage to clear the spec-gap body automatically when Task 6 later reports `spec_sufficient=true`, so stale halt text is not reused.",
            "",
        ]
    )
    body = "\n".join(lines)
    section_matches = re.findall(r"^## ", body, flags=re.MULTILINE)
    if len(section_matches) != len(SECTIONS):
        raise ContractError("rendered comment does not contain exactly five top-level sections")
    for section in SECTIONS:
        if f"## {section}" not in body:
            raise ContractError(f"rendered comment missing section: {section}")
    return body


def write_status(payload, status, should_comment, should_halt, comment_written, comment_cleared):
    write_json(
        {
            "mode": COMMAND,
            "status": status,
            "input_path": path_for_output(INPUT),
            "comment_path": path_for_output(COMMENT),
            "status_path": path_for_output(STATUS),
            "spec_sufficient": payload.get("spec_sufficient"),
            "should_comment": should_comment,
            "should_halt": should_halt,
            "github_mutation_performed": False,
            "no_code_or_push_action": True,
            "comment_artifact_written": comment_written,
            "comment_artifact_cleared": comment_cleared,
            "stale_comment_reuse_prevented": comment_cleared,
            "top_level_sections": SECTIONS if should_comment else [],
            "top_level_section_count": len(SECTIONS) if should_comment else 0,
            "missing_count": len(payload.get("missing", [])),
            "safety_default_gaps_count": len(payload.get("safety_default_gaps", [])),
            "halt_reason": payload.get("halt_reason", ""),
        }
    )


def render(payload):
    COMMENT.parent.mkdir(parents=True, exist_ok=True)
    if payload["spec_sufficient"]:
        COMMENT.write_text("", encoding="utf-8")
        write_status(payload, "clear", False, False, False, True)
        return 0
    body = render_comment(payload)
    COMMENT.write_text(body, encoding="utf-8")
    write_status(payload, "halted", True, True, True, False)
    return 0


def main():
    if COMMAND == "mock-insufficient":
        return render(validate_payload(mock_insufficient_payload()))
    if COMMAND == "mock-sufficient":
        return render(validate_payload(mock_sufficient_payload()))
    if COMMAND == "render":
        return render(validate_payload(load_payload()))
    raise ContractError(f"unsupported command: {COMMAND}")


try:
    sys.exit(main())
except ContractError as exc:
    COMMENT.parent.mkdir(parents=True, exist_ok=True)
    COMMENT.write_text("", encoding="utf-8")
    write_json(
        {
            "mode": COMMAND,
            "status": "blocked",
            "input_path": path_for_output(INPUT),
            "comment_path": path_for_output(COMMENT),
            "status_path": path_for_output(STATUS),
            "spec_sufficient": False,
            "should_comment": False,
            "should_halt": True,
            "github_mutation_performed": False,
            "no_code_or_push_action": True,
            "comment_artifact_written": False,
            "comment_artifact_cleared": True,
            "stale_comment_reuse_prevented": True,
            "blocked_reason": str(exc),
        }
    )
    print(f"grimoire-spec-gap-comment: {exc}", file=sys.stderr)
    sys.exit(1)
PY
}

write_contract "$mode" "$input_path" "$comment_output" "$status_output" "$repo_root"
case "$mode" in
  render)
    printf 'grimoire-spec-gap-comment: rendered status to %s and comment artifact to %s\n' "$status_output" "$comment_output"
    ;;
  mock-insufficient)
    printf 'grimoire-spec-gap-comment: rendered deterministic insufficient fixture to %s and %s\n' "$comment_output" "$status_output"
    ;;
  mock-sufficient)
    printf 'grimoire-spec-gap-comment: cleared deterministic sufficient fixture comment at %s and wrote %s\n' "$comment_output" "$status_output"
    ;;
esac
