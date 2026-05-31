"""OIDC relay contract representation for the v3 workflow."""

from __future__ import annotations

import json
from dataclasses import asdict, dataclass


DEFAULT_RELAY_ARGS_SHA = "d7f6aad9e595f8b84e6b7659586706ef09f85c675b37661944b83fa0cf05cff3"


@dataclass(frozen=True)
class RelayContract:
    action: str
    audience: str
    broker_url: str
    relay_env_key: str
    codex_args_sha256: str
    trusted_actors: tuple[str, ...]

    def to_dict(self) -> dict[str, object]:
        payload = asdict(self)
        payload["trusted_actors"] = list(self.trusted_actors)
        return payload


def default_relay_contract() -> RelayContract:
    return RelayContract(
        action="DongwonTTuna-Labs/home-server-infra/.github/actions/setup-codex-relay@main",
        audience="https://relay-ai.dongwontuna.net/github-actions",
        broker_url="https://relay-ai.dongwontuna.net/v1/oidc/token",
        relay_env_key="AI_RELAY_API_KEY",
        codex_args_sha256=DEFAULT_RELAY_ARGS_SHA,
        trusted_actors=("DongwonTTuna", "codex-reviewer-for-dongwonttuna[bot]"),
    )


def normalize_codex_args(raw_args: str) -> str:
    try:
        args = json.loads(raw_args)
    except json.JSONDecodeError as exc:
        raise ValueError("codex args must be valid JSON") from exc
    if not isinstance(args, list) or not all(isinstance(arg, str) for arg in args):
        raise ValueError("codex args must be a JSON string array")
    normalized: list[str] = []
    index = 0
    while index < len(args):
        if args[index : index + 2] == ["--enable", "use_legacy_landlock"]:
            index += 2
            continue
        normalized.append(args[index])
        index += 1
    return json.dumps(normalized, ensure_ascii=True, separators=(",", ":"))


def redact_sensitive_value(value: str) -> str:
    token_prefixes = ("sk-clb-", "gho_", "ghp_", "github_pat_", "Bearer ")
    if any(value.startswith(prefix) for prefix in token_prefixes):
        return "<redacted>"
    return value
