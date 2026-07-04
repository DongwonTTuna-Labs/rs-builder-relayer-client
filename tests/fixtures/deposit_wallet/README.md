# Deposit Wallet Fixtures

These fixtures are sanitized and contain no production private keys, API keys,
auth headers, or raw production signatures.

Canonical fixture provenance is tracked in `PROVENANCE.md`. That ledger is the
associated provenance record for JSON fixtures that do not carry inline
`metadata`, and it records whether each fixture is official-source authority,
sanitized offline regression evidence, or local synthetic negative coverage.

Sources checked while creating these fixtures:

- Polymarket deposit wallet guide:
  `https://docs.polymarket.com/trading/deposit-wallets`
- Polymarket relayer submit API:
  `https://docs.polymarket.com/api-reference/relayer/submit-a-transaction`
- Polymarket relayer transaction API:
  `https://docs.polymarket.com/api-reference/relayer/get-a-transaction-by-id`
- Polymarket Python relayer client main:
  `6589f36740a2a98736f5b0499f29d2d234c583bc`
- Polymarket TypeScript relayer client main:
  `72886a57116debcbcbf8df43d7f1a53a0f73a771`

The `wallet_submit_body.json` signature is a synthetic 65-byte fixture string,
not a live or production signature.

The `wallet_batch_eip712_multicall.json` and
`wallet_signed_submit_body_multicall.json` fixtures were generated from a
temporary checkout of the official TypeScript relayer SDK at commit
`72886a57116debcbcbf8df43d7f1a53a0f73a771` using
`buildDepositWalletBatchRequest` and `viem` `hashTypedData`. The signer was an
ephemeral synthetic test wallet; its private key is not recorded.

PBRSDK-2 source matrix:
`../../../docs/DEPOSIT_WALLET_SOURCE_MATRIX.md`

Current source refresh recorded by PBRSDK-2:

- TypeScript relayer SDK `@polymarket/builder-relayer-client` `0.0.10`,
  commit `9122f6fb1856f1ecfe4406685bfa19a2c5a7b290`.
- Python relayer SDK `py-builder-relayer-client` `0.0.2`,
  commit `267a36d84d7839b6e4ac134297d9230fc224cf8f`.
- Rust CLOB SDK `polymarket_client_sdk_v2`,
  commit `3ae1aae5e9ded38f984464c9fc0f307f8a9f41fb`.

These fixtures remain offline regression evidence. They do not authorize live
relayer mutation, wallet deployment, order placement, or use of real credentials.
