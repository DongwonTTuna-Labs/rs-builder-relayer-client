# PR 02: Deposit-Wallet HTTP Client

## Summary

Add a mocked HTTP client for relayer nonce, submit, and transaction polling.
This PR connects existing request builders and signing outputs to HTTP behavior
without touching live relayer endpoints.

## In Scope

- Add `src/deposit_wallet/http.rs` for relayer HTTP transport.
- Add a small `DepositWalletRelayerClient` wrapper that uses deposit-wallet
  request builders, signing outputs, and transaction state parsing.
- Implement mocked tests for `GET /nonce`, `POST /submit` with `WALLET-CREATE`,
  `POST /submit` with `WALLET`, and `GET /transaction`.
- Use local test HTTP responders built with `tokio::net::TcpListener`; do not
  add a mock-server dependency unless the PR documents the dependency risk.

## Out Of Scope

- No live network tests in the default suite.
- No pUSD/CTF calldata builders.
- No CLOB order flow.
- No automatic retry after ambiguous submit responses.

## Target API

- `RelayerKeyAuth`: relayer API key identity and credentials, separate from the
  owner signer.
- `DepositWalletRelayerClient`: configured with relayer URL, chain id, owner
  signer, relayer auth, and deposit-wallet contract config.
- `get_wallet_nonce(owner)`: fetches fresh `type=WALLET` nonce.
- `submit_wallet_create(owner)`: submits `WALLET-CREATE`.
- `submit_signed_wallet_batch(batch)`: submits a previously signed `WALLET`
  request.
- `poll_transaction(transaction_id)`: polls until terminal success or terminal
  failure, preserving unknown states.

The implementation PR decides which target APIs are exported. Existing public
APIs must not be removed or silently changed.

## Fixture And Mock Requirements

- Mocked `GET /nonce` must assert `address=<owner>` and `type=WALLET`.
- Mocked `POST /submit` must assert exact JSON body for both `WALLET-CREATE`
  and `WALLET`.
- Mocked polling must cover `STATE_NEW`, `STATE_EXECUTED`, `STATE_MINED`,
  `STATE_CONFIRMED`, `STATE_INVALID`, `STATE_FAILED`, and unknown states.
- Tests must prove relayer auth identity can differ from owner signer identity.
- Ambiguous submit timeout must not create a duplicate submit.

## Validation

- Unit tests for request construction and response parsing.
- Integration-style local HTTP tests with deterministic request/response bodies.
- Standard validation commands from `docs/plans/README.md`.

## Residual Risk

- Mocked HTTP proves client behavior, not production relayer acceptance.
- Live execution remains gated until calldata builders, dry-run evidence, and
  operator approval are complete.
