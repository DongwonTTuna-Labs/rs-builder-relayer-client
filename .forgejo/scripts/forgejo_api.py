#!/usr/bin/env python3
"""Small Forgejo API adapter for Codex review workflows."""

from __future__ import annotations

import json
import os
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass
from typing import Any


class ForgejoApiError(RuntimeError):
    """Raised for sanitized Forgejo API failures."""


def warn(message: str) -> None:
    sys.stderr.write(f"::warning::{message}\n")


def fail(message: str) -> None:
    raise SystemExit(message)


def require_env(name: str) -> str:
    value = os.environ.get(name, "").strip()
    if not value:
        fail(f"{name} is required")
    return value


def require_token(name: str = "FORGEJO_BOT_TOKEN") -> str:
    token = os.environ.get(name, "").strip()
    if not token:
        fail(f"{name} is required")
    return token


def redact_secret(text: str, *secrets: str) -> str:
    redacted = text
    for secret in secrets:
        if secret:
            redacted = redacted.replace(secret, "<redacted>")
    return redacted


def split_repo(repo: str) -> tuple[str, str]:
    owner, sep, name = repo.partition("/")
    if not sep or not owner or not name:
        fail(f"invalid repository name: {repo!r}")
    return owner, name


@dataclass
class ForgejoClient:
    base_url: str
    repo: str
    token: str

    @classmethod
    def from_env(cls) -> "ForgejoClient":
        return cls(
            base_url=require_env("FORGEJO_API_URL").rstrip("/"),
            repo=require_env("FORGEJO_REPOSITORY"),
            token=require_token(),
        )

    def url(self, path: str, query: dict[str, str] | None = None) -> str:
        url = f"{self.base_url}/{path.lstrip('/')}"
        if query:
            url += "?" + urllib.parse.urlencode(query)
        return url

    def repo_path(self, suffix: str) -> str:
        return f"repos/{self.repo}/{suffix.lstrip('/')}"

    def request(
        self,
        method: str,
        path: str,
        body: dict[str, Any] | None = None,
        query: dict[str, str] | None = None,
        accept: str = "application/json",
    ) -> Any:
        safe_to_retry = method.upper() in {"GET", "HEAD"}
        max_attempts = 3 if safe_to_retry else 1
        backoff = 0.25
        data = None
        headers = {
            "Accept": accept,
            "Authorization": f"token {self.token}",
            "User-Agent": "forgejo-codex-review",
        }
        if body is not None:
            data = json.dumps(body).encode("utf-8")
            headers["Content-Type"] = "application/json"
        url = self.url(path, query)
        req = urllib.request.Request(url, data=data, headers=headers, method=method)
        last_error: Exception | None = None
        for attempt in range(1, max_attempts + 1):
            try:
                with urllib.request.urlopen(req, timeout=60) as response:
                    raw = response.read()
                    if not raw:
                        return None
                    content_type = response.headers.get("Content-Type", "")
                    if "json" in content_type:
                        return json.loads(raw.decode("utf-8"))
                    return raw.decode("utf-8", errors="replace")
            except urllib.error.HTTPError as exc:
                details = exc.read().decode("utf-8", errors="replace")
                safe_details = redact_secret(details[-1200:], self.token)
                if safe_to_retry and exc.code in {429, 502, 503, 504} and attempt < max_attempts:
                    time.sleep(backoff)
                    backoff *= 2
                    last_error = exc
                    continue
                raise ForgejoApiError(
                    f"{method} {path} failed with HTTP {exc.code}: {safe_details}"
                ) from exc
            except urllib.error.URLError as exc:
                if safe_to_retry and attempt < max_attempts:
                    time.sleep(backoff)
                    backoff *= 2
                    last_error = exc
                    continue
                raise ForgejoApiError(f"{method} {path} failed: {exc.reason}") from exc
        raise ForgejoApiError(f"{method} {path} failed after {max_attempts} attempts: {last_error}")

    def paginated(
        self,
        path: str,
        query: dict[str, str] | None = None,
        limit: int = 100,
        max_pages: int | None = None,
    ) -> list[dict[str, Any]]:
        items: list[dict[str, Any]] = []
        base_query = dict(query or {})
        page = 1
        while max_pages is None or page <= max_pages:
            page_query = {**base_query, "page": str(page), "limit": str(limit)}
            payload = self.request("GET", path, query=page_query)
            if not isinstance(payload, list):
                break
            items.extend(item for item in payload if isinstance(item, dict))
            if len(payload) < limit:
                break
            page += 1
        return items


def set_output(name: str, value: str) -> None:
    output = os.environ.get("GITHUB_OUTPUT")
    if output:
        with open(output, "a", encoding="utf-8") as handle:
            handle.write(f"{name}={value}\n")
    else:
        print(f"{name}={value}")
