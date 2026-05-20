#!/usr/bin/env python3
"""Stage 0 — 기존 codex-managed inline 코멘트의 해결 여부를 LLM 으로 판정.

PR 의 inline 리뷰 코멘트 중 codex-managed 이며 thread 가 unresolved 인 항목을 모아,
``BATCH_SIZE`` (기본 3) 씩 묶어 Codex 에 보낸다. 각 배치에 대한 응답은
``$RUNNER_TEMP/codex-output-resolve-batch-<N>.json`` 에 저장되고, 모두 합쳐
``$ART_DIR/resolutions.json`` 으로 출력된다.

batch 의 흐름
  1. ``GET /repos/.../pulls/{n}/comments`` 로 inline 코멘트 수집 (REST)
  2. GraphQL 로 thread isResolved=false AND first comment isMinimized=false 인 것만 필터
  3. PR head 작업 디렉토리에서 ``code_snippet`` (±SNIPPET_RADIUS 줄) 추출
  4. ``build prompt → codex_exec.sh → resolutions.schema.json`` 흐름으로 Codex 호출
  5. 정규화 후 결과 누적

필수 환경 변수
  - GH_TOKEN, GITHUB_REPOSITORY, PR_NUMBER, GITHUB_WORKSPACE, RUNNER_TEMP
선택
  - ART_DIR (default: ./artifacts)
  - BATCH_SIZE (default: 3)
  - SNIPPET_RADIUS (default: 15)
"""

from __future__ import annotations

import concurrent.futures
import json
import os
import re
import subprocess
import sys
from pathlib import Path
from typing import Iterable

from codex_redaction import redact
from gh_api import gh_graphql_paginated, gh_paginated

INLINE_MARKER = "<!-- codex-inline-review -->"
SCRIPT_DIR = Path(__file__).resolve().parent
# BASE_DIR: this file lives at <base-ref>/.github/scripts/. Going up twice
# gives the base-ref checkout root — the *trusted* source of agent prompts,
# schemas, and other instructions.
BASE_DIR = SCRIPT_DIR.parent.parent
# WORKSPACE: PR head checkout. Used ONLY to read snippets of the code under
# review (data, never executable). Falls back to GITHUB_WORKSPACE when the
# pipeline runs in single-checkout mode (e.g. local tests).
WORKSPACE = Path(
    os.environ.get("CODEX_WORKSPACE")
    or os.environ.get("GITHUB_WORKSPACE", ".")
)
RUNNER_TEMP = Path(os.environ.get("RUNNER_TEMP", "/tmp"))
ART_DIR = Path(os.environ.get("ART_DIR", "./artifacts"))
BATCH_SIZE = int(os.environ.get("BATCH_SIZE", "3"))
BATCH_PARALLELISM = int(os.environ.get("BATCH_PARALLELISM", "4"))
SNIPPET_RADIUS = int(os.environ.get("SNIPPET_RADIUS", "15"))
SCHEMA_FILE = SCRIPT_DIR / "schemas" / "resolutions.schema.json"
AGENT_PROMPT_FILE = BASE_DIR / ".codex" / "agents" / "resolve-checker-reviewer.md"

UNRESOLVED_THREADS_QUERY = """
query($owner: String!, $name: String!, $pr: Int!, $after: String) {
  repository(owner: $owner, name: $name) {
    pullRequest(number: $pr) {
      reviewThreads(first: 100, after: $after) {
        pageInfo { hasNextPage endCursor }
        nodes {
          isResolved
          comments(first: 1) {
            nodes { databaseId isMinimized }
          }
        }
      }
    }
  }
}
"""


def warn(msg: str) -> None:
    sys.stderr.write(f"::warning::{msg}\n")


def fetch_unresolved_first_comment_ids(repo: str, pr_number: str) -> set[int]:
    """첫 코멘트가 isResolved=false AND isMinimized=false 인 review thread 들의 first comment id 집합.

    페이지당 100개 thread 제한을 넘어가는 PR 에서도 모든 페이지를 순회한다.
    """
    owner, name = repo.split("/", 1)
    threads = gh_graphql_paginated(
        UNRESOLVED_THREADS_QUERY,
        {"owner": owner, "name": name, "pr": int(pr_number)},
        extract=lambda payload: (
            payload.get("data", {})
            .get("repository", {})
            .get("pullRequest", {})
            .get("reviewThreads", {})
        ),
    )
    out: set[int] = set()
    for thread in threads:
        if thread.get("isResolved"):
            continue
        for c in (thread.get("comments") or {}).get("nodes") or []:
            if c.get("isMinimized"):
                continue
            db_id = c.get("databaseId")
            if db_id is not None:
                out.add(int(db_id))
    return out


def extract_marker_key(body: str) -> str:
    match = re.search(r"<!--\s*codex:key:([0-9a-f]+)\s*-->", body or "")
    return match.group(1) if match else ""


def read_snippet(rel_path: str, line: int) -> str | None:
    target = WORKSPACE / rel_path
    if not target.is_file():
        return None
    try:
        lines = target.read_text(encoding="utf-8", errors="replace").splitlines()
    except OSError:
        return None
    if not lines:
        return None
    center = max(1, min(line, len(lines)))
    start = max(1, center - SNIPPET_RADIUS)
    end = min(len(lines), center + SNIPPET_RADIUS)
    numbered = []
    for n in range(start, end + 1):
        prefix = ">>" if n == center else "  "
        numbered.append(f"{prefix} {n:5d}  {lines[n - 1]}")
    return "\n".join(numbered)


def chunked(items: list[dict], size: int) -> Iterable[list[dict]]:
    for i in range(0, len(items), size):
        yield items[i : i + size]


def load_agent_body() -> str:
    if AGENT_PROMPT_FILE.is_file():
        return AGENT_PROMPT_FILE.read_text(encoding="utf-8")
    raise SystemExit(f"agent prompt not found: {AGENT_PROMPT_FILE}")


def build_prompt(agent_body: str, batch: list[dict], batch_index: int) -> Path:
    payload = {"comments": batch}
    out_path = RUNNER_TEMP / f"prompt-resolve-batch-{batch_index}.md"
    out_path.write_text(
        agent_body
        + "\n\n# Resolve Check Batch Payload\n\n```json\n"
        + json.dumps(payload, ensure_ascii=False, indent=2)
        + "\n```\n",
        encoding="utf-8",
    )
    return out_path


def run_codex(prompt_path: Path, out_path: Path) -> None:
    env = os.environ.copy()
    env.update(
        {
            "PROMPT_FILE": str(prompt_path),
            "SCHEMA_FILE": str(SCHEMA_FILE),
            "OUT_FILE": str(out_path),
            "LOG_FILE": str(RUNNER_TEMP / f"{prompt_path.stem}.log"),
        }
    )
    completed = subprocess.run(
        ["bash", str(SCRIPT_DIR / "codex_exec.sh")],
        env=env,
        check=False,
    )
    if completed.returncode != 0:
        raise SystemExit(completed.returncode)


def normalize_batch_output(raw_path: Path, expected_ids: set[int]) -> list[dict]:
    """Codex 출력을 정규화. expected_ids 의 모든 코멘트에 대해 행을 만든다."""
    if not raw_path.is_file():
        warn(f"resolve batch output missing: {raw_path.name}")
        return [
            {"comment_id": cid, "resolved": False, "reason": "Codex output missing"}
            for cid in expected_ids
        ]
    try:
        raw = json.loads(raw_path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        warn(f"failed to parse {raw_path.name}: {exc}")
        return [
            {"comment_id": cid, "resolved": False, "reason": "Codex output invalid"}
            for cid in expected_ids
        ]

    rows_by_id: dict[int, dict] = {}
    for row in raw.get("resolutions") or []:
        if not isinstance(row, dict):
            continue
        try:
            cid = int(row.get("comment_id") or 0)
        except (TypeError, ValueError):
            continue
        if cid <= 0:
            continue
        reason = str(row.get("reason") or "").strip()[:300]
        if not reason:
            continue
        rows_by_id[cid] = {
            "comment_id": cid,
            "resolved": bool(row.get("resolved")),
            "reason": reason,
        }

    final = []
    for cid in expected_ids:
        if cid in rows_by_id:
            final.append(rows_by_id[cid])
        else:
            final.append(
                {
                    "comment_id": cid,
                    "resolved": False,
                    "reason": "Codex did not emit a verdict for this comment.",
                }
            )
    return final


def main() -> int:
    repo = os.environ["GITHUB_REPOSITORY"]
    pr_number = os.environ["PR_NUMBER"]

    ART_DIR.mkdir(parents=True, exist_ok=True)
    out_path = ART_DIR / "resolutions.json"
    out_path.write_text(
        json.dumps({"resolutions": []}, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )

    # isResolved=true 인 thread / isMinimized=true 인 첫 코멘트는 검토 대상에서 제외.
    unresolved_ids = fetch_unresolved_first_comment_ids(repo, pr_number)

    comments = gh_paginated(f"repos/{repo}/pulls/{pr_number}/comments")
    codex_managed: list[dict] = []
    for c in comments:
        if INLINE_MARKER not in (c.get("body") or ""):
            continue
        if c.get("id") is None:
            continue
        cid = int(c.get("id") or 0)
        if cid not in unresolved_ids:
            continue
        live_line = int(c.get("line") or 0)
        original_line = int(c.get("original_line") or 0)
        # Outdated 댓글은 `line` 이 빈 채 `original_line` 만 남는다. 그대로
        # 보내면 LLM 이 위치 정보 없이 판정해 잘못 닫을 위험이 있어,
        # original_line 으로 fallback 하고 outdated 플래그를 같이 넘긴다.
        outdated = live_line <= 0 and original_line >= 1
        effective_line = live_line if live_line >= 1 else original_line
        codex_managed.append(
            {
                "comment_id": cid,
                "file": str(c.get("path") or ""),
                "line": effective_line,
                "outdated": outdated,
                "marker_key": extract_marker_key(c.get("body") or ""),
                "body_excerpt": redact(str(c.get("body") or ""))[:4000],
                "code_snippet": None,
            }
        )

    if not codex_managed:
        print("no codex-managed inline comments to check; resolutions.json is empty")
        return 0

    for entry in codex_managed:
        if entry["file"] and entry["line"]:
            entry["code_snippet"] = read_snippet(entry["file"], entry["line"])

    agent_body = load_agent_body()

    batches = list(enumerate(chunked(codex_managed, BATCH_SIZE), start=1))

    def run_batch(batch_index: int, batch: list[dict]) -> list[dict]:
        prompt_path = build_prompt(agent_body, batch, batch_index)
        out_batch_path = RUNNER_TEMP / f"codex-output-resolve-batch-{batch_index}.json"
        expected = {b["comment_id"] for b in batch}
        try:
            run_codex(prompt_path, out_batch_path)
        except SystemExit as exc:
            warn(
                f"resolve-check batch {batch_index} failed (exit={exc.code}); "
                "falling back to resolved=False"
            )
            return normalize_batch_output(Path("/nonexistent"), expected)
        return normalize_batch_output(out_batch_path, expected)

    # Codex 호출은 각각 외부 프로세스로 IO 바운드라 ThreadPoolExecutor 로 충분히
    # 병렬화된다. batch_index 키로 deterministic ordering 유지.
    workers = max(1, min(BATCH_PARALLELISM, len(batches)))
    print(
        f"resolve-check: {len(codex_managed)} comment(s) in {len(batches)} batch(es), "
        f"running with {workers} worker(s)"
    )
    results_by_index: dict[int, list[dict]] = {}
    with concurrent.futures.ThreadPoolExecutor(max_workers=workers) as executor:
        futures = {
            executor.submit(run_batch, batch_index, batch): batch_index
            for batch_index, batch in batches
        }
        for future in concurrent.futures.as_completed(futures):
            batch_index = futures[future]
            results_by_index[batch_index] = future.result()

    resolutions: list[dict] = []
    for batch_index, _ in batches:
        resolutions.extend(results_by_index.get(batch_index, []))

    out_path.write_text(
        json.dumps({"resolutions": resolutions}, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )
    resolved_count = sum(1 for r in resolutions if r["resolved"])
    print(
        f"resolve-check finished: {len(resolutions)} comment(s) inspected, "
        f"{resolved_count} marked resolved"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
