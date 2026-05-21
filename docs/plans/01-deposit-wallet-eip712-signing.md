# PR 01: Deposit-Wallet EIP-712 Signing

## Summary

Add deterministic, fixture-backed DepositWallet Batch EIP-712 signing support.
This PR proves the signed payload shape before any live HTTP client exists.

## In Scope

- Add `src/deposit_wallet/signing.rs` for DepositWallet Batch EIP-712 payload
  construction, digest calculation, signature injection, and signature recovery
  checks.
- Add `src/deposit_wallet/nonce.rs` only for pure `/nonce` request/query shape
  construction.
- Add `tests/fixtures/deposit_wallet/wallet_batch_eip712.json` and
  `wallet_nonce_request.json`.
- Extend `tests/deposit_wallet_test.rs` or add a focused deposit-wallet signing
  test file.

## Out Of Scope

- No live HTTP nonce fetch, submit, or polling.
- No pUSD/CTF calldata builders.
- No consumer adapter wiring.
- No session signer acceptance. This PR only accepts owner-signed batches;
  session signer support requires owner-signed delegation evidence or a
  crate-owned trusted config source in a later PR.
- No production private keys, API credentials, auth headers, or production
  signatures in fixtures, logs, errors, or responses.

## Target API

- `DepositWalletBatchToSign`: owner, deposit wallet, chain id, nonce, deadline,
  and calls.
- `build_deposit_wallet_batch_typed_data(...)`: builds the EIP-712 domain and
  message.
- `digest_deposit_wallet_batch(...)`: returns the EIP-712 digest used for
  signing and fixture comparison.
- `SignedDepositWalletBatch`: owner-validated signed payload with digest,
  signature, owner, nonce owner, submit `from`, deposit wallet, chain id,
  verified signer, nonce, deadline, and calls.
- `build_wallet_nonce_request(owner)`: returns path/query components for
  `GET /nonce?address=<owner>&type=WALLET`.

The names above are target API names for the implementation PR. Final exports
must be reviewed in that PR before becoming public.

## Fixture Requirements

- The EIP-712 domain must match:
  `name = "DepositWallet"`, `version = "1"`, `chainId`, and
  `verifyingContract = deposit_wallet`.
- The typed data must match official TypeScript/Python builder SDK behavior for
  `Call { target, value, data }` and
  `Batch { wallet, nonce, deadline, calls }`.
- The fixture must include expected digest, expected owner signature, recovered
  owner signer address, owner address, nonce owner, submit `from` owner, a
  non-owner/self-asserted signer rejection case, chain id, nonce, deadline,
  deposit wallet, and calls.
- Signature recovery must prove the recovered signer is the owner used for
  `GET /nonce?type=WALLET` and submit `from`. A signature from any other signer,
  including a self-asserted session signer, must fail validation.
- Session signer support must not rely on a fixture or caller claim alone. A
  later PR must include authorization evidence from an owner-signed delegation or
  a trusted config source, plus scope, expiry, and the owner address that
  authorizes the session signer.
- The signed batch result must preserve the checked metadata needed by the HTTP
  client and consumer adapter to re-check owner, submit `from`, nonce owner,
  chain id, deposit wallet/verifying contract, and verified signer before
  submit, without exposing raw typed-data or signatures through `Debug`.
- Any fixture derived from official TypeScript/Python SDK behavior must record
  the SDK name and version or commit SHA used to generate the expected payload.
- Do not commit a private key. If the signature comes from a public test signer,
  record only the signer address, digest, and signature.

## Validation

- Exact JSON equality for the typed-data fixture.
- Digest equality with the official SDK-generated fixture, including SDK
  provenance in the fixture metadata.
- Signature recovery proves the fixture signature signs the expected digest.
- Negative tests prove validation rejects any recovered signer that does not
  match the owner, including self-asserted session signers.
- Submit-preflight tests prove a signed batch cannot be submitted with mismatched
  owner/nonce/submit identity, chain config, or owner/config-derived deposit
  wallet metadata.
- `/nonce` fixture proves `type=WALLET` and the owner address are present.
- Standard validation commands from `docs/plans/README.md`.

## Residual Risk

- This PR proves signing payload parity, not relayer acceptance.
- A real owner signer is not used in CI. Runtime signer integration remains part
  of the HTTP client PR.
