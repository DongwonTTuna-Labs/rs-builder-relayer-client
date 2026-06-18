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
  owner signer. Secret material must be wrapped so `Debug`, errors, logs,
  snapshots, fixtures, and test failure output expose only redacted identity
  metadata, never raw API keys, bearer values, auth headers, or derived signing
  material.
- `DepositWalletRelayerClient`: configured with relayer URL, chain id, owner
  signer, relayer auth, and deposit-wallet contract config. The relayer URL
  must be a validated endpoint newtype, not an arbitrary string. It must require
  HTTPS and an approved Polymarket relayer host allowlist before any request can
  attach relayer authentication headers.
- `get_wallet_nonce(owner)`: fetches fresh `type=WALLET` nonce.
- `submit_wallet_create(owner, mutation_gate)`: submits `WALLET-CREATE` only
  when an explicit mutation gate permits relayer mutation.
- `get_wallet_nonce_with_lease(owner, mutation_gate)`: fetches a fresh
  `type=WALLET` nonce and keeps the owner-scoped lease alive for the matching
  signed submit.
- `sign_and_submit_wallet_batch_with_nonce_lease(nonce_lease, mutation_gate,
  sign)`: gives the caller a lease-bound signing context, then submits the
  signed `WALLET` request only when an explicit mutation gate permits relayer
  mutation and the crate-owned nonce lease matches the signed owner and nonce.
  The sign callback must produce the returned signed batch through
  `DepositWalletNonceLeaseSigningContext::validate_batch_signature`, which
  constructs the EIP-712 payload from the current lease owner, nonce owner,
  submit-from address, deposit wallet, chain id, and nonce, then verifies the
  raw owner signature before adding the private lease binding. A pre-existing
  `SignedDepositWalletBatch`, including an older same-nonce batch, is not
  accepted under a fresh lease.
- `poll_transaction(transaction_id, poll_policy)`: polls under a bounded policy
  until terminal success or terminal failure, preserving unknown states.

The mutation gate must default to deny live relayer mutation. A live URL, API
key, or signer alone must not be enough to submit `WALLET-CREATE` or `WALLET`.
The implementation PR must document the enable flag or permit type, dry-run
evidence requirement, rollback path, and the error returned when mutation is
blocked.

Relayer authentication headers must only be attached after endpoint validation.
Tests must reject `http://` URLs, non-allowlisted hosts, userinfo-bearing URLs,
host confusion such as suffix/prefix lookalikes, and redirects that would send
credentials to an unapproved origin.

The poll policy must include max attempts or total timeout, initial interval,
backoff or rate-limit handling, and caller cancellation behavior.

The implementation PR decides which target APIs are exported. Existing public
APIs must not be removed or silently changed.

## Fixture And Mock Requirements

- Mocked `GET /nonce` must assert `address=<owner>` and `type=WALLET`.
- Endpoint validation tests must prove relayer auth is never sent to
  non-HTTPS, non-allowlisted, userinfo-bearing, or redirect targets.
- Relayer auth redaction tests must prove `RelayerKeyAuth` and related errors
  do not expose raw API keys, bearer values, auth headers, HMAC material, or
  credential-derived strings through `Debug`, `Display`, error conversion,
  logs, snapshots, or fixture output.
- Mutation gate tests must prove both `submit_wallet_create` and
  `sign_and_submit_wallet_batch_with_nonce_lease` are denied by default before
  any HTTP request or auth header construction, return a stable blocked-mutation
  error, and proceed only when an explicit permit and matching crate-owned nonce
  lease are supplied.
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
- Submit timeouts before a `transactionID` is known must be treated as
  owner-scoped ambiguous mutation. Tests must prove the client records a
  redacted payload hash or equivalent idempotency evidence, blocks additional
  same-owner nonce fetch/sign/submit work, and requires manual or authoritative
  reconciliation before the owner can submit again.
- Tests must prove relayer auth identity can differ from owner signer identity.
- Ambiguous submit timeout must not create a duplicate submit.

## Validation

- Unit tests for request construction and response parsing.
- Unit tests for relayer auth redaction and mutation gate default-deny behavior.
- Integration-style local HTTP tests with deterministic request/response bodies.
- Polling tests cover timeout/max-attempt exhaustion, backoff or rate-limit
  behavior, cancellation, and no duplicate submit after ambiguous responses.
- Polling timeout, backoff, and cancellation tests must use deterministic time,
  such as `tokio::time::pause`/`advance` or an injected clock/sleeper. They must
  assert exact attempt counts and poll intervals without real sleeps or wall
  clock timing.
- Standard validation commands from `docs/plans/README.md`.

## Residual Risk

- Mocked HTTP proves client behavior, not production relayer acceptance.
- Live execution remains gated until calldata builders, dry-run evidence, and
  operator approval are complete.
