# PR 02: Deposit-Wallet HTTP Client

## Summary

- Start from latest `main` on `feature/deposit-wallet-http-client`.
- Add a mocked-only deposit-wallet relayer HTTP client for:
  - `GET /nonce`
  - `POST /submit` `WALLET-CREATE`
  - `POST /submit` signed `WALLET`
  - `GET /transaction`
- Keep live execution gated: no default live mutation, no pUSD/CTF calldata
  builders, and no CLOB order flow.

## References

- Deposit Wallets guide:
  <https://docs.polymarket.com/trading/deposit-wallets>
- Relayer submit API:
  <https://docs.polymarket.com/api-reference/relayer/submit-a-transaction>
- Transaction API:
  <https://docs.polymarket.com/api-reference/relayer/get-a-transaction-by-id>
- Nonce API:
  <https://docs.polymarket.com/api-reference/relayer/get-current-nonce-for-a-user>

## Key Changes

- Add deposit-wallet HTTP surface under `src/deposit_wallet/http.rs`.
- Re-export only the controlled safe public types:
  - `DepositWalletRelayerClient`
  - `DepositWalletRelayerUrl`
  - `RelayerKeyAuth`
  - `DepositWalletMutationGate`
  - `DepositWalletMutationPermit`
  - `DepositWalletPollPolicy`
  - `DepositWalletTransactionReceipt`
- `DepositWalletRelayerUrl` rejects non-HTTPS URLs, userinfo, query/fragment
  injection, non-allowlisted hosts, and lookalike hosts. The production
  allowlist is exactly `relayer-v2.polymarket.com`; tests use a `cfg(test)`
  loopback constructor.
- Build the HTTP client with redirects disabled so auth headers are never
  forwarded across redirects.
- Add `RelayerKeyAuth` with private secret-bearing fields and redacted `Debug`.
  Keep existing `AuthMethod` constructors and fields source-compatible while
  redacting `Debug` for existing auth structs.
- Keep `DepositWalletRelayerClient` free of owner private keys. It accepts owner
  addresses, existing request builders, and already validated
  `SignedDepositWalletBatch` values.
- Require `DepositWalletMutationGate::Permit` for submit methods. The default
  gate denies before URL use, auth header construction, or HTTP request
  creation.
- Add a live-wrapper deadline guard for signed WALLET submits using an injected
  clock; expired signed batches fail before auth/HTTP.

## Ambiguous Submit Policy

- Compute a redacted payload hash from the serialized submit body before
  sending.
- If submit times out or returns a success response without a usable
  `transactionID`, record an owner-scoped ambiguous block.
- Same-owner nonce/sign/submit work remains blocked until explicit manual
  reconciliation clears the block.
- 4xx/5xx API errors do not count as success and do not clear existing
  ambiguous blocks.

## Polling Policy

- `STATE_CONFIRMED` is the only terminal success.
- `STATE_INVALID` and `STATE_FAILED` return terminal errors.
- `STATE_NEW`, `STATE_EXECUTED`, and `STATE_MINED` remain pending.
- Unknown states stop polling with a reconciliation-required error, never
  success.
- `DepositWalletPollPolicy` configures max attempts and fixed interval.
- HTTP 429 returns `RelayerError::QuotaExhausted`.

## Test Plan

- Local mocked HTTP tests use `tokio::net::TcpListener`; no mock-server
  dependency.
- Endpoint/auth safety tests reject `http://`, userinfo URLs,
  non-allowlisted hosts, and lookalike hosts.
- Redirect tests prove redirects are not followed and auth is not sent to
  redirect targets.
- Redaction tests prove debug/error paths do not expose raw API keys,
  passphrases, signatures, HMAC material, or auth header values.
- Request/response tests cover exact `GET /nonce`, `WALLET-CREATE` submit
  fixture body, signed `WALLET` submit fixture body, submit response parsing,
  and transaction response parsing.
- Mutation and ambiguity tests cover default-deny, explicit permit, expired
  deadline preflight, partial submit response ambiguity, timeout ambiguity, and
  manual reconciliation clearing.
- Poll tests cover `STATE_NEW`, `STATE_EXECUTED`, `STATE_MINED`,
  `STATE_CONFIRMED`, `STATE_INVALID`, `STATE_FAILED`, unknown state, exact
  attempt counts, and injected sleeper usage.

## Validation

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo build --workspace --all-targets --all-features
git diff --check
```

## Assumptions

- This PR creates mocked HTTP behavior only; it must not run live relayer calls
  in CI.
- Existing public APIs must not be removed or silently changed.
- New deposit-wallet HTTP APIs are introduced as a controlled `0.2.x` surface.
- pUSD/CTF calldata builders remain PR 03, and consumer adapter/live gate
  remains PR 04.
- No PR merge is performed by the agent.
