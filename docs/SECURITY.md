# SECURITY.md

## Scope

This file covers security rules for this forked Polymarket deposit-wallet relayer crate.

## Secret Handling

Never log or commit:

```text
PRIVATE_KEY
API_SECRET
API_PASSPHRASE
BUILDER_SECRET
BUILDER_PASS_PHRASE
RELAYER_API_KEY
raw auth headers
raw production signatures
```

Secret-bearing structs must not derive `Debug` unless every secret field is redacted.

### Legacy Auth Compatibility

`AuthMethod::RelayerKey` and `BuilderConfig` still expose public `String`
fields for backwards compatibility with existing consumers that construct these
types directly. Treat those fields as secret-bearing legacy API: do not log,
snapshot, or broadly clone them. The crate redacts `Debug`, marks relayer auth
headers sensitive, and returns generic secret-parse errors, but the public field
surface remains an accepted compatibility risk for this non-breaking PR.

Migration path: a future breaking auth cleanup should replace the public raw
fields with private secret wrappers or add new secret-wrapper constructors, then
document consumer changes before removing the legacy fields.

## Signing Safety

Order signing is not this crate's responsibility. This crate is responsible for relayer auth and deposit-wallet batch signing only.

Required:

- typed signer/auth config;
- no private key debug output;
- golden tests for DepositWallet EIP-712 payloads;
- request serialization tests before any live path;
- explicit chain id and verifying contract inputs.

## Relayer Identity Separation

Never collapse these identities:

```text
RELAYER_API_KEY_ADDRESS:
  credential/auth identity used in relayer auth headers

DEPOSIT_WALLET_OWNER_ADDRESS:
  owner/session signer used as /submit `from` and EIP-712 signer

DEPOSIT_WALLET_ADDRESS:
  smart contract wallet/funder that holds pUSD and conditional tokens
```

Equality may happen in one deployment, but it must never be an implicit code assumption.

## Supply-Chain Policy

This fork is internal critical infrastructure.

Allowed:

- local path dependency during development;
- pinned git `rev` dependency for production after review.

Forbidden for production:

- direct upstream `OrderBookTrade/rs-builder-relayer-client` dependency;
- `branch = "main"`;
- versionless git dependency;
- crates.io publishing unless explicitly re-approved.

Run `cargo audit` / `cargo deny` where practical before production import.

## Logging Policy

Logs may include:

```text
safe endpoint name
HTTP status
sanitized relayer transaction id
transaction state
redacted wallet address
redacted call summary
payload hash
```

Logs may not include:

```text
private key
secret
passphrase
auth header
full production signature material
raw signed payload from a production account
```

## Mutation Observability Contract

The `polymarket_relayer::mutation_intent` tracing target has a closed field
contract. Every event contains only the shortened owner, chain id, and intent
epoch as common correlation fields. Depending on the event, the only permitted
additional fields are the operation/status/decision enum Debug value and a
`sha3:0x...` sanitized transaction id. Begin, blocked, submitted, ambiguous,
resolved, abandoned, manual-reconciliation, and adoption events must not grow
additional fields without a new reviewed observability decision.

Never emit any of these values through tracing, logs, Debug, Display, errors,
snapshots, or ticket artifacts:

```text
raw API key or auth header
private key or signer backend error/source text
raw production signature
raw signed typed data
full calldata or replayable submit body
operator reconciliation reference or summary text
unrecognized provider/caller state text
```

Nonce is intentionally absent from tracing even though it is allowed in the
versioned audit artifact. Revision and payload hash are safe upper-bound values
but are not fields in the schema-v1 mutation-intent events. Raw transaction ids
are also forbidden in tracing; they remain only in the protected durable record
and the serialized audit artifact where an operator needs them for
adoption/polling.

`MutationIntentAuditArtifact` is the only ticket/PR attachment form for the
persisted lifecycle. It uses a shortened owner, sanitizes unknown state labels,
and replaces `ReconciliationEvidence` free text with decision, stored UTF-8 byte
lengths, and recorded time. Its manual Debug additionally hashes the transaction
id. Do not attach `MutationIntentRecord` when original reconciliation text is
present; operators retrieve that text directly from their protected store.

`DepositWalletDryRunEvidence` is the complementary pre-submit artifact and may
contain selector/call summaries. `MutationIntentAuditArtifact` deliberately
does not. Their `payload_keccak256` values correlate the review and lifecycle
without logging the signed payload or full body.

## Relayer-Specific Risks

Relayer operations mutate on-chain state.

Rules:

- nonce handling must be explicit and fresh;
- transaction polling must handle unknown states;
- ambiguous submit failures must not blindly retry;
- WALLET batch contents must be event-loggable in redacted form;
- pUSD/CTF calldata builders require golden tests before live use.

## Incident Response

If a secret leaks:

```text
1. Stop live runtime that consumes this crate.
2. Rotate relayer/API credentials.
3. Rotate wallet if private key leaked.
4. Revoke or adjust approvals if needed.
5. Remove secret from repo/history if committed.
6. Review logs/artifacts/CI output.
```

If a relayer transaction has unknown state:

```text
1. Stop new relayer mutation.
2. Persist transaction id and payload hash.
3. Poll/reconcile wallet and account state.
4. Decide on any new signed batch only after reconciliation.
```
