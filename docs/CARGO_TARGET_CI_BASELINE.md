# Cargo Target And CI Baseline

Retrieval date: 2026-07-04

This audit records the PBRSDK-3 target graph and PR validation gate. It is an
offline validation baseline only. It does not authorize live relayer mutation,
wallet deployment, production signing, order placement, trading behavior,
production credentials, private endpoints, funded-wallet data, or replayable
submit bodies.

## Cargo Target Audit

| Target kind | Name | Declared path | Tracked file status | Resolution |
|---|---|---|---|---|
| lib | `polymarket_relayer` | `src/lib.rs` | tracked | keep |
| example | `setup_wallet` | `examples/setup_wallet.rs` | tracked | keep offline example |
| example | `redeem_single` | `examples/redeem_single.rs` | tracked | keep offline example |
| example | `redeem_all` | `examples/redeem_all.rs` | tracked | keep offline example |
| example | `split_merge` | `examples/split_merge.rs` | tracked | keep offline example |
| example | `redeem_magic` | `examples/redeem_magic.rs` | tracked | keep offline example |
| example | `diagnose_gs026` | `examples/diagnose_gs026.rs` | tracked | keep offline example |
| example | `diagnose_nonce` | `examples/diagnose_nonce.rs` | tracked | keep offline example |
| integration test | `auth_test` | `tests/auth_test.rs` | tracked | keep |
| integration test | `builder_test` | `tests/builder_test.rs` | tracked | keep |
| integration test | `client_test` | `tests/client_test.rs` | tracked | keep |
| integration test | `deposit_wallet_signing_test` | `tests/deposit_wallet_signing_test.rs` | tracked | keep |
| integration test | `deposit_wallet_test` | `tests/deposit_wallet_test.rs` | tracked | keep |
| integration test | `integration_test` | `tests/integration_test.rs` | tracked | keep |
| integration test | `operations_test` | `tests/operations_test.rs` | tracked | keep |
| integration test | `source_matrix_test` | `tests/source_matrix_test.rs` | tracked | keep |

No stale declared Cargo target was found in the current baseline. If a future
target is added, it must map to a tracked file or document a restore, removal,
or feature-gate decision before the task can be considered complete.

## CI Gate

`.github/workflows/rust-validation.yml` runs on pull request open, ready for
review, synchronize, and reopen events. It has read-only repository permission
and does not reference GitHub secrets.

The workflow enforces:

- `cargo fmt --all --check`
- `git diff --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --all-features`
- `cargo build --workspace --all-targets --all-features`

The existing Grimoire workflow remains separate. Rust validation does not
depend on Grimoire secrets, live relayer credentials, private endpoints,
production wallets, signing keys, or trading credentials.

## Evidence Requirements

Implementation and review packets should include:

- this target audit,
- final local command output or equivalent CI evidence,
- PR workflow diff evidence,
- `git diff --check` output,
- confirmation that logs and artifacts contain no secrets, auth headers, raw
  production signatures, private endpoints, funded-wallet data, or replayable
  production submit bodies,
- confirmation that PBRSDK-2 source matrix and fixture provenance files remain
  present and live behavior remains gated.
