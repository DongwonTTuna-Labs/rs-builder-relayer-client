# Deposit Wallet Fixtures

These fixtures are sanitized and contain no production private keys, API keys,
auth headers, or raw production signatures.

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

The `wallet_nonce_http_request.json`,
`wallet_create_http_submit_request.json`, and
`wallet_signed_http_submit_request.json` fixtures pin the HTTP method,
path/query, and request body for `GET /nonce?type=WALLET`, `POST /submit`
`WALLET-CREATE`, and `POST /submit` `WALLET`.
`transaction_array_response_cases.json` pins transaction-array selection and
error-boundary ids for matching, missing, duplicate, invalid, and oversized
relayer response arrays.
`wallet_nonce_response_cases.json` pins invalid WALLET nonce response boundary
cases. `poll_jitter_vectors.json` pins deterministic retry/backoff jitter
durations and caps.

The `wallet_batch_eip712_multicall.json` and
`wallet_signed_submit_body_multicall.json` fixtures were generated from a
temporary checkout of the official TypeScript relayer SDK at commit
`72886a57116debcbcbf8df43d7f1a53a0f73a771` using
`buildDepositWalletBatchRequest` and `viem` `hashTypedData`. The signer was an
ephemeral synthetic test wallet; its private key is not recorded.
