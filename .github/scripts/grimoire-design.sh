#!/usr/bin/env bash
set -euo pipefail

readonly DEFAULT_REVIEW_INPUT=".omo/ci/review-findings.json"
readonly DEFAULT_OUTPUT=".omo/ci/spec-sufficiency.json"
readonly DEFAULT_PLAN=".omo/ci/design-plan.md"
readonly DEFAULT_SPEC_ROOT="openspec"
readonly DEFAULT_MODEL="ai-relay/gpt-5.5"
readonly DEFAULT_VARIANT="xhigh"

mode="${GRIMOIRE_DESIGN_MODE:-real}"
review_input="${GRIMOIRE_DESIGN_INPUT:-${DEFAULT_REVIEW_INPUT}}"
output_path="${GRIMOIRE_DESIGN_OUTPUT:-${DEFAULT_OUTPUT}}"
plan_path="${GRIMOIRE_DESIGN_PLAN:-${DEFAULT_PLAN}}"
spec_root="${GRIMOIRE_DESIGN_SPEC_ROOT:-${DEFAULT_SPEC_ROOT}}"
repo_root="${GITHUB_WORKSPACE:-$(pwd)}"
spec_paths=()

usage() {
  cat <<'USAGE'
Usage: grimoire-design.sh [options]

Spec-grounded grimoire design stage. Consumes Task 5 review findings and
writes Task 6 spec sufficiency plus a markdown-only design plan or halt note.

Options:
  --mode MODE          real, mock-sufficient, or mock-insufficient. Default: real.
  --input PATH         Review findings JSON. Default: .omo/ci/review-findings.json.
  --review-input PATH  Alias for --input.
  --spec PATH          OpenSpec evidence file or directory. Repeatable.
  --spec-root PATH     OpenSpec root for default discovery. Default: openspec.
  --output PATH        Spec sufficiency JSON. Default: .omo/ci/spec-sufficiency.json.
  --plan PATH          Markdown plan or halt note. Default: .omo/ci/design-plan.md.
  --repo-root PATH     Repository root for real-mode opencode execution.
  --help               Show this help text.

JSON contract:
  Stable root fields: spec_sufficient, bindings, missing,
  safety_default_gaps, suggested_spec_patch, plan_path, and halt_reason.

Modes:
  mock-sufficient     Deterministic local mode. Binds each review finding to
                      explicit OpenSpec evidence from --spec or discovered specs.
  mock-insufficient   Deterministic local mode for fail-closed gaps. Missing
                      OpenSpec evidence produces spec_sufficient=false and a
                      halt-only markdown note.
  real                Fail-closed unless AI_RELAY_API_KEY, opencode,
                      GRIMOIRE_DESIGN_READY=1, and GRIMOIRE_TEAM_MODE_ENABLED=1
                      are present. Uses opencode without --pure, asks Prometheus
                      for markdown-only planning, and forbids code/config/script
                      edits, GitHub mutation, comments, labels, pushes, and
                      secret disclosure.

OpenSpec evidence:
  If --spec is omitted, the script scans non-archived files under
  openspec/specs and openspec/changes. Archived specs are only used when passed
  explicitly with --spec so historical artifacts cannot silently approve work.
USAGE
}

fail_usage() {
  printf 'grimoire-design: %s\n\n' "$1" >&2
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
    --input|--review-input)
      [ "$#" -ge 2 ] || fail_usage "$1 requires a value"
      review_input="$2"
      shift 2
      ;;
    --spec)
      [ "$#" -ge 2 ] || fail_usage "--spec requires a value"
      spec_paths+=("$2")
      shift 2
      ;;
    --spec-root)
      [ "$#" -ge 2 ] || fail_usage "--spec-root requires a value"
      spec_root="$2"
      shift 2
      ;;
    --output)
      [ "$#" -ge 2 ] || fail_usage "--output requires a value"
      output_path="$2"
      shift 2
      ;;
    --plan)
      [ "$#" -ge 2 ] || fail_usage "--plan requires a value"
      plan_path="$2"
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
  real|mock-sufficient|mock-insufficient)
    ;;
  *)
    fail_usage "unsupported mode: ${mode}"
    ;;
esac

if ! command -v python3 >/dev/null 2>&1; then
  printf 'grimoire-design: python3 is required\n' >&2
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
PLAN = pathlib.Path(sys.argv[3])
REVIEW_INPUT = pathlib.Path(sys.argv[4])
SPEC_ROOT = pathlib.Path(sys.argv[5])
REPO_ROOT = pathlib.Path(sys.argv[6])
SPEC_ARGS = sys.argv[7:]
STAGE = "grimoire-design"
SUPPORTED_SPEC_SUFFIXES = {".md", ".txt", ".yaml", ".yml", ".json"}
REQUIRED_REVIEW_FIELDS = ["status", "approval_signal", "read_only", "mutation_allowed", "findings"]
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
REQUIRED_DESIGN_FIELDS = [
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


def ensure_stable_fields(payload):
    payload.setdefault("spec_sufficient", False)
    payload.setdefault("bindings", [])
    payload.setdefault("missing", [])
    payload.setdefault("safety_default_gaps", [])
    payload.setdefault("suggested_spec_patch", "")
    payload.setdefault("plan_path", path_for_output(PLAN))
    payload.setdefault("halt_reason", "")
    return payload


def write_json(payload):
    payload.setdefault("schema_version", 1)
    payload.setdefault("stage", STAGE)
    payload.setdefault("generated_at", utc_now())
    ensure_stable_fields(payload)
    OUTPUT.parent.mkdir(parents=True, exist_ok=True)
    OUTPUT.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def clean_markdown_text(text):
    text = text.strip()
    text = re.sub(r"^#+\s*", "", text)
    text = re.sub(r"^[-*]\s*", "", text)
    text = re.sub(r"^Requirement:\s*", "", text, flags=re.IGNORECASE)
    return text.strip()


def slugify(text):
    slug = re.sub(r"[^a-z0-9]+", "-", text.lower()).strip("-")
    return slug or "openspec-citation"


def load_json_file(path):
    if not path.exists():
        raise ContractError(f"review input does not exist: {path}")
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        raise ContractError(f"review input is not valid JSON: {exc}") from exc


def validate_review(candidate):
    if not isinstance(candidate, dict):
        raise ContractError("review input is not a JSON object")
    for field in REQUIRED_REVIEW_FIELDS:
        if field not in candidate:
            raise ContractError(f"review input missing root field: {field}")
    if candidate.get("read_only") is not True:
        raise ContractError("review input read_only must be true")
    if candidate.get("mutation_allowed") is not False:
        raise ContractError("review input mutation_allowed must be false")
    findings = candidate.get("findings")
    if not isinstance(findings, list):
        raise ContractError("review input findings must be an array")
    for index, finding in enumerate(findings):
        if not isinstance(finding, dict):
            raise ContractError(f"finding {index} is not an object")
        for field in REQUIRED_FINDING_FIELDS:
            if field not in finding:
                raise ContractError(f"finding {index} missing field: {field}")
    return candidate


def supported_spec_file(path):
    return path.is_file() and path.suffix.lower() in SUPPORTED_SPEC_SUFFIXES


def explicit_spec_files(paths):
    spec_files = []
    missing_inputs = []
    for raw_path in paths:
        path = pathlib.Path(raw_path)
        if not path.exists():
            missing_inputs.append(raw_path)
            continue
        if path.is_file():
            if supported_spec_file(path):
                spec_files.append(path)
            continue
        if path.is_dir():
            for child in sorted(path.rglob("*")):
                if supported_spec_file(child):
                    spec_files.append(child)
    return dedupe_paths(spec_files), missing_inputs


def default_spec_files():
    roots = [SPEC_ROOT / "specs", SPEC_ROOT / "changes"]
    spec_files = []
    for root in roots:
        if not root.exists():
            continue
        for child in sorted(root.rglob("*")):
            parts = set(child.parts)
            if "archive" in parts:
                continue
            if supported_spec_file(child):
                spec_files.append(child)
    return dedupe_paths(spec_files)


def dedupe_paths(paths):
    seen = set()
    result = []
    for path in paths:
        key = str(path.resolve()) if path.exists() else str(path)
        if key in seen:
            continue
        seen.add(key)
        result.append(path)
    return result


def read_spec_lines(spec_files):
    rows = []
    for path in spec_files:
        try:
            lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
        except OSError:
            continue
        output_path = path_for_output(path)
        for line_number, line in enumerate(lines, start=1):
            stripped = line.strip()
            if not stripped:
                continue
            rows.append(
                {
                    "path": output_path,
                    "line_number": line_number,
                    "line": stripped,
                    "lower": stripped.lower(),
                }
            )
    return rows


def finding_location(finding):
    file_name = str(finding.get("file", "")).strip()
    line = finding.get("line")
    if file_name and isinstance(line, int):
        return f"{file_name}:{line}"
    if file_name and str(line).isdigit():
        return f"{file_name}:{line}"
    return file_name or "unknown location"


def candidate_terms(finding):
    terms = []
    binding_key = str(finding.get("binding_key", "")).strip()
    if binding_key:
        terms.append(binding_key)
    location = finding_location(finding)
    if ":" in location:
        terms.append(location)
    title = str(finding.get("title", "")).strip()
    if len(title) >= 8:
        terms.append(title)
    suggested_fix = str(finding.get("suggested_fix", "")).strip()
    if len(suggested_fix) >= 30:
        terms.append(suggested_fix)
    deduped = []
    seen = set()
    for term in terms:
        normalized = " ".join(term.split()).lower()
        if normalized and normalized not in seen:
            seen.add(normalized)
            deduped.append(term)
    return deduped


def binding_for_finding(index, finding, spec_lines):
    terms = candidate_terms(finding)
    for row in spec_lines:
        for term in terms:
            if term.lower() not in row["lower"]:
                continue
            evidence = row["line"]
            requirement = clean_markdown_text(evidence) or str(finding.get("title", "")).strip()
            citation = f"{row['path']}:{row['line_number']}"
            title = str(finding.get("title", "")).strip()
            return {
                "binding_key": finding_location(finding),
                "citation": citation,
                "citations": [citation],
                "evidence": evidence,
                "finding_file": str(finding.get("file", "")),
                "finding_index": index,
                "finding_line": finding.get("line"),
                "finding_title": title,
                "matched_term": term,
                "requirement": requirement,
                "section_citation": f"{row['path']}#{slugify(requirement)}",
                "spec_line": row["line_number"],
                "spec_path": row["path"],
            }
    return None


def missing_for_finding(index, finding, reason):
    title = str(finding.get("title", "")).strip() or "Untitled finding"
    location = finding_location(finding)
    return {
        "finding_index": index,
        "finding_file": str(finding.get("file", "")),
        "finding_line": finding.get("line"),
        "finding_title": title,
        "location": location,
        "reason": reason,
        "required_evidence": (
            "Add an OpenSpec requirement or scenario that explicitly names this "
            "finding, its target file:line, or an equivalent binding key."
        ),
        "suggested_spec_section": (
            f"### Requirement: {title}\n"
            f"- Binding key: {location}\n"
            "- Required behavior: describe the intended behavior and safety default.\n"
            "- Acceptance: cite the observable pass/fail condition."
        ),
    }


def safety_gap(scope, gap, required_default):
    return {"scope": scope, "gap": gap, "required_default": required_default}


def suggested_patch_text(missing):
    if not missing:
        return "No patch required."
    lines = [
        "Add or update OpenSpec coverage before rerunning grimoire design:",
    ]
    for item in missing:
        title = item.get("finding_title", "Missing OpenSpec evidence")
        location = item.get("location", item.get("scope", "unknown"))
        lines.append(f"- {title} ({location}): {item.get('required_evidence', item.get('reason', 'Provide explicit OpenSpec evidence.'))}")
    return "\n".join(lines)


def write_sufficient_plan(review, bindings):
    PLAN.parent.mkdir(parents=True, exist_ok=True)
    findings_count = len(review.get("findings", []))
    lines = [
        "# Grimoire Design Plan",
        "",
        "## Spec Sufficiency Report",
        "",
        "- `spec_sufficient`: `true`",
        f"- `review_findings_count`: `{findings_count}`",
        f"- `bindings_count`: `{len(bindings)}`",
        "- `missing_count`: `0`",
        "- `safety_default_gaps_count`: `0`",
        "",
        "### Citations",
        "",
    ]
    if bindings:
        for binding in bindings:
            lines.append(
                "- Finding "
                f"`{binding['binding_key']}` `{binding['finding_title']}` -> "
                f"`{binding['citation']}` (`{binding['requirement']}`)."
            )
    else:
        lines.append("- None; Task 5 reported no review findings requiring implementation.")
    lines.extend(
        [
            "",
            "### Missing Evidence",
            "",
            "- None.",
            "",
            "### Safety-Default Gaps",
            "",
            "- None.",
            "",
            "### Suggested Spec Patch",
            "",
            "No patch required.",
            "",
            "## Plan",
            "",
        ]
    )
    if bindings:
        for binding in bindings:
            lines.append(
                "- [ ] Address "
                f"`{binding['finding_title']}` in `{binding['binding_key']}` by applying the review fix. "
                f"Grounding: `{binding['citation']}`."
            )
    else:
        lines.append("No implementation plan is needed because the review stage produced no findings.")
    lines.extend(
        [
            "",
            "## Execution Boundary",
            "",
            "This plan is approved only because every review finding is bound to explicit OpenSpec evidence above. Later fix stages must not expand scope beyond these cited bindings.",
            "",
        ]
    )
    PLAN.write_text("\n".join(lines), encoding="utf-8")


def write_halt_plan(halt_reason, missing, safety_default_gaps, suggested_spec_patch):
    PLAN.parent.mkdir(parents=True, exist_ok=True)
    lines = [
        "# Grimoire Design Halt",
        "",
        "## Spec Sufficiency Report",
        "",
        "- `spec_sufficient`: `false`",
        f"- `halt_reason`: {halt_reason}",
        f"- `missing_count`: `{len(missing)}`",
        f"- `safety_default_gaps_count`: `{len(safety_default_gaps)}`",
        "",
        "## Missing OpenSpec Evidence",
        "",
    ]
    if missing:
        for item in missing:
            title = item.get("finding_title", item.get("scope", "Missing evidence"))
            reason = item.get("reason", "OpenSpec evidence is absent.")
            location = item.get("location", item.get("scope", "unknown"))
            lines.append(f"- `{location}` — {title}: {reason}")
    else:
        lines.append("- None recorded.")
    lines.extend(["", "## Safety-Default Gaps", ""])
    if safety_default_gaps:
        for item in safety_default_gaps:
            lines.append(
                f"- `{item.get('scope', 'openspec')}` — {item.get('gap', 'gap not specified')} "
                f"Required default: {item.get('required_default', 'halt before planning')}."
            )
    else:
        lines.append("- None recorded.")
    lines.extend(
        [
            "",
            "## Suggested Spec Patch",
            "",
            suggested_spec_patch or "Add explicit OpenSpec evidence before rerunning.",
            "",
            "## Non-Executable Boundary",
            "",
            "This is a halt-only artifact. It is not an implementation plan and must not be used by the fix stage as approval to edit code.",
            "",
        ]
    )
    PLAN.write_text("\n".join(lines), encoding="utf-8")


def insufficient_payload(mode, review, missing, safety_default_gaps, halt_reason):
    suggested_spec_patch = suggested_patch_text(missing)
    write_halt_plan(halt_reason, missing, safety_default_gaps, suggested_spec_patch)
    payload = {
        "mode": mode,
        "status": "halted",
        "spec_sufficient": False,
        "bindings": [],
        "missing": missing,
        "safety_default_gaps": safety_default_gaps,
        "suggested_spec_patch": suggested_spec_patch,
        "plan_path": path_for_output(PLAN),
        "halt_reason": halt_reason,
        "review_findings_count": len(review.get("findings", [])) if isinstance(review, dict) else 0,
        "real_mode_attempted": mode == "real",
    }
    write_json(payload)


def sufficient_payload(mode, review, bindings):
    write_sufficient_plan(review, bindings)
    payload = {
        "mode": mode,
        "status": "planned",
        "spec_sufficient": True,
        "bindings": bindings,
        "missing": [],
        "safety_default_gaps": [],
        "suggested_spec_patch": "No patch required.",
        "plan_path": path_for_output(PLAN),
        "halt_reason": "",
        "review_findings_count": len(review.get("findings", [])),
        "real_mode_attempted": mode == "real",
    }
    write_json(payload)


def run_deterministic(mode):
    review = validate_review(load_json_file(REVIEW_INPUT))
    findings = review.get("findings", [])
    if review.get("status") == "blocked" or review.get("approval_signal") == "GRIMOIRE_REVIEW_BLOCKED":
        missing = [
            {
                "scope": "review-input",
                "location": path_for_output(REVIEW_INPUT),
                "finding_title": "Review stage blocked",
                "reason": "Task 5 did not produce actionable read-only findings, so Task 6 cannot infer requirements.",
                "required_evidence": "Rerun Task 5 until it emits approved or findings status with the stable review contract.",
            }
        ]
        safety = [
            safety_gap(
                "review-input",
                "Design cannot safely plan from a blocked review artifact.",
                "Halt before Prometheus planning and preserve the blocked JSON.",
            )
        ]
        insufficient_payload(mode, review, missing, safety, "review findings input is blocked")
        return 1

    spec_files, missing_spec_inputs = explicit_spec_files(SPEC_ARGS) if SPEC_ARGS else (default_spec_files(), [])
    missing = []
    safety = []
    for raw_path in missing_spec_inputs:
        missing.append(
            {
                "scope": "openspec-input",
                "location": raw_path,
                "finding_title": "OpenSpec evidence path missing",
                "reason": f"Explicit --spec path does not exist: {raw_path}",
                "required_evidence": "Pass an existing OpenSpec spec/change file or directory.",
            }
        )
        safety.append(
            safety_gap(
                raw_path,
                "An explicit OpenSpec input path was missing.",
                "Halt instead of dropping the missing evidence silently.",
            )
        )

    if not findings:
        if missing:
            insufficient_payload(mode, review, missing, safety, "explicit OpenSpec evidence path is missing")
            return 1
        sufficient_payload(mode, review, [])
        return 0

    if not spec_files:
        for index, finding in enumerate(findings):
            missing.append(missing_for_finding(index, finding, "No OpenSpec evidence files were available for binding."))
        safety.append(
            safety_gap(
                path_for_output(SPEC_ROOT),
                "No non-archived OpenSpec spec or change files were available for required bindings.",
                "Set spec_sufficient=false and write a halt-only plan.",
            )
        )
        insufficient_payload(mode, review, missing, safety, "OpenSpec evidence is absent for review findings")
        return 1

    spec_lines = read_spec_lines(spec_files)
    bindings = []
    for index, finding in enumerate(findings):
        binding = binding_for_finding(index, finding, spec_lines)
        if binding is None:
            missing.append(missing_for_finding(index, finding, "No supplied OpenSpec line matched the finding title, binding key, or file:line."))
            safety.append(
                safety_gap(
                    finding_location(finding),
                    "Review finding lacks an explicit OpenSpec binding.",
                    "Halt before creating an executable plan for this finding.",
                )
            )
        else:
            bindings.append(binding)

    if missing:
        insufficient_payload(mode, review, missing, safety, "one or more review findings lack OpenSpec citations")
        return 1
    sufficient_payload(mode, review, bindings)
    return 0


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


def validate_design_object(candidate):
    if not isinstance(candidate, dict):
        return False, "candidate is not a JSON object"
    for field in REQUIRED_DESIGN_FIELDS:
        if field not in candidate:
            return False, f"missing root field: {field}"
    if not isinstance(candidate.get("spec_sufficient"), bool):
        return False, "spec_sufficient must be boolean"
    if not isinstance(candidate.get("bindings"), list):
        return False, "bindings must be an array"
    if not isinstance(candidate.get("missing"), list):
        return False, "missing must be an array"
    if not isinstance(candidate.get("safety_default_gaps"), list):
        return False, "safety_default_gaps must be an array"
    if not isinstance(candidate.get("suggested_spec_patch"), str):
        return False, "suggested_spec_patch must be a string"
    if not isinstance(candidate.get("plan_path"), str):
        return False, "plan_path must be a string"
    if not isinstance(candidate.get("halt_reason"), str):
        return False, "halt_reason must be a string"
    if candidate["spec_sufficient"] and (candidate["missing"] or candidate["halt_reason"]):
        return False, "sufficient output must not include missing items or halt_reason"
    if not candidate["spec_sufficient"] and not candidate["halt_reason"]:
        return False, "insufficient output must include halt_reason"
    return True, "ok"


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
        ok, reason = validate_design_object(candidate)
        if not ok:
            last_error = reason
            continue
        candidate["schema_version"] = 1
        candidate["stage"] = STAGE
        candidate.setdefault("mode", "real")
        candidate["real_mode_attempted"] = True
        candidate["plan_path"] = path_for_output(PLAN)
        ensure_stable_fields(candidate)
        if candidate["spec_sufficient"]:
            if not PLAN.exists():
                write_sufficient_plan({"findings": []}, candidate["bindings"])
            write_json(candidate)
            return 0
        write_halt_plan(
            candidate["halt_reason"],
            candidate["missing"],
            candidate["safety_default_gaps"],
            candidate["suggested_spec_patch"],
        )
        write_json(candidate)
        return 1
    insufficient_payload(
        "real",
        {},
        [
            {
                "scope": "opencode-output",
                "location": raw_path,
                "finding_title": "Invalid real-mode design output",
                "reason": f"opencode did not return a valid design JSON object: {last_error}",
                "required_evidence": "Return exactly the Task 6 spec sufficiency JSON contract.",
            }
        ],
        [
            safety_gap(
                "opencode-output",
                "Model output did not satisfy the machine contract.",
                "Fail closed and do not reuse any previous plan.",
            )
        ],
        "opencode real-mode output failed contract validation",
    )
    return 1


def blocked(mode, blockers):
    missing = []
    safety = []
    for blocker in blockers:
        missing.append(
            {
                "scope": "real-mode-prerequisite",
                "location": "environment/tooling",
                "finding_title": "Real-mode prerequisite missing",
                "reason": blocker,
                "required_evidence": "Provide the prerequisite and rerun only after the CI readiness gate proves it safe.",
            }
        )
        safety.append(
            safety_gap(
                "real-mode-prerequisite",
                blocker,
                "Write blocked JSON and exit nonzero before any model call.",
            )
        )
    insufficient_payload(mode, {}, missing, safety, "real mode prerequisites missing")


if COMMAND == "blocked":
    blocked(sys.argv[7] if len(sys.argv) > 7 else "real", sys.argv[8:])
    sys.exit(0)
if COMMAND in {"mock-sufficient", "mock-insufficient"}:
    sys.exit(run_deterministic(COMMAND))
if COMMAND == "extract-real":
    sys.exit(extract_real(sys.argv[7]))
raise SystemExit(f"unknown command: {COMMAND}")
PY
}

write_blocked() {
  write_contract blocked "$output_path" "$plan_path" "$review_input" "$spec_root" "$repo_root" "$mode" "$@"
}

write_contract_with_optional_specs() {
  if [ "${#spec_paths[@]}" -gt 0 ]; then
    write_contract "$@" "${spec_paths[@]}"
  else
    write_contract "$@"
  fi
}

run_mock_sufficient() {
  if write_contract_with_optional_specs mock-sufficient "$output_path" "$plan_path" "$review_input" "$spec_root" "$repo_root"; then
    printf 'grimoire-design: wrote sufficient spec artifact to %s and plan to %s\n' "$output_path" "$plan_path"
  else
    printf 'grimoire-design: mock-sufficient failed closed; artifact written to %s\n' "$output_path" >&2
    exit 1
  fi
}

run_mock_insufficient() {
  if write_contract_with_optional_specs mock-insufficient "$output_path" "$plan_path" "$review_input" "$spec_root" "$repo_root"; then
    printf 'grimoire-design: mock-insufficient unexpectedly produced a sufficient plan at %s\n' "$plan_path" >&2
    exit 1
  fi
  printf 'grimoire-design: wrote insufficient spec artifact to %s and halt note to %s\n' "$output_path" "$plan_path" >&2
  exit 1
}

markdown_only_permission_json() {
  cat <<'JSON'
{"read":{"*":"allow","*.env":"deny","*.env.*":"deny","*.env.example":"allow"},"edit":{".omo/plans/**":"allow",".omo/ci/*.md":"allow","*.md":"allow","*":"deny"},"bash":"deny","task":"deny","webfetch":"deny","external_directory":"deny","question":"deny","plan_enter":"deny","plan_exit":"deny"}
JSON
}

spec_input_summary() {
  if [ "${#spec_paths[@]}" -eq 0 ]; then
    printf 'default non-archived OpenSpec discovery under %s' "$spec_root"
    return
  fi
  printf '%s\n' "${spec_paths[@]}"
}

real_design_prompt() {
  cat <<PROMPT
You are running the grimoire Task 6 design stage for this repository.

Operate as spec-grounded Prometheus in non-interactive CI mode:
- Do not ask questions, open interviews, or use any Question tool.
- Do not edit source code, config, scripts, workflows, GitHub state, labels, comments, commits, pushes, or secrets.
- Planning writes are markdown-only and limited to ${plan_path} or other .omo markdown plan/draft paths.
- Use no GitHub mutation, no workflow dispatch, no pull request comment, no label change, and no push.
- Do not reveal, transform, list, or summarize secret values.
- If OpenSpec evidence is absent or insufficient, halt. Do not infer implementation requirements.

Inputs:
- Review findings JSON: ${review_input}
- Spec sufficiency output JSON path: ${output_path}
- Markdown plan/halt path: ${plan_path}
- OpenSpec evidence inputs: $(spec_input_summary)

Return exactly one JSON object and no prose with this schema:
{
  "spec_sufficient": true | false,
  "bindings": [
    {
      "finding_index": 0,
      "binding_key": "path:line",
      "finding_title": "title",
      "requirement": "OpenSpec requirement title",
      "citation": "openspec/path/spec.md:12",
      "citations": ["openspec/path/spec.md:12"],
      "evidence": "exact cited OpenSpec line"
    }
  ],
  "missing": [
    {
      "finding_index": 0,
      "location": "path:line",
      "finding_title": "title",
      "reason": "what OpenSpec evidence is missing",
      "required_evidence": "what spec item must be added"
    }
  ],
  "safety_default_gaps": [
    {
      "scope": "finding or OpenSpec path",
      "gap": "safety default missing",
      "required_default": "halt/no-guess default needed"
    }
  ],
  "suggested_spec_patch": "readable markdown bullet list, or No patch required.",
  "plan_path": "${plan_path}",
  "halt_reason": "empty when sufficient; concrete halt reason when insufficient"
}

Sufficient output requirements:
- spec_sufficient=true only if every review finding is bound to explicit OpenSpec evidence.
- Each binding must include a path:line citation and the same citation must appear in the markdown plan.
- The markdown plan must remain implementation planning only, not code mutation.

Insufficient output requirements:
- spec_sufficient=false when evidence is absent, ambiguous, stale-only, or incomplete.
- Include concrete missing items and suggested spec patch text.
- Overwrite ${plan_path} with a halt-only note or leave no executable plan.
PROMPT
}

run_real() {
  blockers=()
  if [ -z "${AI_RELAY_API_KEY:-}" ]; then
    blockers+=("AI_RELAY_API_KEY is not set")
  fi
  if [ "${GRIMOIRE_DESIGN_READY:-}" != "1" ]; then
    blockers+=("GRIMOIRE_DESIGN_READY=1 readiness gate is not set")
  fi
  if [ "${GRIMOIRE_TEAM_MODE_ENABLED:-}" != "1" ]; then
    blockers+=("GRIMOIRE_TEAM_MODE_ENABLED=1 readiness gate is not set")
  fi
  if ! command -v opencode >/dev/null 2>&1; then
    blockers+=("opencode CLI is not available")
  fi
  if [ "${#blockers[@]}" -gt 0 ]; then
    write_blocked "${blockers[@]}"
    printf 'grimoire-design: real mode blocked; artifact written to %s\n' "$output_path" >&2
    exit 1
  fi

  tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/grimoire-design.XXXXXX")"
  prompt_file="${tmp_dir}/prompt.txt"
  opencode_output="${tmp_dir}/opencode.jsonl"
  real_design_prompt > "$prompt_file"

  OPENCODE_PERMISSION="$(markdown_only_permission_json)"
  export OPENCODE_PERMISSION
  export OPENCODE_MODEL="${OPENCODE_MODEL:-${DEFAULT_MODEL}}"
  export OPENCODE_VARIANT="${OPENCODE_VARIANT:-${DEFAULT_VARIANT}}"
  prompt_text="$(<"$prompt_file")"

  if ! opencode run \
    --format json \
    --dir "$repo_root" \
    --agent prometheus \
    --model "$OPENCODE_MODEL" \
    --variant "$OPENCODE_VARIANT" \
    "$prompt_text" > "$opencode_output" 2>&1; then
    write_blocked "opencode run exited nonzero in markdown-only real mode"
    printf 'grimoire-design: real mode failed closed; artifact written to %s\n' "$output_path" >&2
    exit 1
  fi

  if write_contract extract-real "$output_path" "$plan_path" "$review_input" "$spec_root" "$repo_root" "$opencode_output"; then
    printf 'grimoire-design: wrote real design artifact to %s and plan to %s\n' "$output_path" "$plan_path"
  else
    printf 'grimoire-design: real mode halted or failed contract validation; artifact written to %s\n' "$output_path" >&2
    exit 1
  fi
}

case "$mode" in
  mock-sufficient)
    run_mock_sufficient
    ;;
  mock-insufficient)
    run_mock_insufficient
    ;;
  real)
    run_real
    ;;
esac
