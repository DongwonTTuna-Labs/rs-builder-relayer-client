import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "src"))

from codex_review.relay import DEFAULT_RELAY_ARGS_SHA, default_relay_contract, normalize_codex_args, redact_sensitive_value


class RelayContractTests(unittest.TestCase):
    def test_default_contract_matches_current_oidc_action(self):
        contract = default_relay_contract()

        self.assertEqual("DongwonTTuna-Labs/home-server-infra/.github/actions/setup-codex-relay@main", contract.action)
        self.assertEqual("https://relay-ai.dongwontuna.net/github-actions", contract.audience)
        self.assertEqual("https://relay-ai.dongwontuna.net/v1/oidc/token", contract.broker_url)
        self.assertEqual("AI_RELAY_API_KEY", contract.relay_env_key)
        self.assertEqual(DEFAULT_RELAY_ARGS_SHA, contract.codex_args_sha256)

    def test_normalize_codex_args_removes_legacy_landlock_flag(self):
        raw = '["--enable","use_legacy_landlock","--ignore-user-config","-c","model_provider=\\"ai-relay\\""]'

        normalized = normalize_codex_args(raw)

        self.assertEqual('["--ignore-user-config","-c","model_provider=\\"ai-relay\\""]', normalized)

    def test_normalize_codex_args_preserves_non_legacy_feature_flags(self):
        raw = '["--enable","other_feature","--ignore-rules"]'

        normalized = normalize_codex_args(raw)

        self.assertEqual('["--enable","other_feature","--ignore-rules"]', normalized)

    def test_normalize_codex_args_requires_json_string_array(self):
        with self.assertRaises(ValueError) as ctx:
            normalize_codex_args('{"args":[]}')

        self.assertIn("codex args must be a JSON string array", str(ctx.exception))

    def test_redact_sensitive_value_masks_tokens(self):
        self.assertEqual("<redacted>", redact_sensitive_value("sk-clb-secret"))
        self.assertEqual("<redacted>", redact_sensitive_value("gho_secret"))
        self.assertEqual("<redacted>", redact_sensitive_value("github_pat_secret"))

    def test_redact_sensitive_value_leaves_non_tokens(self):
        self.assertEqual("plain-value", redact_sensitive_value("plain-value"))


if __name__ == "__main__":
    unittest.main()
