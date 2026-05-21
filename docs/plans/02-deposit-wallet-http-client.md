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
- `submit_wallet_create(owner, mutation_gate)`: submits `WALLET-CREATE` only
  when an explicit mutation gate permits relayer mutation.
- `submit_signed_wallet_batch(batch, mutation_gate)`: submits a previously
  signed `WALLET` request only when an explicit mutation gate permits relayer
  mutation.
- `poll_transaction(transaction_id, poll_policy)`: polls under a bounded policy
  until terminal success or terminal failure, preserving unknown states.

The mutation gate must default to deny live relayer mutation. A live URL, API
key, or signer alone must not be enough to submit `WALLET-CREATE` or `WALLET`.
The implementation PR must document the enable flag or permit type, dry-run
evidence requirement, rollback path, and the error returned when mutation is
blocked.

The poll policy must include max attempts or total timeout, initial interval,
backoff or rate-limit handling, and caller cancellation behavior.

The implementation PR decides which target APIs are exported. Existing public
APIs must not be removed or silently changed.

## Fixture And Mock Requirements

- Mocked `GET /nonce` must assert `address=<owner>` and `type=WALLET`.
- Mocked `POST /submit` must assert exact JSON body for both `WALLET-CREATE`
  and `WALLET`.
- Mocked polling must cover `STATE_NEW`, `STATE_EXECUTED`, `STATE_MINED`,
  `STATE_CONFIRMED`, `STATE_INVALID`, `STATE_FAILED`, and unknown states with
  exact assertions for parsed state, terminal status, success status, and the
  client action for each state.
- `STATE_NEW`, `STATE_EXECUTED`, and `STATE_MINED` must remain pending and
  non-success under the bounded poll policy. `STATE_CONFIRMED` is the only
  terminal success state for deposit-wallet readiness.
- `STATE_INVALID` and `STATE_FAILED` must be terminal failures.
- Unknown states and partial submit responses must not be treated as success and
  must not trigger duplicate submit. They must stop mutation and require
  reconciliation evidence before any new submit.
- Tests must prove relayer auth identity can differ from owner signer identity.
- Ambiguous submit timeout must not create a duplicate submit.

## Validation

- Unit tests for request construction and response parsing.
- Integration-style local HTTP tests with deterministic request/response bodies.
- Polling tests cover timeout/max-attempt exhaustion, backoff or rate-limit
  behavior, cancellation, and no duplicate submit after ambiguous responses.
- Standard validation commands from `docs/plans/README.md`.

## Residual Risk

- Mocked HTTP proves client behavior, not production relayer acceptance.
- Live execution remains gated until calldata builders, dry-run evidence, and
  operator approval are complete.
