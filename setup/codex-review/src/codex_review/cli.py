"""CLI entrypoints for Codex review v3 stages."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

from .artifacts import write_json_artifact
from .relay import default_relay_contract


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


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="codex-review")
    subparsers = parser.add_subparsers(dest="command", required=True)
    for stage in STAGE_SCHEMAS:
        _add_stage_parser(subparsers, stage)
    relay = subparsers.add_parser("relay-contract")
    relay.set_defaults(func=run_relay_contract)
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    return int(args.func(args))


if __name__ == "__main__":
    raise SystemExit(main())
