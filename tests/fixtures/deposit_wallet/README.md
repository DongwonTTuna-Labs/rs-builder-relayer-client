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
