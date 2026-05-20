"""GitHub API 호출 공통 헬퍼.

REST 호출 (`gh_json`, `gh_paginated`) 은 ``gh`` CLI 를 통해 보내고, GraphQL
호출 (`gh_graphql`) 은 ``urllib`` 로 직접 보낸다. 두 방식 모두 ``GH_TOKEN``
환경 변수의 토큰을 사용한다 (Codex PR Review 워크플로우에서는 GitHub App
installation token).
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import urllib.error
import urllib.request
from typing import Any, Callable

PER_PAGE = 100
GRAPHQL_PAGE_SIZE = 100
GRAPHQL_PAGE_LIMIT = 50  # safety bound (5000 nodes); avoids runaway loops


def gh_json(
    endpoint: str,
    method: str = "GET",
    payload: dict | None = None,
) -> Any:
    """Single ``gh api`` call. Returns parsed JSON, or ``None`` for empty bodies."""
    cmd = ["gh", "api"]
    if method != "GET":
        cmd.extend(["-X", method])
    cmd.append(endpoint)
    if payload is not None:
        cmd.extend(["--input", "-"])
    completed = subprocess.run(
        cmd,
        input=json.dumps(payload).encode("utf-8") if payload is not None else None,
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if completed.returncode != 0:
        sys.stderr.write(completed.stderr.decode("utf-8", "replace"))
        raise SystemExit(completed.returncode)
    if not completed.stdout.strip():
        return None
    return json.loads(completed.stdout.decode("utf-8"))


def gh_paginated(endpoint: str) -> list[dict]:
    """``gh api`` 의 페이지네이션 처리. 모든 페이지의 항목을 모아 반환."""
    items: list[dict] = []
    page = 1
    while True:
        separator = "&" if "?" in endpoint else "?"
        batch = gh_json(f"{endpoint}{separator}per_page={PER_PAGE}&page={page}")
        if not batch:
            break
        items.extend(batch)
        if len(batch) < PER_PAGE:
            break
        page += 1
    return items


def gh_graphql(query: str, variables: dict[str, object]) -> dict:
    """GitHub GraphQL API 직접 호출 (urllib).

    HTTP error / GraphQL ``errors`` 응답 모두 ``stderr`` 에 명시적으로 출력하고
    ``SystemExit(1)`` 을 던진다.
    """
    token = os.environ.get("GH_TOKEN") or os.environ.get("GITHUB_TOKEN")
    if not token:
        raise SystemExit("GH_TOKEN / GITHUB_TOKEN is required for GraphQL calls")
    body = json.dumps({"query": query, "variables": variables}).encode("utf-8")
    req = urllib.request.Request(
        "https://api.github.com/graphql",
        data=body,
        headers={
            "Authorization": f"Bearer {token}",
            "Content-Type": "application/json",
            "Accept": "application/vnd.github.v4+json",
            "User-Agent": "codex-pr-review-script",
        },
        method="POST",
    )
    try:
        with urllib.request.urlopen(req, timeout=30) as resp:
            raw = resp.read()
    except urllib.error.HTTPError as exc:
        sys.stderr.write(
            f"GraphQL HTTP {exc.code}: {exc.read().decode('utf-8', 'replace')}\n"
        )
        raise SystemExit(1) from exc
    except urllib.error.URLError as exc:
        sys.stderr.write(f"GraphQL network error: {exc}\n")
        raise SystemExit(1) from exc
    payload = json.loads(raw.decode("utf-8"))
    if payload.get("errors"):
        sys.stderr.write(
            f"GraphQL errors: {json.dumps(payload['errors'], ensure_ascii=False)}\n"
        )
        raise SystemExit(1)
    return payload


def gh_graphql_paginated(
    query: str,
    variables: dict[str, object],
    extract: Callable[[dict], dict],
) -> list[dict]:
    """Cursor-based pagination for a GraphQL connection.

    ``query`` MUST accept an ``$after: String`` variable and the connection
    MUST return ``pageInfo { hasNextPage endCursor }`` alongside ``nodes``.
    ``extract`` receives the raw payload (``{"data": ...}``) and must return
    the connection dict (the one containing ``nodes`` / ``pageInfo``).

    Iterates until the GitHub API reports ``hasNextPage=false`` or a safety
    cap is reached, and returns the concatenated ``nodes`` list.
    """
    nodes: list[dict] = []
    cursor: str | None = None
    for _ in range(GRAPHQL_PAGE_LIMIT):
        page_vars = dict(variables)
        page_vars["after"] = cursor
        payload = gh_graphql(query, page_vars)
        connection = extract(payload) or {}
        page_nodes = connection.get("nodes") or []
        nodes.extend(page_nodes)
        page_info = connection.get("pageInfo") or {}
        if not page_info.get("hasNextPage"):
            return nodes
        cursor = page_info.get("endCursor")
        if not cursor:
            return nodes
    sys.stderr.write(
        f"::warning::gh_graphql_paginated hit GRAPHQL_PAGE_LIMIT={GRAPHQL_PAGE_LIMIT}; "
        "remaining pages truncated\n"
    )
    return nodes
