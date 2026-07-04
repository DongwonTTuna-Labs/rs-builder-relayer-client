# Deposit Wallet Fixtures

These fixtures are sanitized and contain no production private keys, API keys, auth header values, seed phrases, funded-wallet data, or raw production signatures.

Canonical fixture provenance is pinned in `docs/DEPOSIT_WALLET_SOURCE_MATRIX.json` under `fixturePathAudit` and `fixtureProvenance`. PBRSDK-2 audited the existing `tests/fixtures/deposit_wallet/` path on 2026-07-04; the path is present, so absent-path handling is not applicable.

Selected official source pins for this audit:

- TypeScript relayer SDK: `Polymarket/builder-relayer-client` commit `9122f6fb1856f1ecfe4406685bfa19a2c5a7b290`, package `@polymarket/builder-relayer-client` version `0.0.10`.
- Python relayer SDK: `Polymarket/py-builder-relayer-client` commit `267a36d84d7839b6e4ac134297d9230fc224cf8f`, package `py_builder_relayer_client` version `0.0.2`.
- Rust CLOB SDK: `Polymarket/rs-clob-client-v2` commit `3ae1aae5e9ded38f984464c9fc0f307f8a9f41fb`, package `polymarket_client_sdk_v2` version `0.6.0`.

Relevant official relayer SDK deposit-wallet source files were compared with the earlier fixture-generation commits. The TypeScript deposit-wallet builder/types/constants files are unchanged for the relevant wire and EIP-712 shapes; the TypeScript config diff is newline-only. The Python deposit-wallet builder/models/config/constants files are unchanged for the relevant wire and EIP-712 shapes. Therefore no fixture JSON content was regenerated in PBRSDK-2.

## Fixture authority summary

| Fixture | Decision | Authority status | Notes |
| --- | --- | --- | --- |
| `derive_address.json` | current | official_sdk_commit_authoritative | derive/config fixture; old and selected current SDK derive/config files were compared and relevant content is unchanged |
| `transaction_array_response_cases.json` | current | sanitized_local_fixture_authoritative_for_parser_boundaries_only | synthetic array-selection and rejection cases; not live response evidence |
| `wallet_batch_eip712.json` | current | sanitized_fixture_authoritative_for_offline_eip712_digest_and_recovery_tests | generated from official SDK shape; selected current SDK typed-data files are unchanged from fixture commit |
| `wallet_batch_eip712_amoy.json` | current | sanitized_local_fixture_authoritative_for_Amoy_config_edge_test_only | synthetic one-off signer; not live authority |
| `wallet_batch_eip712_multicall.json` | current | sanitized_fixture_authoritative_for_offline_multicall_eip712_tests | generated from official TypeScript SDK shape; selected current SDK typed-data files are unchanged from fixture commit |
| `wallet_batch_unsupported_chain_eip712.json` | current | sanitized_local_fixture_authoritative_for_negative_validation_only | synthetic unsupported-chain validation fixture; not live authority |
| `wallet_batch_wrong_wallet_eip712.json` | current | sanitized_local_fixture_authoritative_for_negative_validation_only | synthetic wrong-wallet validation fixture; not live authority |
| `wallet_create_submit_body.json` | current | official_sdk_commit_authoritative | WALLET-CREATE serialization shape agrees with official docs and SDK |
| `wallet_nonce_http_request.json` | current | official_sdk_commit_authoritative_for_WALLET_nonce_request | documents GET /nonce request path/query and parser response shape offline |
| `wallet_nonce_request.json` | current | official_sdk_commit_authoritative_for_WALLET_nonce_request | documents owner address and type=WALLET request fixture |
| `wallet_nonce_response_cases.json` | current | sanitized_local_fixture_authoritative_for_parser_boundaries_only | negative/invalid nonce parser cases; not live success evidence |
| `wallet_signed_submit_body.json` | current | sanitized_fixture_authoritative_for_offline_submit_serialization_only | synthetic signature; official SDK wire shape unchanged |
| `wallet_signed_submit_body_amoy.json` | current | sanitized_local_fixture_authoritative_for_Amoy_submit_edge_test_only | synthetic signature and Amoy config fixture; not live authority |
| `wallet_signed_submit_body_multicall.json` | current | sanitized_fixture_authoritative_for_offline_multicall_submit_serialization_only | synthetic signature; official SDK wire shape unchanged |
| `wallet_submit_body.json` | current | sanitized_fixture_authoritative_for_offline_submit_serialization_only | legacy synthetic signature fixture retained for serialization guard; not live authority |
| `wallet_transaction_response.json` | current_non_authoritative_for_live | sanitized_local_fixture_authoritative_for_parser_shape_only | sanitized WALLET-shaped transaction response fixture; official API enum still lists SAFE/PROXY only, so live WALLET polling remains blocked |

The `wallet_submit_body.json`, `wallet_signed_submit_body*.json`, and EIP-712 signature fields are synthetic offline fixture values. They are not live or production signatures and must not be used as replayable relayer payloads.

The `wallet_transaction_response.json` fixture remains parser-shape evidence only. It does not authorize live WALLET polling because the official transaction API reference still documents a SAFE/PROXY-oriented example and enum while the deposit-wallet guide instructs polling WALLET submissions.
