#!/usr/bin/env python3
"""Small Forgejo API adapter for Codex review workflows."""

from __future__ import annotations

import json
import os
import re
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
import codecs
from collections.abc import Callable
from dataclasses import dataclass
from typing import Any

DEFAULT_MAX_PAGES = 50


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


def sanitize_error_detail(text: str, *secrets: str) -> str:
    redacted = redact_secret(text, *secrets)
    redacted = re.sub(r"://[^/\s:@]+:[^/\s@]+@", "://<redacted>@", redacted)
    redacted = re.sub(
        r"([?&](?:access_)?token=)[^&\s]+",
        r"\1<redacted>",
        redacted,
        flags=re.IGNORECASE,
    )
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
                safe_details = sanitize_error_detail(details[-1200:], self.token)
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
                safe_reason = sanitize_error_detail(str(exc.reason), self.token)
                raise ForgejoApiError(f"{method} {path} failed: {safe_reason}") from exc
            except TimeoutError as exc:
                if safe_to_retry and attempt < max_attempts:
                    time.sleep(backoff)
                    backoff *= 2
                    last_error = exc
                    continue
                safe_reason = sanitize_error_detail(str(exc), self.token)
                raise ForgejoApiError(f"{method} {path} failed while reading response: {safe_reason}") from exc
        safe_last_error = sanitize_error_detail(str(last_error), self.token)
        raise ForgejoApiError(f"{method} {path} failed after {max_attempts} attempts: {safe_last_error}")

    def request_text_prefix(
        self,
        path: str,
        prefix_chars: int,
        query: dict[str, str] | None = None,
        accept: str = "text/plain",
        on_line: Callable[[str], None] | None = None,
        chunk_size: int = 65536,
        continue_after_prefix: bool = True,
        max_stream_chars: int | None = None,
    ) -> tuple[str, bool]:
        """Stream a text response, keeping only a prefix while optionally visiting every line."""

        max_attempts = 3
        backoff = 0.25
        headers = {
            "Accept": accept,
            "Authorization": f"token {self.token}",
            "User-Agent": "forgejo-codex-review",
        }
        url = self.url(path, query)
        req = urllib.request.Request(url, headers=headers, method="GET")
        last_error: Exception | None = None

        for attempt in range(1, max_attempts + 1):
            try:
                with urllib.request.urlopen(req, timeout=60) as response:
                    decoder = codecs.getincrementaldecoder("utf-8")("replace")
                    prefix_parts: list[str] = []
                    prefix_len = 0
                    stream_len = 0
                    truncated = False
                    stopped_after_prefix = False
                    pending_line_parts: list[str] = []

                    def consume(text: str, final: bool = False) -> None:
                        nonlocal pending_line_parts
                        if on_line is None:
                            return
                        if not text and not final:
                            return
                        for line in text.splitlines(keepends=True):
                            if line.endswith(("\n", "\r")):
                                if pending_line_parts:
                                    pending_line_parts.append(line)
                                    on_line("".join(pending_line_parts).rstrip("\r\n"))
                                    pending_line_parts = []
                                else:
                                    on_line(line.rstrip("\r\n"))
                            else:
                                pending_line_parts.append(line)
                        if final and pending_line_parts:
                            on_line("".join(pending_line_parts))
                            pending_line_parts = []

                    while True:
                        raw = response.read(chunk_size)
                        if not raw:
                            break
                        text = decoder.decode(raw)
                        decoded_len = len(text)
                        if max_stream_chars is not None:
                            remaining_stream = max_stream_chars - stream_len
                            if remaining_stream <= 0:
                                truncated = True
                                stopped_after_prefix = True
                                break
                            if len(text) > remaining_stream:
                                text = text[:remaining_stream]
                                truncated = True
                                stopped_after_prefix = True
                            stream_len += len(text)
                        kept_text = text
                        if prefix_len < prefix_chars:
                            remaining = prefix_chars - prefix_len
                            kept_text = text[:remaining]
                            prefix_parts.append(kept_text)
                            prefix_len += min(len(text), remaining)
                            if len(text) > remaining:
                                truncated = True
                                if not continue_after_prefix:
                                    stopped_after_prefix = True
                                    consume(kept_text)
                                    break
                        elif text:
                            truncated = True
                            if not continue_after_prefix:
                                break
                        consume(text)
                        if truncated and max_stream_chars is not None and stream_len >= max_stream_chars:
                            break

                    tail = decoder.decode(b"", final=True)
                    if tail:
                        if prefix_len < prefix_chars:
                            remaining = prefix_chars - prefix_len
                            prefix_parts.append(tail[:remaining])
                            prefix_len += min(len(tail), remaining)
                            if len(tail) > remaining:
                                truncated = True
                        else:
                            truncated = True
                        consume(tail)
                    if not stopped_after_prefix:
                        consume("", final=True)
                    return "".join(prefix_parts), truncated
            except urllib.error.HTTPError as exc:
                details = exc.read().decode("utf-8", errors="replace")
                safe_details = sanitize_error_detail(details[-1200:], self.token)
                if exc.code in {429, 502, 503, 504} and attempt < max_attempts:
                    time.sleep(backoff)
                    backoff *= 2
                    last_error = exc
                    continue
                raise ForgejoApiError(f"GET {path} failed with HTTP {exc.code}: {safe_details}") from exc
            except urllib.error.URLError as exc:
                if attempt < max_attempts:
                    time.sleep(backoff)
                    backoff *= 2
                    last_error = exc
                    continue
                safe_reason = sanitize_error_detail(str(exc.reason), self.token)
                raise ForgejoApiError(f"GET {path} failed: {safe_reason}") from exc
            except TimeoutError as exc:
                if attempt < max_attempts:
                    time.sleep(backoff)
                    backoff *= 2
                    last_error = exc
                    continue
                safe_reason = sanitize_error_detail(str(exc), self.token)
                raise ForgejoApiError(f"GET {path} failed while reading response: {safe_reason}") from exc

        safe_last_error = sanitize_error_detail(str(last_error), self.token)
        raise ForgejoApiError(f"GET {path} failed after {max_attempts} attempts: {safe_last_error}")

    def authenticated_login(self) -> str | None:
        payload = self.request("GET", "user")
        if not isinstance(payload, dict):
            return None
        login = str(payload.get("login") or payload.get("username") or "").strip()
        return login or None

    def paginated(
        self,
        path: str,
        query: dict[str, str] | None = None,
        limit: int = 100,
        max_pages: int | None = DEFAULT_MAX_PAGES,
    ) -> list[dict[str, Any]]:
        items: list[dict[str, Any]] = []
        seen_ids: set[str] = set()
        base_query = dict(query or {})
        page = 1
        while max_pages is None or page <= max_pages:
            page_query = {**base_query, "page": str(page), "limit": str(limit)}
            payload = self.request("GET", path, query=page_query)
            if not isinstance(payload, list):
                break
            if not payload:
                break
            page_items = [item for item in payload if isinstance(item, dict)]
            id_values = [
                str(item.get("id"))
                for item in page_items
                if item.get("id") is not None
            ]
            if id_values and not any(item_id not in seen_ids for item_id in id_values):
                break
            for item in page_items:
                item_id = item.get("id")
                if item_id is not None:
                    item_id_text = str(item_id)
                    if item_id_text in seen_ids:
                        continue
                    seen_ids.add(item_id_text)
                items.append(item)
            page += 1
        if max_pages is not None and page > max_pages:
            warn(f"pagination for {path} stopped after max_pages={max_pages}")
        return items


def set_output(name: str, value: str) -> None:
    output = os.environ.get("GITHUB_OUTPUT")
    if output:
        with open(output, "a", encoding="utf-8") as handle:
            handle.write(f"{name}={value}\n")
    else:
        print(f"{name}={value}")
