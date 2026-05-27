# PR 02: Deposit-Wallet HTTP Client

## Summary

Add a guarded relayer HTTP client for validated endpoint handling, transaction
polling diagnostics, and test-loopback nonce/submit request construction for
`WALLET-CREATE` and `WALLET`. This PR connects existing request builders and
signing outputs to relayer HTTP behavior while keeping production WALLET nonce
reads and production submit publicly blocked until a later live-submit PR adds
durable owner state, acceptance evidence, and a trusted mutation capability.

## In Scope

- Add `src/deposit_wallet/http.rs` for relayer HTTP transport.
- Add a small `DepositWalletRelayerClient` wrapper that uses deposit-wallet
  request builders, signing outputs, and transaction state parsing.
- Implement production URL validation for the allowlisted Polymarket relayer,
  read-only `GET /transaction`, and test-loopback `GET /nonce` coverage.
- Implement test-loopback `POST /submit` transport for `WALLET-CREATE` and
  `WALLET` fixtures. Production `POST /submit` remains blocked by public permit
  construction in this PR.
- Implement mocked tests for those endpoints using local loopback responders.
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
- Internal loopback nonce fetch: exercises `GET /nonce?type=WALLET` fixtures
  without exposing a bare public nonce-read API for live signing.
- `submit_wallet_create(owner, mutation_gate)`: submits `WALLET-CREATE` only
  when an explicit mutation gate permits relayer mutation.
- `submit_signed_wallet_batch(batch, mutation_gate)`: submits a previously
  signed `WALLET` request only when an explicit mutation gate permits relayer
  mutation.
- `poll_transaction(transaction_id, poll_policy)`: polls under a bounded policy
  until terminal success or terminal failure, preserving unknown states.

The mutation gate must default to deny relayer mutation. A production URL, API
key, or signer alone must not be enough to read WALLET nonce state or submit
`WALLET-CREATE`/`WALLET`. In this PR, public owner-scoped permits are limited to
test-loopback clients so production `GET /nonce?type=WALLET` and `POST /submit`
cannot be reached through the public API. A later live-submit PR may add a
crate-owned trusted capability for the allowlisted production endpoint only
after durable owner state, dry-run evidence, rollback path, acceptance evidence,
and operator approval are documented and tested. This implementation must
document the permit type and the stable error returned when mutation is blocked.

Relayer authentication headers must only be attached after endpoint validation.
Tests must reject `http://` URLs, non-allowlisted hosts, userinfo-bearing URLs,
host confusion such as suffix/prefix lookalikes, and redirects that would send
credentials to an unapproved origin.

The poll policy must include max attempts or total timeout, initial interval,
backoff or rate-limit handling, and caller cancellation behavior.

The implementation PR decides which target APIs are exported. Existing public
APIs must not be removed or silently changed.

## Fixture And Mock Requirements

- Mock transport is test-only. Default CI must never submit live relayer
  mutations or require live Polymarket credentials.
- Mocked `GET /nonce` must assert `address=<owner>` and `type=WALLET`.
- Endpoint validation tests must prove relayer auth is never sent to
  non-HTTPS, non-allowlisted, userinfo-bearing, or redirect targets.
- Relayer auth redaction tests must prove `RelayerKeyAuth` and related errors
  do not expose raw API keys, bearer values, auth headers, HMAC material, or
  credential-derived strings through `Debug`, `Display`, error conversion,
  logs, snapshots, or fixture output.
- Mutation gate tests must prove both `submit_wallet_create` and
  `submit_signed_wallet_batch` are denied by default before any HTTP request or
  auth header construction, return a stable blocked-mutation error, and proceed
  only when an explicit owner-scoped permit is supplied.
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

- Mocked default tests prove client behavior, not production relayer acceptance.
- Production submit request construction is covered by test-loopback fixtures,
  but production `POST /submit` is not publicly reachable in this PR. End-to-end
  live trading remains blocked until durable owner state, calldata builders,
  dry-run evidence, rollback path, and operator approval are complete.
