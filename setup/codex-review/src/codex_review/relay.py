"""OIDC relay contract representation for the v3 workflow."""

from __future__ import annotations

from dataclasses import asdict, dataclass


DEFAULT_RELAY_ARGS_SHA = "c36946ed34d86ecd40b4805e1427c031b34c8a2b5f3018085b45a048e07bcf09"


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


def redact_sensitive_value(value: str) -> str:
    token_prefixes = ("sk-clb-", "gho_", "ghp_", "github_pat_", "Bearer ")
    if any(value.startswith(prefix) for prefix in token_prefixes):
        return "<redacted>"
    return value
