"""CLI entrypoints for Codex review v3 stages."""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path

from .artifacts import write_json_artifact
from .relay import default_relay_contract, normalize_codex_args


STAGE_SCHEMAS = {
    "stage00-resolve-gate": "codex.stage00.resolve_gate.v1",
    "stage01-review": "codex.stage01.review.v1",
    "stage02-techlead": "codex.stage02.techlead.v1",
    "stage03-design": "codex.stage03.design.v1",
    "stage04-design-chief": "codex.stage04.design_chief.v1",
    "stage05-fix-dispatch": "codex.stage05.fix_dispatch.v1",
    "stage06-fix-merge": "codex.stage06.fix_merge.v1",
    "stage07-push": "codex.stage07.push.v1",
    "stage08-reentry": "codex.stage08.reentry.v1",
}


def _add_stage_parser(subparsers: argparse._SubParsersAction[argparse.ArgumentParser], name: str) -> None:
    parser = subparsers.add_parser(name)
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--out", type=Path, required=False)
    if name == "stage00-resolve-gate":
        parser.add_argument("--inventory", type=Path, required=False)
    if name == "stage01-review":
        parser.add_argument("--request", type=Path, required=False)
        parser.add_argument("--model-output", type=Path, required=False)
    if name == "stage02-techlead":
        parser.add_argument("--review", type=Path, required=False)
    if name == "stage03-design":
        parser.add_argument("--review", type=Path, required=False)
        parser.add_argument("--techlead", type=Path, required=False)
        parser.add_argument("--model-design", type=Path, required=False)
    if name == "stage04-design-chief":
        parser.add_argument("--design", type=Path, required=False)
        parser.add_argument("--model-approval", type=Path, required=False)
    if name == "stage05-fix-dispatch":
        parser.add_argument("--design", type=Path, required=False)
        parser.add_argument("--design-chief", type=Path, required=False)
    if name == "stage06-fix-merge":
        parser.add_argument("--dispatch", type=Path, required=False)
        parser.add_argument("--fix-outputs", type=Path, required=False)
    if name == "stage07-push":
        parser.add_argument("--fix-merge", type=Path, required=False)
        parser.add_argument("--trusted-push", type=Path, required=False)
    if name == "stage08-reentry":
        parser.add_argument("--push", type=Path, required=False)
        parser.add_argument("--run-state", type=Path, required=False)
    parser.set_defaults(func=lambda args, stage=name: run_stage(stage, args))


def run_stage(stage: str, args: argparse.Namespace) -> int:
    if stage == "stage00-resolve-gate" and getattr(args, "inventory", None):
        from .artifacts import read_json_artifact
        from .stage00 import build_resolve_gate_result

        payload = build_resolve_gate_result(read_json_artifact(args.inventory))
        if args.out:
            write_json_artifact(args.out, payload)
        else:
            print(json.dumps(payload, ensure_ascii=False, sort_keys=True))
        return 0
    if stage == "stage01-review" and getattr(args, "request", None) and getattr(args, "model_output", None):
        from .artifacts import read_json_artifact
        from .stage01 import build_review_result

        payload = build_review_result(read_json_artifact(args.request), read_json_artifact(args.model_output))
        if args.out:
            write_json_artifact(args.out, payload)
        else:
            print(json.dumps(payload, ensure_ascii=False, sort_keys=True))
        return 0
    if stage == "stage02-techlead" and getattr(args, "review", None):
        from .artifacts import read_json_artifact
        from .stage02 import build_techlead_result

        payload = build_techlead_result(read_json_artifact(args.review))
        if args.out:
            write_json_artifact(args.out, payload)
        else:
            print(json.dumps(payload, ensure_ascii=False, sort_keys=True))
        return 0
    if (
        stage == "stage03-design"
        and getattr(args, "review", None)
        and getattr(args, "techlead", None)
        and getattr(args, "model_design", None)
    ):
        from .artifacts import read_json_artifact
        from .stage03 import build_design_result

        payload = build_design_result(
            read_json_artifact(args.review),
            read_json_artifact(args.techlead),
            read_json_artifact(args.model_design),
        )
        if args.out:
            write_json_artifact(args.out, payload)
        else:
            print(json.dumps(payload, ensure_ascii=False, sort_keys=True))
        return 0
    if stage == "stage04-design-chief" and getattr(args, "design", None) and getattr(args, "model_approval", None):
        from .artifacts import read_json_artifact
        from .stage04 import build_design_chief_result

        payload = build_design_chief_result(read_json_artifact(args.design), read_json_artifact(args.model_approval))
        if args.out:
            write_json_artifact(args.out, payload)
        else:
            print(json.dumps(payload, ensure_ascii=False, sort_keys=True))
        return 0
    if stage == "stage05-fix-dispatch" and getattr(args, "design", None) and getattr(args, "design_chief", None):
        from .artifacts import read_json_artifact
        from .stage05 import build_fix_dispatch_result

        payload = build_fix_dispatch_result(read_json_artifact(args.design), read_json_artifact(args.design_chief))
        if args.out:
            write_json_artifact(args.out, payload)
        else:
            print(json.dumps(payload, ensure_ascii=False, sort_keys=True))
        return 0
    if stage == "stage06-fix-merge" and getattr(args, "dispatch", None) and getattr(args, "fix_outputs", None):
        from .artifacts import read_json_artifact
        from .stage06 import build_fix_merge_result

        payload = build_fix_merge_result(read_json_artifact(args.dispatch), read_json_artifact(args.fix_outputs))
        if args.out:
            write_json_artifact(args.out, payload)
        else:
            print(json.dumps(payload, ensure_ascii=False, sort_keys=True))
        return 0
    if stage == "stage07-push" and getattr(args, "fix_merge", None) and getattr(args, "trusted_push", None):
        from .artifacts import read_json_artifact
        from .stage07 import build_push_result

        payload = build_push_result(read_json_artifact(args.fix_merge), read_json_artifact(args.trusted_push))
        if args.out:
            write_json_artifact(args.out, payload)
        else:
            print(json.dumps(payload, ensure_ascii=False, sort_keys=True))
        return 0
    if stage == "stage08-reentry" and getattr(args, "push", None) and getattr(args, "run_state", None):
        from .artifacts import read_json_artifact
        from .stage08 import build_reentry_result

        payload = build_reentry_result(read_json_artifact(args.push), read_json_artifact(args.run_state))
        if args.out:
            write_json_artifact(args.out, payload)
        else:
            print(json.dumps(payload, ensure_ascii=False, sort_keys=True))
        return 0
    if not args.dry_run:
        print(f"{stage} requires --dry-run in phase 1", file=sys.stderr)
        return 2
    payload = {
        "schema_version": STAGE_SCHEMAS[stage],
        "stage": stage,
        "status": "dry_run",
        "side_effects": [],
    }
    if args.out:
        write_json_artifact(args.out, payload)
    else:
        print(json.dumps(payload, ensure_ascii=False, sort_keys=True))
    return 0


def run_relay_contract(_: argparse.Namespace) -> int:
    print(json.dumps(default_relay_contract().to_dict(), ensure_ascii=False, indent=2, sort_keys=True))
    return 0


def run_normalize_codex_args(args: argparse.Namespace) -> int:
    print(normalize_codex_args(args.raw))
    return 0


def run_stage00_context(args: argparse.Namespace) -> int:
    from .artifacts import read_json_artifact
    from .stage00 import build_context_artifacts

    payload = build_context_artifacts(
        read_json_artifact(args.pr_json),
        read_json_artifact(args.review_threads),
        repository=args.repository,
        pr_number=args.pr_number,
        base_sha=args.base_sha,
        run_id=args.run_id,
        event_name=args.event_name,
    )
    write_json_artifact(args.out_dir / "thread-inventory.json", payload["thread_inventory"])
    write_json_artifact(args.out_dir / "review-request.json", payload["review_request"])
    write_json_artifact(args.out_dir / "run-state.json", payload["run_state"])
    if args.github_output:
        with args.github_output.open("a", encoding="utf-8") as output:
            for key in ["pr_number", "base_sha", "head_sha", "head_ref", "head_repo"]:
                print(f"{key}={payload['outputs'][key]}", file=output)
    return 0


def run_stage05_fallback_fix_outputs(args: argparse.Namespace) -> int:
    from .artifacts import read_json_artifact
    from .stage06 import build_conflict_fix_outputs

    payload = build_conflict_fix_outputs(read_json_artifact(args.dispatch), args.reason)
    if args.out:
        write_json_artifact(args.out, payload)
    else:
        print(json.dumps(payload, ensure_ascii=False, sort_keys=True))
    return 0


def run_stage06_deferred_validation(args: argparse.Namespace) -> int:
    from .artifacts import read_json_artifact
    from .stage06 import run_deferred_validation

    try:
        payload = run_deferred_validation(read_json_artifact(args.fix_merge), args.workspace)
    except ValueError as exc:
        print(str(exc), file=sys.stderr)
        return 1
    except subprocess.CalledProcessError as exc:
        return int(exc.returncode)
    if args.out:
        write_json_artifact(args.out, payload)
    else:
        print(json.dumps(payload, ensure_ascii=False, sort_keys=True))
    return 0


def run_stage06_finalize_validation(args: argparse.Namespace) -> int:
    from .artifacts import read_json_artifact
    from .stage06 import build_validated_fix_merge_result

    try:
        payload = build_validated_fix_merge_result(
            read_json_artifact(args.fix_merge),
            read_json_artifact(args.validation),
        )
    except ValueError as exc:
        print(str(exc), file=sys.stderr)
        return 1
    if args.out:
        write_json_artifact(args.out, payload)
    else:
        print(json.dumps(payload, ensure_ascii=False, sort_keys=True))
    return 0


def run_stage00_lifecycle(args: argparse.Namespace) -> int:
    from .artifacts import read_json_artifact
    from .stage00 import build_lifecycle_result

    payload = build_lifecycle_result(read_json_artifact(args.gate), read_json_artifact(args.inventory))
    if args.out:
        write_json_artifact(args.out, payload)
    else:
        print(json.dumps(payload, ensure_ascii=False, sort_keys=True))
    return 0


def run_stage02_comment(args: argparse.Namespace) -> int:
    from .artifacts import read_json_artifact
    from .stage02 import build_review_comment_body

    body = build_review_comment_body(
        read_json_artifact(args.review),
        read_json_artifact(args.techlead),
        run_url=args.run_url,
    )
    if args.out:
        args.out.write_text(body, encoding="utf-8")
    else:
        print(body, end="")
    return 0


def run_stage07_validation(args: argparse.Namespace) -> int:
    from .stage07 import run_validation_commands

    commands = args.commands.read_text(encoding="utf-8").splitlines()
    try:
        run_validation_commands(commands, args.workspace)
    except ValueError as exc:
        print(str(exc), file=sys.stderr)
        return 1
    except subprocess.CalledProcessError as exc:
        return int(exc.returncode)
    return 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="codex-review")
    subparsers = parser.add_subparsers(dest="command", required=True)
    for stage in STAGE_SCHEMAS:
        _add_stage_parser(subparsers, stage)
    context = subparsers.add_parser("stage00-context")
    context.add_argument("--pr-json", type=Path, required=True)
    context.add_argument("--review-threads", type=Path, required=True)
    context.add_argument("--repository", required=True)
    context.add_argument("--pr-number", required=True)
    context.add_argument("--base-sha", required=True)
    context.add_argument("--run-id", required=True)
    context.add_argument("--event-name", required=True)
    context.add_argument("--out-dir", type=Path, required=True)
    context.add_argument("--github-output", type=Path, required=False)
    context.set_defaults(func=run_stage00_context)
    lifecycle = subparsers.add_parser("stage00-lifecycle")
    lifecycle.add_argument("--gate", type=Path, required=True)
    lifecycle.add_argument("--inventory", type=Path, required=True)
    lifecycle.add_argument("--out", type=Path, required=False)
    lifecycle.set_defaults(func=run_stage00_lifecycle)
    comment = subparsers.add_parser("stage02-comment")
    comment.add_argument("--review", type=Path, required=True)
    comment.add_argument("--techlead", type=Path, required=True)
    comment.add_argument("--run-url", required=True)
    comment.add_argument("--out", type=Path, required=False)
    comment.set_defaults(func=run_stage02_comment)
    fallback = subparsers.add_parser("stage05-fallback-fix-outputs")
    fallback.add_argument("--dispatch", type=Path, required=True)
    fallback.add_argument("--reason", required=True)
    fallback.add_argument("--out", type=Path, required=False)
    fallback.set_defaults(func=run_stage05_fallback_fix_outputs)
    deferred = subparsers.add_parser("stage06-run-deferred-validation")
    deferred.add_argument("--fix-merge", type=Path, required=True)
    deferred.add_argument("--workspace", type=Path, required=True)
    deferred.add_argument("--out", type=Path, required=False)
    deferred.set_defaults(func=run_stage06_deferred_validation)
    finalize = subparsers.add_parser("stage06-finalize-validation")
    finalize.add_argument("--fix-merge", type=Path, required=True)
    finalize.add_argument("--validation", type=Path, required=True)
    finalize.add_argument("--out", type=Path, required=False)
    finalize.set_defaults(func=run_stage06_finalize_validation)
    validation = subparsers.add_parser("stage07-run-validation")
    validation.add_argument("--commands", type=Path, required=True)
    validation.add_argument("--workspace", type=Path, required=True)
    validation.set_defaults(func=run_stage07_validation)
    relay = subparsers.add_parser("relay-contract")
    relay.set_defaults(func=run_relay_contract)
    normalize = subparsers.add_parser("normalize-codex-args")
    normalize.add_argument("--raw", required=True)
    normalize.set_defaults(func=run_normalize_codex_args)
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    return int(args.func(args))


if __name__ == "__main__":
    raise SystemExit(main())
