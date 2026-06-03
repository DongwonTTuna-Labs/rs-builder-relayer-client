"""CLI dispatcher for Codex Review v3 helpers."""
from __future__ import annotations

import argparse
import glob
import json
import os
import sys
from pathlib import Path
from typing import Any

from codex_review.core.artifacts import read_json, read_text, write_json, write_text
from codex_review.core.config import load_config
from codex_review.core.env import read_event_payload
from codex_review.core.errors import CodexReviewError, ValidationError, format_error
from codex_review.core.output import append_step_summary, mask_secret, write_output


def _maybe_json(path: str | None, default: Any = None) -> Any:
    if not path:
        return default
    try:
        return json.loads(Path(path).read_text(encoding="utf-8"))
    except FileNotFoundError:
        raise ValidationError(f"missing JSON artifact: {path}") from None
    except json.JSONDecodeError as exc:
        raise ValidationError(f"malformed JSON artifact {path}: {exc}") from exc


def _json_or_default(path: str | None, default: Any) -> Any:
    if not path:
        return default
    p = Path(path)
    if not p.exists():
        return default
    return _maybe_json(path, default)


def _maybe_text(path: str | None, default: str = "") -> str:
    if not path:
        return default
    p = Path(path)
    if not p.exists():
        return default
    return read_text(path)


def _emit(payload: Any, out: str | None = None, schema_version: str | None = None) -> None:
    if out:
        if isinstance(payload, dict):
            write_json(out, payload, schema_version)
        else:
            write_text(out, str(payload))
    else:
        print(json.dumps(payload, ensure_ascii=False, indent=2, sort_keys=True) if isinstance(payload, (dict, list)) else str(payload))


def _artifact_paths(values: list[str] | None, *, names: tuple[str, ...] = ("*.json",)) -> list[str]:
    """Expand files, directories and globs into deterministic artifact paths."""
    out: list[str] = []
    for value in values or []:
        matches = glob.glob(value)
        candidates = matches or [value]
        for candidate in candidates:
            p = Path(candidate)
            if p.is_dir():
                for pattern in names:
                    out.extend(str(x) for x in sorted(p.rglob(pattern)))
            elif p.exists():
                out.append(str(p))
    seen: set[str] = set()
    deduped: list[str] = []
    for path in sorted(out):
        if path not in seen:
            seen.add(path)
            deduped.append(path)
    return deduped


def _preferred_artifact_paths(values: list[str] | None, *, primary: str, fallback: str) -> list[str]:
    paths = _artifact_paths(values, names=(primary,))
    return paths or _artifact_paths(values, names=(fallback,))


def _safe_path_component(value: Any) -> str:
    text = str(value or "").strip()
    safe = "".join(ch if ch.isalnum() or ch in {"-", "_", "."} else "_" for ch in text)
    return safe.strip("._") or "task"


def _repo_parts_from_context(ctx: dict[str, Any]) -> tuple[str | None, str | None]:
    owner = ctx.get("owner")
    repo = ctx.get("repo")
    repository = ctx.get("repository") or ctx.get("base_repo_full_name")
    if (not owner or not repo) and isinstance(repository, str) and "/" in repository:
        owner, repo = repository.split("/", 1)
    return owner, repo


def _default_inspection_evidence(purpose: str, observation: str) -> list[dict[str, str]]:
    return [{"path": "AGENTS.md", "purpose": purpose, "observation": observation}]


def _add_common(p: argparse.ArgumentParser) -> None:
    p.add_argument("command", nargs="?")
    p.add_argument("--config", default=None)
    p.add_argument("--in", dest="in_path", default=None)
    p.add_argument("--out", default=None)
    p.add_argument("--inventory", default=None)
    p.add_argument("--result", default=None)
    p.add_argument("--pr-context", default=None)
    p.add_argument("--review-context", default=None)
    p.add_argument("--docs-context", default=None)
    p.add_argument("--openspec-context", default=None)
    p.add_argument("--changed-lines", default=None)
    p.add_argument("--axis", default=None)
    p.add_argument("--artifacts", nargs="*", default=None)
    p.add_argument("--token", default=os.environ.get("GITHUB_TOKEN"))
    p.add_argument("--repo-path", default=".")
    p.add_argument("--patch", default=None)
    p.add_argument("--dry-run", action="store_true")
    p.add_argument("--summary", action="store_true")
    p.add_argument("--event", default=None)
    p.add_argument("--loop-state", default=None)
    p.add_argument("--chief-decision", default=None)
    p.add_argument("--design-plan", default=None)
    p.add_argument("--mode", default=None)
    p.add_argument("--stage", default=None)
    p.add_argument("--prompt", default=None)
    p.add_argument("--prompt-out", default=None)
    p.add_argument("--raw-out", default=None)
    p.add_argument("--model-command", default=os.environ.get("CODEX_REVIEW_MODEL_COMMAND"))
    p.add_argument("--work-dir", default=None)
    p.add_argument("--model-cwd", default=os.environ.get("CODEX_REVIEW_MODEL_CWD") or os.environ.get("CODEX_REVIEW_TRUSTED_CHECKOUT"))
    p.add_argument("--validation", default=None)
    p.add_argument("--schema", default=None)
    p.add_argument("--semantic-safety", default=None)
    p.add_argument("--audience", default=None)
    p.add_argument("--broker-url", default=None)
    p.add_argument("--name", default=None)


def _model_or_fallback(args: argparse.Namespace, *, stage: str, expected_schema: str, fallback: dict[str, Any]) -> dict[str, Any]:
    from codex_review.model.adapter import run_model_or_fallback
    return run_model_or_fallback(
        stage=stage,
        prompt_path=args.prompt,
        output_path=args.out,
        expected_schema=expected_schema,
        fallback=fallback,
        model_command=args.model_command,
        cwd=args.model_cwd or args.repo_path,
        target_repo_path=args.repo_path if args.repo_path not in {None, "."} else None,
    )


# Compatibility hooks retained for tests/consumers that import them.
def register_event_commands(parser: argparse.ArgumentParser) -> None: return None
def register_context_commands(parser: argparse.ArgumentParser) -> None: return None
def register_loop_commands(parser: argparse.ArgumentParser) -> None: return None
def register_stage00_commands(parser: argparse.ArgumentParser) -> None: return None
def register_stage01_commands(parser: argparse.ArgumentParser) -> None: return None
def register_stage02_commands(parser: argparse.ArgumentParser) -> None: return None
def register_stage03_commands(parser: argparse.ArgumentParser) -> None: return None
def register_stage04_commands(parser: argparse.ArgumentParser) -> None: return None
def register_stage05_commands(parser: argparse.ArgumentParser) -> None: return None
def register_stage06_commands(parser: argparse.ArgumentParser) -> None: return None
def register_stage07_commands(parser: argparse.ArgumentParser) -> None: return None
def register_stage08_commands(parser: argparse.ArgumentParser) -> None: return None
def register_stage09_commands(parser: argparse.ArgumentParser) -> None: return None


def _handle_event(args: argparse.Namespace) -> tuple[Any, str | None]:
    if args.command in {"resolve-current", "read-payload", None}:
        payload = read_event_payload(args.in_path or args.event)
        pr = payload.get("pull_request") or payload
        repo_obj = payload.get("repository") or (pr.get("base") or {}).get("repo") or {}
        repository = repo_obj.get("full_name") or pr.get("base_repo_full_name") or os.environ.get("GITHUB_REPOSITORY")
        owner = ((repo_obj.get("owner") or {}).get("login") if isinstance(repo_obj.get("owner"), dict) else repo_obj.get("owner"))
        repo_name = repo_obj.get("name")
        if repository and "/" in repository:
            owner = owner or repository.split("/", 1)[0]
            repo_name = repo_name or repository.split("/", 1)[1]
        head = pr.get("head") or {}
        base = pr.get("base") or {}
        out = {
            "schema_version": "event-context.v1",
            "event_name": os.environ.get("GITHUB_EVENT_NAME") or payload.get("event_name"),
            "repository": repository,
            "owner": owner,
            "repo": repo_name,
            "pr_number": payload.get("number") or pr.get("number") or (payload.get("inputs") or {}).get("pr_number") or os.environ.get("CODEX_REVIEW_PR_NUMBER"),
            "head_sha": head.get("sha") or pr.get("head_sha"),
            "base_sha": base.get("sha") or pr.get("base_sha"),
            "head_ref": head.get("ref") or pr.get("head_ref"),
            "base_ref": base.get("ref") or pr.get("base_ref"),
            "head_repo_full_name": (head.get("repo") or {}).get("full_name") if isinstance(head.get("repo"), dict) else None,
            "base_repo_full_name": (base.get("repo") or {}).get("full_name") if isinstance(base.get("repo"), dict) else repository,
            "sender": (payload.get("sender") or {}).get("login") if isinstance(payload.get("sender"), dict) else payload.get("sender"),
        }
        if out.get("head_repo_full_name") and out.get("base_repo_full_name"):
            out["same_repo"] = out["head_repo_full_name"] == out["base_repo_full_name"]
        else:
            out["same_repo"] = None
        return out, "event-context.v1"
    if args.command in {"write-outputs", "github-outputs"}:
        payload = _maybe_json(args.in_path or args.pr_context, {})
        keys = ["same_repo", "head_sha", "head_repo_full_name", "head_ref", "base_sha", "base_repo_full_name", "pr_number"]
        outputs: dict[str, str] = {}
        for key in keys:
            value = payload.get(key)
            if value is None:
                continue
            text = str(value).lower() if isinstance(value, bool) else str(value)
            outputs[key] = text
            write_output(key, text)
        return {"outputs": outputs}, None
    raise ValueError(f"unknown event command: {args.command}")


def _signal_context_truncation(context: dict[str, Any]) -> None:
    """Surface PR-context truncation so dropped coverage is visible, not silent."""
    from codex_review.context.pr import context_truncation_evidence

    evidence = context_truncation_evidence(context)
    if not evidence:
        return
    write_output("context_truncated", "true")
    write_output("context_truncated_patch_count", str(evidence["truncated_patch_count"]))
    append_step_summary(
        "> [!WARNING] PR context exceeded token budget and was truncated: "
        f"diff_truncated={evidence['diff_truncated']}, "
        f"patches_truncated={evidence['patches_truncated']} "
        f"({evidence['truncated_patch_count']} file patches reduced to hunk headers). "
        "Review coverage of the dropped content may be incomplete."
    )
    artifact_root = os.environ.get("CODEX_REVIEW_ARTIFACT_ROOT")
    if artifact_root:
        try:
            from codex_review.loop.events import append_event_log, record_event

            append_event_log(Path(artifact_root) / "events.jsonl", record_event("CONTEXT_TRUNCATED", "context.pr", evidence))
        except Exception:
            pass


def _handle_context(args: argparse.Namespace, config: dict[str, Any]) -> tuple[Any, str | None]:
    cmd = args.command
    if cmd in {"pr", "build-pr"}:
        from codex_review.context.pr import build_pr_context
        from codex_review.github.pull_requests import get_pull_request, list_pull_request_files
        event_payload = read_event_payload(args.event) if args.event else _json_or_default(args.in_path, {})
        event_ctx = _json_or_default(args.pr_context, {})
        owner, repo = _repo_parts_from_context(event_ctx or event_payload)
        pr_number = event_ctx.get("pr_number") or event_payload.get("number") or (event_payload.get("pull_request") or {}).get("number") or (event_payload.get("inputs") or {}).get("pr_number") or os.environ.get("CODEX_REVIEW_PR_NUMBER")
        pr = event_payload.get("pull_request") or {}
        files: list[dict[str, Any]] = []
        if owner and repo and pr_number and args.token:
            try:
                pr = get_pull_request(owner, repo, int(pr_number), args.token)
                files = list_pull_request_files(owner, repo, int(pr_number), args.token)
            except CodexReviewError:
                raise
            except Exception:
                # Local dry-runs and tests can still construct context from the event payload.
                files = []
        if not pr:
            pr = event_payload.get("pull_request") or event_ctx
        diff = "\n".join(str(f.get("patch") or "") for f in files)
        context = build_pr_context(event_payload if isinstance(event_payload, dict) else {}, pr, files, diff, config)
        _signal_context_truncation(context)
        return context, "pr-context.v1"
    if cmd in {"changed-lines", "changed"}:
        from codex_review.context.diff import build_changed_line_map, serialize_changed_line_map
        payload = _json_or_default(args.in_path, {})
        if not payload and args.pr_context:
            payload = _json_or_default(args.pr_context, {})
        changed = payload.get("changed_line_map") if isinstance(payload, dict) and payload.get("changed_line_map") else build_changed_line_map(payload)
        return {"schema_version": "changed-lines.v1", "changed_line_map": serialize_changed_line_map(changed)}, None
    if cmd in {"docs", "docs-context"}:
        from codex_review.context.docs import find_repository_docs, read_docs_with_budget, render_docs_context
        docs = read_docs_with_budget(find_repository_docs(args.repo_path), int(config.get("docs_context_budget", 20000)))
        return render_docs_context(docs), None
    if cmd in {"openspec", "openspec-context"}:
        from codex_review.context.openspec import collect_openspec_context
        pr = _json_or_default(args.pr_context or args.in_path, {})
        return collect_openspec_context(pr, args.repo_path, args.token), "openspec-context.v1"
    if cmd in {"openspec-markdown", "render-openspec"}:
        from codex_review.context.openspec import render_openspec_context_markdown, sections_for_stage
        budget = int((config.get("context", {}) or {}).get("openspec_tokens", 0)) or None
        return render_openspec_context_markdown(
            _maybe_json(args.in_path or args.openspec_context, {}),
            sections=sections_for_stage(args.stage),
            budget_tokens=budget,
        ), None
    if cmd in {"openspec-outputs", "openspec-github-outputs"}:
        payload = _maybe_json(args.in_path or args.openspec_context, {})
        outputs = {
            "openspec_present": str(bool(payload.get("present"))).lower(),
            "openspec_status": str(payload.get("status") or ""),
            "openspec_decision": str(payload.get("decision") or ""),
        }
        for key, value in outputs.items():
            write_output(key, value)
        return {"outputs": outputs}, None
    if cmd in {"review", "review-context"}:
        from codex_review.context.threads import build_review_context_markdown
        pr = _json_or_default(args.pr_context, {})
        threads_payload = _json_or_default(args.in_path, [])
        threads = threads_payload.get("threads") if isinstance(threads_payload, dict) else threads_payload
        return build_review_context_markdown(pr, threads or [], [], []), None
    raise ValueError(f"unknown context command: {cmd}")


def _handle_loop(args: argparse.Namespace) -> tuple[Any, str | None]:
    from codex_review.loop.router import route_after_stage00, route_after_stage02, route_after_stage04, route_after_stage07, write_route_outputs
    cmd = args.command
    payload = _maybe_json(args.in_path, {})
    if cmd in {"route-after-stage00", "route"}:
        route = route_after_stage00(payload)
        write_route_outputs(route)
        return route, None
    if cmd == "route-after-stage02":
        route = route_after_stage02(payload)
        write_route_outputs(route)
        return route, None
    if cmd == "route-after-stage04":
        route = route_after_stage04(payload)
        write_route_outputs(route)
        return route, None
    if cmd == "route-after-stage07":
        route = route_after_stage07(payload)
        write_route_outputs(route)
        return route, None
    if cmd == "summary":
        from codex_review.loop.events import render_event_summary
        return render_event_summary(payload if isinstance(payload, list) else payload.get("events", [])), None
    if cmd in {"read-state", "read-loop-state"}:
        from codex_review.loop.state import read_loop_state_from_comments
        empty = {"schema_version": "loop-state.v1", "recent_pushes": [], "round_count": 0}
        pr = _json_or_default(args.pr_context, {})
        owner, repo = _repo_parts_from_context(pr)
        pr_number = pr.get("pr_number")
        if not (owner and repo and pr_number and args.token):
            return empty, None
        # Tolerant: a comment-read hiccup must not break bootstrap; degrade to no history.
        try:
            from codex_review.github.comments import list_issue_comments
            comments = list_issue_comments(owner, repo, int(pr_number), args.token)
            state = read_loop_state_from_comments(comments)
        except Exception:
            state = None
        return ({**empty, **state} if state else empty), None
    raise ValueError(f"unknown loop command: {cmd}")


def _handle_stage00(args: argparse.Namespace, config: dict[str, Any]) -> tuple[Any, str | None]:
    cmd = args.command
    if cmd == "collect":
        from codex_review.github.review_threads import collect_review_threads
        from codex_review.stages.stage00_resolve_gate.collect import collect_thread_inventory
        pr = _maybe_json(args.pr_context, {})
        raw = _json_or_default(args.in_path, None)
        if raw is None:
            owner, repo = _repo_parts_from_context(pr)
            pr_number = pr.get("pr_number")
            threads = collect_review_threads(owner, repo, int(pr_number), args.token) if owner and repo and pr_number and args.token else []
        else:
            threads = raw.get("threads") or raw.get("review_threads") if isinstance(raw, dict) else raw
        inv = collect_thread_inventory(pr, threads or [], [], config)
        return inv, "stage00-thread-inventory.v1"
    if cmd in {"collect-resolved", "resolved-memory"}:
        from codex_review.github.review_threads import collect_review_threads
        from codex_review.stages.stage00_resolve_gate.collect import build_resolved_memory
        pr = _maybe_json(args.pr_context, {})
        raw = _json_or_default(args.in_path, None)
        if raw is None:
            owner, repo = _repo_parts_from_context(pr)
            pr_number = pr.get("pr_number")
            threads = collect_review_threads(owner, repo, int(pr_number), args.token) if owner and repo and pr_number and args.token else []
        else:
            threads = raw.get("threads") or raw.get("review_threads") if isinstance(raw, dict) else raw
        return build_resolved_memory(pr, threads or [], config), "stage00-resolved-memory.v1"
    if cmd in {"default-result", "noop-result", "model-result"}:
        inv = _maybe_json(args.inventory or args.in_path, {})
        decisions = []
        for item in inv.get("items", []):
            state = "needs_human" if item.get("forced_needs_human") else "current_head_keep_open"
            decisions.append({"thread_id": item.get("thread_id"), "state": state, "reason": item.get("forced_reason") or "safe default keeps existing thread open"})
        fallback = {"schema_version": "stage00-lifecycle-result.v1", "decisions": decisions, "defaulted": True}
        if cmd == "model-result":
            return _model_or_fallback(args, stage="stage00", expected_schema="stage00-lifecycle-result.v1", fallback=fallback), "stage00-lifecycle-result.v1"
        return fallback, "stage00-lifecycle-result.v1"
    if cmd in {"build-prompt", "prompt"}:
        from codex_review.stages.stage00_resolve_gate.prompt import build_lifecycle_prompt
        inv = _maybe_json(args.inventory or args.in_path, {})
        prompt = build_lifecycle_prompt(inv, _maybe_text(args.review_context), _maybe_text(args.docs_context), config)
        return prompt, None
    if cmd in {"validate", "validate-result"}:
        from codex_review.stages.stage00_resolve_gate.validate import validate_lifecycle_result
        return validate_lifecycle_result(_maybe_json(args.result or args.in_path, {}), _maybe_json(args.inventory, {})), "stage00-lifecycle-result.v1"
    if cmd == "apply":
        from codex_review.stages.stage00_resolve_gate.apply import apply_lifecycle_result
        return apply_lifecycle_result(_maybe_json(args.result or args.in_path, {}), _maybe_json(args.pr_context, {}), args.token, config, dry_run=args.dry_run), None
    if cmd == "route":
        from codex_review.stages.stage00_resolve_gate.route import build_gate_result, emit_gate_outputs
        apply_report = _maybe_json(args.artifacts[0], {}) if args.artifacts else {}
        gate = build_gate_result(_maybe_json(args.inventory, {}), _maybe_json(args.result or args.in_path, {}), apply_report)
        emit_gate_outputs(gate)
        return gate, "stage00-gate-result.v1"
    if cmd == "render":
        from codex_review.stages.stage00_resolve_gate.render import render_stage00_step_summary
        return render_stage00_step_summary(_maybe_json(args.in_path, {})), None
    raise ValueError(f"unknown stage00 command: {cmd}")


def _handle_stage01(args: argparse.Namespace, config: dict[str, Any]) -> tuple[Any, str | None]:
    cmd = args.command
    if cmd == "axes":
        from codex_review.stages.stage01_review.axes import review_axes
        return {"axes": review_axes(config)}, None
    if cmd in {"default-result", "noop-result", "model-result"}:
        axis = args.axis or "correctness"
        fallback = {
            "schema_version": "stage01-axis-findings.v1",
            "axis": axis,
            "findings": [],
            "inspection_evidence": _default_inspection_evidence("deterministic fallback", "No model result was available for this axis."),
            "defaulted": True,
        }
        if cmd == "model-result":
            return _model_or_fallback(args, stage=f"stage01_{axis}", expected_schema="stage01-axis-findings.v1", fallback=fallback), "stage01-axis-findings.v1"
        return fallback, "stage01-axis-findings.v1"
    if cmd in {"build-review-prompt", "prompt"}:
        from codex_review.stages.stage01_review.prompt import build_axis_prompt
        return build_axis_prompt(args.axis or "correctness", _maybe_json(args.pr_context, {}), _maybe_text(args.review_context), _maybe_text(args.docs_context), config), None
    if cmd in {"validate-axis", "validate"}:
        from codex_review.stages.stage01_review.validate import validate_axis_findings
        payload = _maybe_json(args.in_path, {})
        changed_payload = _json_or_default(args.changed_lines, {})
        changed = changed_payload.get("changed_line_map", changed_payload) if isinstance(changed_payload, dict) else {}
        return validate_axis_findings(args.axis or payload.get("axis"), payload, _maybe_json(args.pr_context, {}), changed, config, args.repo_path), "stage01-axis-findings.v1"
    if cmd == "combine":
        from codex_review.stages.stage01_review.combine import combine_axis_findings
        paths = _preferred_artifact_paths(args.artifacts, primary="findings.validated.json", fallback="findings.json")
        return combine_axis_findings(paths, config), "stage01-combined-findings.v1"
    if cmd == "render":
        from codex_review.stages.stage01_review.render import render_combined_summary
        return render_combined_summary(_maybe_json(args.in_path, {})), None
    raise ValueError(f"unknown stage01 command: {cmd}")


def _handle_stage02(args: argparse.Namespace, config: dict[str, Any]) -> tuple[Any, str | None]:
    cmd = args.command
    if cmd in {"default-result", "noop-result", "model-result"}:
        combined = _maybe_json(args.inventory or args.in_path, {"findings": []})
        decisions = [{"finding_id": f.get("finding_id") or f.get("id"), "action": "publish_only", "reason": "safe deterministic default"} for f in combined.get("findings", [])]
        fallback = {
            "schema_version": "stage02-techlead-decision.v1",
            "decisions": decisions,
            "needs_design": False,
            "status": "ready" if decisions else "lgtm",
            "inspection_evidence": _default_inspection_evidence("deterministic fallback", "No techlead model result was available."),
            "defaulted": True,
        }
        if cmd == "model-result":
            return _model_or_fallback(args, stage="stage02", expected_schema="stage02-techlead-decision.v1", fallback=fallback), "stage02-techlead-decision.v1"
        return fallback, "stage02-techlead-decision.v1"
    if cmd in {"filter-resolved", "filter-against-resolved"}:
        from codex_review.stages.stage02_techlead.filter_resolved import filter_findings_against_resolved
        combined = _maybe_json(args.in_path, {"findings": []})
        resolved_memory = _json_or_default(args.inventory, {})
        pr = _json_or_default(args.pr_context, {})
        changed_line_map = pr.get("changed_line_map") or {}
        filtered, _ = filter_findings_against_resolved(combined, resolved_memory, changed_line_map, config)
        return filtered, "stage01-combined-findings.v1"
    if cmd in {"build-techlead-prompt", "prompt"}:
        from codex_review.stages.stage02_techlead.prompt import build_techlead_prompt
        return build_techlead_prompt(_maybe_json(args.in_path, {}), _maybe_json(args.pr_context, {}), _maybe_text(args.review_context), _maybe_text(args.docs_context), config), None
    if cmd == "validate":
        from codex_review.stages.stage02_techlead.validate import validate_techlead_decision
        combined = _maybe_json(args.artifacts[0], {}) if args.artifacts else _maybe_json(args.inventory, {})
        return validate_techlead_decision(_maybe_json(args.in_path, {}), combined, config, args.repo_path), "stage02-techlead-decision.v1"
    if cmd == "classify":
        from codex_review.stages.stage02_techlead.classify import build_review_publication
        combined = _maybe_json(args.artifacts[0], {}) if args.artifacts else _maybe_json(args.inventory, {})
        return build_review_publication(_maybe_json(args.in_path, {}), combined, config), "stage02-review-publication.v1"
    if cmd in {"write-deferred-outputs", "deferred-outputs"}:
        payload = _maybe_json(args.in_path, {})
        count = len(payload.get("deferred_items") or [])
        write_output("has_deferred_issue_items", "true" if count else "false")
        write_output("deferred_issue_count", str(count))
        return {"has_deferred_issue_items": bool(count), "deferred_issue_count": count}, None
    if cmd == "publish":
        from codex_review.stages.stage02_techlead.publish import publish_review
        changed_payload = _json_or_default(args.changed_lines, {})
        changed = changed_payload.get("changed_line_map", changed_payload) if isinstance(changed_payload, dict) else {}
        token = None if args.dry_run else args.token
        return publish_review(_maybe_json(args.in_path, {}), _maybe_json(args.pr_context, {}), changed, token, config, dry_run=args.dry_run), None
    if cmd == "render":
        from codex_review.stages.stage02_techlead.render import render_review_body
        return render_review_body(_maybe_json(args.in_path, {})), None
    raise ValueError(f"unknown stage02 command: {cmd}")


def _handle_stage03(args: argparse.Namespace, config: dict[str, Any]) -> tuple[Any, str | None]:
    cmd = args.command
    if cmd == "context":
        from codex_review.stages.stage03_design.context import build_design_context
        return build_design_context(_maybe_json(args.pr_context, {}), _maybe_json(args.in_path, {}), _maybe_text(args.review_context), _maybe_text(args.docs_context), None, _json_or_default(args.openspec_context, {})), "stage03-design-context.v1"
    if cmd in {"build-inventory-prompt", "inventory-prompt"}:
        from codex_review.stages.stage03_design.normalize import build_normalize_prompt
        return build_normalize_prompt(_maybe_json(args.in_path or args.inventory, {})), None
    if cmd in {"default-inventory", "default-result", "model-inventory"}:
        ctx = _maybe_json(args.in_path, {})
        items = []
        for finding in ctx.get("findings", []):
            items.append({"finding_id": finding.get("finding_id"), "invariant": finding.get("root_cause_key") or finding.get("title") or finding.get("finding_id"), "summary": finding.get("summary", "")})
        fallback = {
            "schema_version": "stage03-design-inventory.v1",
            "items": items,
            "item_count": len(items),
            "inspection_evidence": _default_inspection_evidence("deterministic fallback", "No design inventory model result was available."),
            "defaulted": True,
        }
        if cmd == "model-inventory":
            if not args.prompt:
                from codex_review.model.adapter import write_prompt_if_needed
                from codex_review.stages.stage03_design.normalize import build_normalize_prompt
                args.prompt = str(write_prompt_if_needed(build_normalize_prompt(ctx), args.out))
            return _model_or_fallback(args, stage="stage03_inventory", expected_schema="stage03-design-inventory.v1", fallback=fallback), "stage03-design-inventory.v1"
        return fallback, "stage03-design-inventory.v1"
    if cmd == "normalize":
        from codex_review.stages.stage03_design.normalize import validate_design_inventory
        ctx = _maybe_json(args.pr_context or args.inventory, {})
        tech = ctx.get("techlead_decision", ctx)
        return validate_design_inventory(_maybe_json(args.in_path, {}), tech, args.repo_path), "stage03-design-inventory.v1"
    if cmd in {"build-clusters-prompt", "clusters-prompt"}:
        from codex_review.stages.stage03_design.cluster import build_cluster_prompt
        return build_cluster_prompt(_maybe_json(args.inventory or args.in_path, {}), _maybe_json(args.pr_context, {})), None
    if cmd in {"default-clusters", "model-clusters"}:
        inv = _maybe_json(args.inventory or args.in_path, {})
        clusters = []
        for idx, item in enumerate(inv.get("items", []), 1):
            clusters.append({"cluster_id": f"cluster-{idx}", "finding_ids": [item.get("finding_id")], "summary": item.get("summary", "")})
        fallback = {
            "schema_version": "stage03-design-clusters.v1",
            "clusters": clusters,
            "cluster_count": len(clusters),
            "inspection_evidence": _default_inspection_evidence("deterministic fallback", "No design cluster model result was available."),
            "defaulted": True,
        }
        if cmd == "model-clusters":
            if not args.prompt:
                from codex_review.model.adapter import write_prompt_if_needed
                from codex_review.stages.stage03_design.cluster import build_cluster_prompt
                args.prompt = str(write_prompt_if_needed(build_cluster_prompt(inv, _maybe_json(args.pr_context, {})), args.out))
            return _model_or_fallback(args, stage="stage03_clusters", expected_schema="stage03-design-clusters.v1", fallback=fallback), "stage03-design-clusters.v1"
        return fallback, "stage03-design-clusters.v1"
    if cmd == "cluster":
        from codex_review.stages.stage03_design.cluster import validate_design_clusters
        return validate_design_clusters(_maybe_json(args.in_path, {}), _maybe_json(args.inventory, {}), args.repo_path), "stage03-design-clusters.v1"
    if cmd == "batch":
        from codex_review.stages.stage03_design.batch import make_cluster_batches
        return {"batches": make_cluster_batches(_maybe_json(args.in_path, {}), config)}, None
    if cmd in {"prepare-analysis-matrix", "prepare-analysis"}:
        from codex_review.stages.stage03_design.analyze import build_cluster_analysis_prompt
        from codex_review.stages.stage03_design.batch import make_cluster_batches
        clusters = _maybe_json(args.in_path or args.inventory, {})
        design_context = _maybe_json(args.pr_context, {})
        batches = make_cluster_batches(clusters, config)
        base_dir = Path(args.work_dir) if args.work_dir else Path("codex-review-artifacts/stage03/batches")
        base_dir.mkdir(parents=True, exist_ok=True)
        include = []
        for batch in batches:
            if not batch.get("clusters"):
                continue
            batch_index = int(batch.get("batch_index", len(include)))
            batch_path = str(batch_index)
            batch_dir = base_dir / batch_path
            batch_dir.mkdir(parents=True, exist_ok=True)
            batch_file = batch_dir / "batch.json"
            prompt_file = batch_dir / "analysis.prompt.md"
            output_file = batch_dir / "analysis.raw.json"
            validated_file = batch_dir / "analysis.validated.json"
            write_json(batch_file, batch, "stage03-cluster-batch.v1")
            write_text(prompt_file, build_cluster_analysis_prompt(batch, design_context))
            include.append(
                {
                    "batch_index": batch_index,
                    "batch_path": batch_path,
                    "batch_file": batch_file.as_posix(),
                    "prompt_file": prompt_file.as_posix(),
                    "output_file": output_file.as_posix(),
                    "validated_file": validated_file.as_posix(),
                    "working_directory": args.repo_path,
                }
            )
        matrix = {"include": include}
        if os.environ.get("GITHUB_OUTPUT"):
            write_output("has_analysis_batches", "true" if include else "false")
            write_output("analysis_matrix", json.dumps(matrix, sort_keys=True, separators=(",", ":")))
        return matrix, None
    if cmd in {"collect-analyses", "collect-analysis"}:
        from codex_review.stages.stage03_design.analyze import combine_cluster_analyses
        paths = _preferred_artifact_paths(args.artifacts, primary="analysis.validated.json", fallback="*.json")
        analyses = combine_cluster_analyses(paths)
        return {"schema_version": "stage03-cluster-analysis.v1", "analyses": analyses, "analysis_count": len(analyses)}, "stage03-cluster-analysis.v1"
    if cmd in {"build-analysis-prompt", "analysis-prompt"}:
        from codex_review.stages.stage03_design.analyze import build_cluster_analysis_prompt
        return build_cluster_analysis_prompt(_maybe_json(args.inventory or args.in_path, {}), _maybe_json(args.pr_context, {})), None
    if cmd in {"default-analysis", "model-analysis"}:
        clusters = _maybe_json(args.inventory or args.in_path, {})
        analyses = [{"cluster_id": c.get("cluster_id"), "status": "needs_human", "recommendation": "model analysis not provided"} for c in clusters.get("clusters", [])]
        fallback = {
            "schema_version": "stage03-cluster-analysis.v1",
            "analyses": analyses,
            "inspection_evidence": _default_inspection_evidence("deterministic fallback", "No cluster analysis model result was available."),
            "defaulted": True,
        }
        if cmd == "model-analysis":
            if not args.prompt:
                from codex_review.model.adapter import write_prompt_if_needed
                from codex_review.stages.stage03_design.analyze import build_cluster_analysis_prompt
                args.prompt = str(write_prompt_if_needed(build_cluster_analysis_prompt(clusters, _maybe_json(args.pr_context, {})), args.out))
            return _model_or_fallback(args, stage="stage03_analysis", expected_schema="stage03-cluster-analysis.v1", fallback=fallback), "stage03-cluster-analysis.v1"
        return fallback, "stage03-cluster-analysis.v1"
    if cmd == "analyze":
        from codex_review.stages.stage03_design.analyze import validate_cluster_analysis
        batch = _maybe_json(args.artifacts[0], {}) if args.artifacts else _maybe_json(args.inventory, {})
        return validate_cluster_analysis(_maybe_json(args.in_path, {}), batch, args.repo_path), "stage03-cluster-analysis.v1"
    if cmd in {"default-plan", "model-plan"}:
        from codex_review.stages.stage03_design.coordinate import validate_design_plan
        ctx = _maybe_json(args.pr_context or args.inventory, {})
        findings = ctx.get("findings", [])
        plan = {
            "schema_version": "stage03-design-plan.v1",
            "edit_sequence": [],
            "tests": [],
            "inspection_evidence": _default_inspection_evidence("deterministic fallback", "No design plan model result was available."),
            "defaulted": True,
        }
        if findings:
            plan["openspec_backed"] = bool(ctx.get("openspec_backed"))
            plan["edit_sequence"] = [
                {
                    "task_id": f"openspec-fallback-{idx}" if ctx.get("openspec_backed") else f"manual-design-{idx}",
                    "finding_ids": [finding.get("finding_id") or finding.get("id")],
                    "summary": finding.get("summary") or "Implement the OpenSpec-backed finding conservatively.",
                    "allowed_files": finding.get("files") or ([finding.get("file")] if finding.get("file") else []),
                    "acceptance_criteria": ["OpenSpec tasks and affected tests pass"] if ctx.get("openspec_backed") else [],
                }
                for idx, finding in enumerate(findings, 1)
            ]
            plan["tests"] = ["cargo fmt --all --check", "cargo test --workspace --all-features"] if ctx.get("openspec_backed") else ["Human design review required before automated tests are selected"]
            if not ctx.get("openspec_backed"):
                plan["requires_human_review"] = True
            fallback = validate_design_plan(plan, ctx, config)
        else:
            fallback = validate_design_plan(plan, ctx, config)
        if cmd == "model-plan":
            if not args.prompt:
                from codex_review.model.adapter import write_prompt_if_needed
                from codex_review.stages.stage03_design.coordinate import build_coordinate_prompt
                args.prompt = str(write_prompt_if_needed(build_coordinate_prompt(ctx, _maybe_json(args.inventory, {}), []), args.out))
            return _model_or_fallback(args, stage="stage03_plan", expected_schema="stage03-design-plan.v1", fallback=fallback), "stage03-design-plan.v1"
        return fallback, "stage03-design-plan.v1"
    if cmd in {"build-plan-prompt", "plan-prompt"}:
        from codex_review.stages.stage03_design.coordinate import build_coordinate_prompt
        analyses_payload = _json_or_default(args.result, {})
        analyses = analyses_payload.get("analyses", analyses_payload if isinstance(analyses_payload, list) else [])
        return build_coordinate_prompt(_maybe_json(args.pr_context, {}), _maybe_json(args.inventory, {}), analyses), None
    if cmd in {"coordinate", "validate-plan"}:
        from codex_review.stages.stage03_design.coordinate import validate_design_plan
        return validate_design_plan(_maybe_json(args.in_path, {}), _maybe_json(args.pr_context, {}), config, args.repo_path), "stage03-design-plan.v1"
    if cmd == "render":
        from codex_review.stages.stage03_design.render import render_design_plan_markdown
        return render_design_plan_markdown(_maybe_json(args.in_path, {})), None
    raise ValueError(f"unknown stage03 command: {cmd}")


def _handle_stage04(args: argparse.Namespace, config: dict[str, Any]) -> tuple[Any, str | None]:
    cmd = args.command
    if cmd in {"default-result", "noop-result", "model-result"}:
        plan = _maybe_json(args.design_plan or args.in_path, {})
        status = "needs_human" if plan.get("requires_human_review") else ("no_fix_needed" if not plan.get("edit_sequence") else "needs_human")
        fallback = {
            "schema_version": "stage04-design-chief-decision.v1",
            "status": status,
            "reason": "safe deterministic default",
            "inspection_evidence": _default_inspection_evidence("deterministic fallback", "No design chief model result was available."),
            "defaulted": True,
        }
        if cmd == "model-result":
            return _model_or_fallback(args, stage="stage04", expected_schema="stage04-design-chief-decision.v1", fallback=fallback), "stage04-design-chief-decision.v1"
        return fallback, "stage04-design-chief-decision.v1"
    if cmd in {"build-chief-prompt", "prompt"}:
        from codex_review.stages.stage04_design_chief.prompt import build_design_chief_prompt
        return build_design_chief_prompt(_maybe_json(args.in_path, {}), _maybe_json(args.inventory, {}), _maybe_json(args.pr_context, {}), config), None
    if cmd == "validate":
        from codex_review.stages.stage04_design_chief.validate import validate_chief_decision
        return validate_chief_decision(_maybe_json(args.in_path, {}), _maybe_json(args.inventory or args.design_plan, {}), config, args.repo_path), "stage04-design-chief-decision.v1"
    if cmd == "route":
        from codex_review.stages.stage04_design_chief.route import route_after_design_chief, write_chief_route_outputs
        route = route_after_design_chief(_maybe_json(args.in_path, {}))
        write_chief_route_outputs(route)
        return route, None
    if cmd == "publish":
        from codex_review.stages.stage04_design_chief.publish import publish_design_summary
        token = None if args.dry_run else args.token
        return publish_design_summary(_maybe_json(args.inventory or args.design_plan, {}), _maybe_json(args.in_path, {}), token, _maybe_json(args.pr_context, {}), dry_run=args.dry_run), None
    if cmd == "render":
        from codex_review.stages.stage04_design_chief.render import render_chief_decision_markdown
        return render_chief_decision_markdown(_maybe_json(args.in_path, {})), None
    raise ValueError(f"unknown stage04 command: {cmd}")


def _handle_stage05(args: argparse.Namespace, config: dict[str, Any]) -> tuple[Any, str | None]:
    cmd = args.command
    if cmd in {"plan-tasks", "plan"}:
        from codex_review.stages.stage05_fix_dispatch.plan import plan_fix_tasks
        return plan_fix_tasks(_maybe_json(args.in_path, {}), _maybe_json(args.inventory or args.chief_decision, {}), config), "stage05-fix-task-manifest.v1"
    if cmd in {"build-agent-prompt", "prompt"}:
        from codex_review.stages.stage05_fix_dispatch.prompt import build_fix_agent_prompt
        task = _maybe_json(args.in_path, {})
        return build_fix_agent_prompt(task, _maybe_json(args.inventory, {}), _maybe_json(args.result, {}), _maybe_text(args.docs_context), config), None
    if cmd in {"prepare-agents", "prepare-agent-matrix"}:
        from codex_review.stages.stage05_fix_dispatch.prompt import build_fix_agent_prompt
        manifest = _maybe_json(args.inventory or args.in_path, {})
        design_plan = _maybe_json(args.design_plan, {})
        chief = _maybe_json(args.chief_decision or args.result, {})
        docs = _maybe_text(args.docs_context)
        base_dir = Path(args.work_dir) if args.work_dir else Path("codex-review-artifacts/stage05/agents")
        base_dir.mkdir(parents=True, exist_ok=True)
        include = []
        for task in manifest.get("tasks", []):
            task_id = str(task.get("task_id") or f"task-{len(include) + 1}")
            task_path = _safe_path_component(task_id)
            task_dir = base_dir / task_path
            task_dir.mkdir(parents=True, exist_ok=True)
            task_file = task_dir / "task.json"
            prompt_file = task_dir / "prompt.md"
            output_file = task_dir / "result.json"
            validated_file = task_dir / "result.validated.json"
            write_json(task_file, task, None)
            write_text(prompt_file, build_fix_agent_prompt(task, design_plan, chief, docs, config))
            include.append(
                {
                    "task_id": task_id,
                    "task_path": task_path,
                    "task_file": task_file.as_posix(),
                    "prompt_file": prompt_file.as_posix(),
                    "output_file": output_file.as_posix(),
                    "validated_file": validated_file.as_posix(),
                    "working_directory": args.repo_path,
                }
            )
        matrix = {"include": include}
        if os.environ.get("GITHUB_OUTPUT"):
            write_output("has_agent_tasks", "true" if include else "false")
            write_output("agent_matrix", json.dumps(matrix, sort_keys=True, separators=(",", ":")))
        return matrix, None
    if cmd in {"default-agent-result", "noop-result"}:
        task = _maybe_json(args.inventory or args.in_path, {})
        return {"schema_version": "stage05-fix-agent-result.v1", "task_id": task.get("task_id"), "status": "no_safe_fix", "reason": "model fix result was not provided", "defaulted": True}, "stage05-fix-agent-result.v1"
    if cmd in {"run-agents", "model-agents"}:
        from codex_review.model.adapter import run_model_or_fallback
        from codex_review.stages.stage05_fix_dispatch.collect import build_fix_collection_result
        from codex_review.stages.stage05_fix_dispatch.prompt import build_fix_agent_prompt
        from codex_review.stages.stage05_fix_dispatch.validate_agent_result import validate_fix_agent_result
        manifest = _maybe_json(args.inventory or args.in_path, {})
        design_plan = _maybe_json(args.design_plan, {})
        chief = _maybe_json(args.chief_decision or args.result, {})
        docs = _maybe_text(args.docs_context)
        base_dir = Path(args.work_dir) if args.work_dir else (Path(args.out).parent / "agents" if args.out else Path("codex-review-artifacts/stage05/agents"))
        base_dir.mkdir(parents=True, exist_ok=True)
        results=[]
        for task in manifest.get("tasks", []):
            task_id = str(task.get("task_id"))
            task_dir = base_dir / task_id
            task_dir.mkdir(parents=True, exist_ok=True)
            prompt = build_fix_agent_prompt(task, design_plan, chief, docs, config)
            prompt_path = task_dir / "prompt.md"
            write_text(prompt_path, prompt)
            out_path = task_dir / "result.json"
            fallback = {"schema_version":"stage05-fix-agent-result.v1", "task_id": task_id, "status":"no_safe_fix", "reason":"model fix result was not provided", "defaulted": True}
            raw = run_model_or_fallback(stage=f"stage05_{task_id}", prompt_path=prompt_path, output_path=out_path, expected_schema="stage05-fix-agent-result.v1", fallback=fallback, model_command=args.model_command, cwd=args.model_cwd, target_repo_path=args.repo_path)
            validated = validate_fix_agent_result(raw, task, config.get("autofix", {}), args.repo_path)
            write_json(task_dir / "result.validated.json", validated, "stage05-fix-agent-result.v1")
            results.append(validated)
        return build_fix_collection_result(manifest, results), "stage05-fix-collection-result.v1"
    if cmd in {"validate-agent-result", "validate"}:
        from codex_review.stages.stage05_fix_dispatch.validate_agent_result import validate_fix_agent_result
        task = _maybe_json(args.inventory, {})
        return validate_fix_agent_result(_maybe_json(args.in_path, {}), task, config.get("autofix", {}), args.repo_path), "stage05-fix-agent-result.v1"
    if cmd == "collect":
        from codex_review.stages.stage05_fix_dispatch.collect import collect_agent_results
        paths = _artifact_paths(args.artifacts, names=("*.validated.json", "*.json"))
        return collect_agent_results(_maybe_json(args.inventory, {}), paths), "stage05-fix-collection-result.v1"
    if cmd == "render":
        from codex_review.stages.stage05_fix_dispatch.render import render_agent_result_summary
        return render_agent_result_summary(_maybe_json(args.in_path, {})), None
    raise ValueError(f"unknown stage05 command: {cmd}")


def _handle_stage06(args: argparse.Namespace, config: dict[str, Any]) -> tuple[Any, str | None]:
    cmd = args.command
    if cmd == "premerge":
        from codex_review.stages.stage06_fix_merge.premerge import run_premerge_check
        return run_premerge_check(_maybe_json(args.in_path, {}), args.repo_path), "stage06-premerge-report.v1"
    if cmd == "merge":
        from codex_review.stages.stage06_fix_merge.premerge import create_merged_fix_from_premerge
        return create_merged_fix_from_premerge(_maybe_json(args.inventory, {}), _maybe_json(args.in_path, {}), _maybe_json(args.pr_context, {}), args.out), "stage06-merged-fix.v1"
    if cmd in {"model-merged-fix", "model-merge"}:
        from codex_review.model.adapter import run_model_or_fallback, write_prompt_if_needed
        from codex_review.stages.stage06_fix_merge.premerge import create_merged_fix_from_premerge
        from codex_review.stages.stage06_fix_merge.prompt import build_fix_merge_prompt
        pre = _maybe_json(args.inventory, {})
        collection = _maybe_json(args.in_path, {})
        pr = _maybe_json(args.pr_context, {})
        if pre.get("clean") or not collection.get("results"):
            return create_merged_fix_from_premerge(pre, collection, pr, None), "stage06-merged-fix.v1"
        fallback = {"schema_version":"stage06-merged-fix.v1", "status":"blocked", "patch":"", "expected_head_sha": pr.get("head_sha"), "premerge_clean": False, "conflicts": pre.get("results", []), "defaulted": True}
        if not args.prompt:
            args.prompt = str(write_prompt_if_needed(build_fix_merge_prompt(collection, pre, pr, {}, _maybe_text(args.docs_context)), args.out))
        return run_model_or_fallback(stage="stage06_merge", prompt_path=args.prompt, output_path=args.out, expected_schema="stage06-merged-fix.v1", fallback=fallback, model_command=args.model_command, cwd=args.model_cwd, target_repo_path=args.repo_path), "stage06-merged-fix.v1"
    if cmd in {"build-merge-prompt", "prompt"}:
        from codex_review.stages.stage06_fix_merge.prompt import build_fix_merge_prompt
        return build_fix_merge_prompt(_maybe_json(args.in_path, {}), _maybe_json(args.inventory, {}), _maybe_json(args.result, {}), {}, _maybe_text(args.docs_context)), None
    if cmd in {"prepare-merge-model", "prepare-model-merge"}:
        from codex_review.stages.stage06_fix_merge.premerge import create_merged_fix_from_premerge
        from codex_review.stages.stage06_fix_merge.prompt import build_fix_merge_prompt
        pre = _maybe_json(args.inventory, {})
        collection = _maybe_json(args.in_path, {})
        pr = _maybe_json(args.pr_context, {})
        raw_out = args.raw_out or args.result
        prompt_out = args.prompt_out or args.prompt
        if pre.get("clean") or not collection.get("results"):
            merged = create_merged_fix_from_premerge(pre, collection, pr, raw_out)
            if raw_out and not Path(raw_out).exists():
                write_json(raw_out, merged, "stage06-merged-fix.v1")
            route = {"needs_model": False, "raw_output": raw_out, "status": merged.get("status")}
            if os.environ.get("GITHUB_OUTPUT"):
                write_output("needs_model", "false")
            return route, None
        if not prompt_out:
            raise ValidationError("prepare-merge-model requires --prompt-out when model merge is needed")
        write_text(prompt_out, build_fix_merge_prompt(pre, collection, {}, {}, {"pr_context": pr, "docs_context": _maybe_text(args.docs_context)}))
        route = {"needs_model": True, "prompt": prompt_out, "raw_output": raw_out}
        if os.environ.get("GITHUB_OUTPUT"):
            write_output("needs_model", "true")
        return route, None
    if cmd == "default-merged-fix":
        pre = _maybe_json(args.inventory or args.in_path, {})
        return {"schema_version": "stage06-merged-fix.v1", "status": "no_fix", "patch": "", "premerge_clean": pre.get("clean", False), "defaulted": True}, "stage06-merged-fix.v1"
    if cmd == "validate":
        from codex_review.stages.stage06_fix_merge.validate import validate_merged_fix
        from codex_review.patches.fix_edits import ensure_patch_from_edits
        pre = _maybe_json(args.inventory, {})
        chief = _maybe_json(args.result or args.chief_decision, {})
        merged = ensure_patch_from_edits(_maybe_json(args.in_path, {}), args.repo_path)
        return validate_merged_fix(merged, pre, chief, config.get("autofix", {}), args.repo_path), "stage06-merged-fix.v1"
    if cmd in {"build-semantic-safety-prompt", "semantic-safety-prompt"}:
        from codex_review.stages.stage06_fix_merge.semantic_safety import build_semantic_patch_safety_prompt
        prompt = build_semantic_patch_safety_prompt(
            _maybe_json(args.in_path, {}),
            _maybe_json(args.pr_context, {}),
            _maybe_text(args.docs_context, ""),
            repo_path=args.repo_path,
            token_budget=int((config.get("context", {}) or {}).get("model_token_budget", 0)) or None,
        )
        return prompt, None
    if cmd in {"validate-semantic-safety", "semantic-safety-validate"}:
        from codex_review.stages.stage06_fix_merge.semantic_safety import validate_semantic_patch_safety_result
        return validate_semantic_patch_safety_result(_maybe_json(args.in_path, {}), _maybe_json(args.inventory, {})), "stage06-semantic-patch-safety.v1"
    if cmd in {"write-semantic-safety-outputs", "semantic-safety-outputs"}:
        from codex_review.stages.stage06_fix_merge.semantic_safety import write_semantic_safety_outputs
        return write_semantic_safety_outputs(_maybe_json(args.in_path, {})), None
    if cmd == "render":
        from codex_review.stages.stage06_fix_merge.render import render_merged_fix_summary
        return render_merged_fix_summary(_maybe_json(args.in_path, {})), None
    raise ValueError(f"unknown stage06 command: {cmd}")


def _handle_stage07(args: argparse.Namespace, config: dict[str, Any]) -> tuple[Any, str | None]:
    cmd = args.command
    if cmd == "validate-current-head":
        from codex_review.stages.stage07_push.validate import validate_current_head
        validate_current_head(_maybe_json(args.pr_context, {}), _maybe_json(args.in_path, {}), args.token)
        return {"ok": True}, None
    if cmd == "apply-patch":
        from codex_review.stages.stage07_push.apply_patch import apply_merged_patch
        return apply_merged_patch(args.patch or args.in_path, args.repo_path), None
    if cmd == "run-tests":
        from codex_review.stages.stage07_push.run_tests import run_required_tests, select_test_commands
        return run_required_tests(select_test_commands(_maybe_json(args.in_path, {}), config), args.repo_path), None
    if cmd in {"validate-fix", "test-apply", "validate-and-test"}:
        from codex_review.stages.stage07_push.orchestrate import validate_and_test_fix
        return validate_and_test_fix(
            _maybe_json(args.in_path, {}),
            _maybe_json(args.pr_context, {}),
            config,
            args.repo_path,
            dry_run=args.dry_run,
            semantic_safety=_json_or_default(args.semantic_safety, {}),
        ), "stage07-validated-fix.v1"
    if cmd == "commit":
        from codex_review.stages.stage07_push.orchestrate import commit_validated_fix
        return commit_validated_fix(_maybe_json(args.in_path, {}), _maybe_json(args.pr_context, {}), config, args.repo_path), None
    if cmd in {"commit-push", "push-validated"}:
        from codex_review.stages.stage07_push.orchestrate import commit_and_push_validated_fix
        return commit_and_push_validated_fix(_maybe_json(args.in_path, {}), _maybe_json(args.validation, {}), _maybe_json(args.pr_context, {}), config, args.repo_path, args.token, dry_run=args.dry_run), "stage07-push-result.v1"
    if cmd in {"push", "run"}:
        from codex_review.stages.stage07_push.orchestrate import run_push_flow
        return run_push_flow(_maybe_json(args.in_path, {}), _maybe_json(args.pr_context, {}), config, args.repo_path, args.token, dry_run=args.dry_run), "stage07-push-result.v1"
    if cmd in {"check-loop-budget", "loop-budget"}:
        from codex_review.loop.state import build_push_entry, detect_oscillation
        validated = _maybe_json(args.in_path, {})
        # Only a patch that WOULD push can continue an oscillation; otherwise pass through.
        if not validated.get("validated"):
            return {**validated, "loop_budget_ok": True}, "stage07-validated-fix.v1"
        merged = _json_or_default(args.result, {})
        patch = merged.get("patch") or merged.get("patch_text") or ""
        if not patch and merged.get("patch_path"):
            patch = _maybe_text(str(merged["patch_path"]))
        design_plan = _json_or_default(args.design_plan, {})
        prior = _json_or_default(args.loop_state, {})
        candidate = build_push_entry(int(prior.get("round_count", 0)) + 1, validated, patch, design_plan)
        verdict = detect_oscillation(prior, candidate, config)
        if verdict["ok"]:
            return {**validated, "loop_budget_ok": True}, "stage07-validated-fix.v1"
        return {
            **validated,
            "status": verdict["status"],
            "validated": False,
            "pushed": False,
            "loop_budget_ok": False,
            "loop_budget_status": verdict["status"],
            "loop_budget_reason": verdict["reason"],
        }, "stage07-validated-fix.v1"
    if cmd in {"record-push", "record-loop-state"}:
        from codex_review.loop.state import append_push_to_loop_state, build_push_entry, write_loop_state_comment
        push_result = _maybe_json(args.in_path, {})
        # Only successful pushes extend the history; anything else is a no-op pass-through.
        if not push_result.get("pushed"):
            return {"recorded": False, "reason": "no_push"}, None
        merged = _json_or_default(args.result, {})
        patch = merged.get("patch") or merged.get("patch_text") or ""
        if not patch and merged.get("patch_path"):
            patch = _maybe_text(str(merged["patch_path"]))
        design_plan = _json_or_default(args.design_plan, {})
        prior = _json_or_default(args.loop_state, {})
        window = int(config.get("autofix", {}).get("oscillation_window", 10))
        entry = build_push_entry(int(prior.get("round_count", 0)) + 1, push_result, patch, design_plan)
        next_state = append_push_to_loop_state(prior, entry, window)
        persisted = False
        if args.token and args.pr_context:
            pr = _maybe_json(args.pr_context, {})
            owner, repo = _repo_parts_from_context(pr)
            pr_number = pr.get("pr_number")
            if owner and repo and pr_number:
                write_loop_state_comment(owner, repo, int(pr_number), next_state, args.token)
                persisted = True
        return {"recorded": True, "persisted": persisted, "loop_state": next_state}, None
    if cmd in {"write-validation-outputs", "validation-outputs"}:
        payload = _maybe_json(args.in_path, {})
        status = str(payload.get("status") or "unknown")
        validated = bool(payload.get("validated"))
        requires_push_token = validated
        # A validated patch will push, so the loop should re-review afterwards.
        # Terminal loop reasons (oscillation / round cap) stop the loop and route to the issue workflow.
        terminal_reasons = {"oscillation_detected", "max_rounds_reached", "no_diff_repeat", "no-diff-repeat"}
        loop_terminal_reason = status if (not validated and status in terminal_reasons) else ""
        should_continue = validated
        write_output("validation_status", status)
        write_output("requires_push_token", str(requires_push_token).lower())
        write_output("loop_terminal_reason", loop_terminal_reason)
        write_output("should_continue", str(should_continue).lower())
        return {
            "validation_status": status,
            "requires_push_token": requires_push_token,
            "loop_terminal_reason": loop_terminal_reason,
            "should_continue": should_continue,
        }, None
    if cmd in {"write-outputs", "github-outputs"}:
        payload = _maybe_json(args.in_path, {})
        status = str(payload.get("status") or "unknown")
        write_output("push_status", status)
        return {"push_status": status}, None
    if cmd == "render":
        from codex_review.stages.stage07_push.render import render_push_summary
        return render_push_summary(_maybe_json(args.in_path, {})), None
    raise ValueError(f"unknown stage07 command: {cmd}")


def _handle_stage08(args: argparse.Namespace, config: dict[str, Any]) -> tuple[Any, str | None]:
    cmd = args.command
    if cmd in {"record-reentry", "record"}:
        from codex_review.stages.stage08_reentry.record import build_reentry_record, persist_reentry_loop_state
        push_result = _maybe_json(args.in_path, {})
        loop_state = _json_or_default(args.loop_state, {}) or _maybe_json(args.inventory, {})
        record = build_reentry_record(push_result, loop_state, {"push_result": push_result})
        if args.token and args.pr_context:
            record = persist_reentry_loop_state(record, _maybe_json(args.pr_context, {}), args.token)
        return record, "stage08-loop-reentry.v1"
    if cmd == "validate":
        from codex_review.stages.stage08_reentry.validate import validate_reentry_record
        return validate_reentry_record(_maybe_json(args.in_path, {}), _json_or_default(args.loop_state, {})), "stage08-loop-reentry.v1"
    if cmd == "route":
        from codex_review.stages.stage08_reentry.route import determine_reentry_expectation
        return determine_reentry_expectation(_maybe_json(args.in_path, {})), None
    if cmd == "render":
        from codex_review.stages.stage08_reentry.render import render_reentry_summary
        return render_reentry_summary(_maybe_json(args.in_path, {})), None
    raise ValueError(f"unknown stage08 command: {cmd}")


def _handle_stage09(args: argparse.Namespace, config: dict[str, Any]) -> tuple[Any, str | None]:
    cmd = args.command
    if cmd in {"plan", "build-plan"}:
        from codex_review.stages.stage09_issue_fallback.issue import build_issue_fallback_plan
        payload = _json_or_default(args.in_path, {})
        reason = args.mode or payload.get("reason") or payload.get("route") or payload.get("status") or "manual_fallback"
        attempted = payload.get("attempted_stages") if isinstance(payload.get("attempted_stages"), list) else []
        deferred_items = payload.get("deferred_items") if isinstance(payload.get("deferred_items"), list) else []
        if deferred_items and not attempted:
            attempted = ["stage02_defer_to_issue"]
        return build_issue_fallback_plan(
            reason=str(reason),
            pr_context=_maybe_json(args.pr_context, {}),
            openspec_context=_json_or_default(args.openspec_context, {}),
            attempted_stages=attempted,
            deferred_items=deferred_items,
        ), "stage09-issue-fallback.v1"
    if cmd in {"infer-reason", "reason"}:
        from codex_review.stages.stage09_issue_fallback.issue import infer_issue_reason
        return infer_issue_reason(
            review_publication=_json_or_default(args.review_context, {}),
            design_route=_json_or_default(args.chief_decision, {}),
            fix_validation=_json_or_default(args.validation, {}),
            fallback_reason=args.mode,
        ), None
    if cmd in {"build-prompt", "content-prompt"}:
        from codex_review.stages.stage09_issue_fallback.issue import build_issue_content_prompt
        return build_issue_content_prompt(_maybe_json(args.in_path, {})), None
    if cmd in {"compose", "compose-content"}:
        from codex_review.stages.stage09_issue_fallback.issue import build_issue_content_prompt, compose_issue_content
        plan = _maybe_json(args.in_path, {})
        if args.result:
            content = _json_or_default(args.result, {})
        else:
            from codex_review.stages.stage09_issue_fallback.issue import CONTENT_SCHEMA_VERSION
            prompt_path = args.prompt
            if not prompt_path and args.prompt_out:
                from codex_review.model.adapter import write_prompt_if_needed
                prompt_path = str(write_prompt_if_needed(build_issue_content_prompt(plan), args.prompt_out))
            content = _model_or_fallback(
                argparse.Namespace(**{**vars(args), "prompt": prompt_path}),
                stage="stage09",
                expected_schema=CONTENT_SCHEMA_VERSION,
                fallback={"title": plan.get("title"), "body": plan.get("body")},
            )
        return compose_issue_content(plan, content), "stage09-issue-fallback.v1"
    if cmd in {"apply", "publish"}:
        from codex_review.stages.stage09_issue_fallback.issue import apply_issue_fallback
        return apply_issue_fallback(_maybe_json(args.in_path, {}), _maybe_json(args.pr_context, {}), args.token, dry_run=args.dry_run), "stage09-issue-fallback.v1"
    raise ValueError(f"unknown stage09 command: {cmd}")


def _handle_auth(args: argparse.Namespace) -> tuple[Any, str | None]:
    from codex_review.github.app_token import create_installation_token_for_repo, permissions_for_write_mode
    if args.command not in {"app-token", "github-app-token"}:
        raise ValueError(f"unknown auth command: {args.command}")
    pr = _maybe_json(args.pr_context, {})
    owner, repo = _repo_parts_from_context(pr)
    if not owner or not repo:
        raise ValidationError("auth app-token requires owner/repo from --pr-context")
    mode = args.mode or args.stage or "write"
    permissions = permissions_for_write_mode(mode)
    token = create_installation_token_for_repo(owner, repo, permissions)
    mask_secret(token)
    permissions_json = json.dumps(permissions, sort_keys=True, separators=(",", ":"))
    if os.environ.get("GITHUB_OUTPUT"):
        write_output("token", token)
        write_output("token_created", "true")
        write_output("permissions_json", permissions_json)
    return {"schema_version":"github-app-token.v1", "token_created": True, "owner": owner, "repo": repo, "permissions": permissions, "permissions_json": permissions_json, "repository_scoped": True}, None


def _handle_oidc(args: argparse.Namespace) -> tuple[Any, str | None]:
    from codex_review.github.oidc_token import (
        DEFAULT_AUDIENCE,
        DEFAULT_BROKER_URL,
        DEFAULT_RESPONSES_ENDPOINT,
        mint_relay_token,
    )
    if args.command not in {"relay-token", "mint"}:
        raise ValueError(f"unknown oidc command: {args.command}")
    audience = args.audience or DEFAULT_AUDIENCE
    broker_url = args.broker_url or DEFAULT_BROKER_URL
    credential = mint_relay_token(audience, broker_url)
    mask_secret(credential["relay_token"])
    if os.environ.get("GITHUB_OUTPUT"):
        write_output("relay_token", credential["relay_token"])
        write_output("expires_at", credential["expires_at"])
        write_output("endpoint", DEFAULT_RESPONSES_ENDPOINT)
    return {
        "schema_version": "codex-oidc-relay.v1",
        "relay_token_minted": True,
        "expires_at": credential["expires_at"],
        "endpoint": DEFAULT_RESPONSES_ENDPOINT,
    }, None


def _handle_io(args: argparse.Namespace) -> tuple[Any, str | None]:
    if args.command != "to-output":
        raise ValueError(f"unknown io command: {args.command}")
    if not args.name:
        raise ValidationError("io to-output requires --name")
    if not args.in_path:
        raise ValidationError("io to-output requires --in")
    content = read_text(args.in_path)
    write_output(args.name, content)
    return {"schema_version": "io-to-output.v1", "name": args.name, "bytes": len(content)}, None


def _handle_schema(args: argparse.Namespace) -> tuple[Any, str | None]:
    if args.command not in {"openai-strict", "openai-structured-output"}:
        raise ValueError(f"unknown schema command: {args.command}")
    if not args.schema:
        raise ValidationError("schema openai-strict requires --schema")
    from codex_review.core.schema import load_schema_json, make_openai_structured_output_schema
    return make_openai_structured_output_schema(load_schema_json(args.schema)), None


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="codex-review")
    parser.add_argument("area", choices=["auth", "oidc", "io", "event", "context", "loop", "schema", "stage00", "stage01", "stage02", "stage03", "stage04", "stage05", "stage06", "stage07", "stage08", "stage09"])
    _add_common(parser)
    args = parser.parse_args(argv)
    try:
        config = load_config(args.config) if args.area.startswith("stage") or args.area == "context" else {}
        if args.area == "auth": payload, schema = _handle_auth(args)
        elif args.area == "oidc": payload, schema = _handle_oidc(args)
        elif args.area == "io": payload, schema = _handle_io(args)
        elif args.area == "event": payload, schema = _handle_event(args)
        elif args.area == "context": payload, schema = _handle_context(args, config)
        elif args.area == "loop": payload, schema = _handle_loop(args)
        elif args.area == "schema": payload, schema = _handle_schema(args)
        elif args.area == "stage00": payload, schema = _handle_stage00(args, config)
        elif args.area == "stage01": payload, schema = _handle_stage01(args, config)
        elif args.area == "stage02": payload, schema = _handle_stage02(args, config)
        elif args.area == "stage03": payload, schema = _handle_stage03(args, config)
        elif args.area == "stage04": payload, schema = _handle_stage04(args, config)
        elif args.area == "stage05": payload, schema = _handle_stage05(args, config)
        elif args.area == "stage06": payload, schema = _handle_stage06(args, config)
        elif args.area == "stage07": payload, schema = _handle_stage07(args, config)
        elif args.area == "stage08": payload, schema = _handle_stage08(args, config)
        elif args.area == "stage09": payload, schema = _handle_stage09(args, config)
        else: raise ValueError(args.area)
        _emit(payload, args.out, schema)
        if args.summary and isinstance(payload, str):
            append_step_summary(payload)
        return 0
    except Exception as exc:
        print(json.dumps(format_error(exc, {"area": getattr(args, "area", None), "command": getattr(args, "command", None)}), ensure_ascii=False), file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
