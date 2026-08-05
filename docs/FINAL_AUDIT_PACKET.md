# FINAL_AUDIT_PACKET.md

This is the PBRSDK-30 audit packet. It links each non-negotiable requirement to
the evidence that supports it, records the validation results at the audited
commit, and states what the evidence does not establish.

It does not declare the fork production-ready. Readiness is a decision an
operator makes from evidence, and the last requirement below is one this packet
deliberately leaves open.

## Audited state

| What | Value |
| --- | --- |
| Repository | `DongwonTTuna-Labs/rs-builder-relayer-client` |
| Audited commit | recorded below, after this file stops changing |
| Branch | `don-75-pbrsdk-30-final-audit` |
| Upstream | `OrderBookTrade/rs-builder-relayer-client` at `a2306c8`, merge-base `521ab0b` |
| Delivery | 24 open pull requests, #138 through #161. #138 and #140 through #161 form one stack, each based on the previous; #139 is a second PBRSDK-2 pull request against `main` |
| Consumer | `DongwonTTuna-Labs/polymarket-liquidity-farming-rs`, draft pull requests #18, #19, #20 |

All 24 fork pull requests and all 3 consumer pull requests are open and
unmerged. Every one is a draft except #139. No agent merged a pull request at
any point in this work.

Other open pull requests exist in this repository from work that predates this
campaign. They are outside this audit.

## Ticket coverage

| Ticket | Subject | Delivered by | Status |
| --- | --- | --- | --- |
| PBRSDK-1..4 | Provenance, baseline, Cargo target graph, public API boundary | #138 and #139 (PBRSDK-2), #140 (PBRSDK-3), #141 (PBRSDK-4) | Done |
| PBRSDK-6 | Relayer HTTP transport as a production-safe surface | #142 | In review |
| PBRSDK-7 | Explicit mutation permit and dry-run gate | #143 | In review |
| PBRSDK-8 | Deposit-wallet deployment lifecycle | #144 | In review |
| PBRSDK-9 | Fresh-nonce `WALLET` batch execution | #145 | In review |
| PBRSDK-10 | Bounded polling and transaction state policy | #146 | In review |
| PBRSDK-12 | Owner-scoped nonce lease and mutation intent state | #147 | In review |
| PBRSDK-13 | Ambiguous submit recovery and reconciliation gate | #148 | In review |
| PBRSDK-14 | Deterministic owner concurrency tests | #149 | In review |
| PBRSDK-15 | Redacted mutation evidence and observability contract | #150 | In review |
| PBRSDK-17 | Verified calldata configuration and allowlist | #151 | In review |
| PBRSDK-18 | pUSD and CTF approval calldata builders | #152 | In review |
| PBRSDK-19 | CTF split, merge, and redeem adapter routes | #153 | In review |
| PBRSDK-20 | `WALLET` batch composition tests | #154 | In review |
| PBRSDK-22 | Consumer `RelayerPort` adapter boundary | #155 (fork), consumer #18 | In review |
| PBRSDK-23 | CLOB deposit-wallet funder separation | #156 (fork), consumer #19 | In review |
| PBRSDK-24 | End-to-end consumer dry-run evidence | consumer #20 | In review |
| PBRSDK-26 | Manual live gate runbook with stop conditions | #158 | In review |
| PBRSDK-27 | Operator-approved tiny-value live validation | #159 | In review |
| PBRSDK-28 | Release provenance, dependency pin, supply-chain review | #160 | In review |
| PBRSDK-29 | Documentation at final feature state | #161 | In review |
| PBRSDK-30 | This packet | #162 | In review |

One defect found mid-campaign was fixed on its own branch rather than folded
into an unrelated ticket: #157 removed a dry-run evidence field that exposed the
full four-byte calldata selector.

### The completion criterion this packet does not satisfy

PBRSDK-30 asks that every ticket be Done. A ticket cannot reach Done while its
pull request is unmerged, and merging is prohibited for the agent that wrote the
change. The prohibition wins. This packet therefore records merge-pending as
blocked evidence rather than reporting a completion that did not happen:

> Every ticket above has its work delivered, verified, and reviewed. None is
> merged. What remains is a human decision on 24 fork pull requests and 3
> consumer pull requests.

## Non-negotiable requirements and their evidence

| Requirement | Evidence |
| --- | --- |
| No agent merges a pull request | All 27 pull requests are open and unmerged; 26 are drafts, and `gh pr list --state all` reports 0 merged in the campaign range |
| No production credential in the committed tree | Secret scan below; `src/` makes no `env::var` call; `Debug` redaction is asserted by `public_observability_debug_and_artifact_json_omit_all_secret_sentinels`, `mutation_intent_serialization_and_debug_are_secret_free_and_redacted`, and `dry_run_and_permit_debug_redact_replayable_and_authorization_material` in `src/deposit_wallet/http/tests.rs`, plus `test_builder_config_debug_redacts_all_secret_fields` in `tests/auth_test.rs`. The identity types additionally carry no `Display`, `Serialize`, `Deserialize`, or `Deref`, asserted at compile time in `tests/public_api_boundary_test.rs` |
| Production dependency pinned by commit SHA, never by branch | `docs/CONSUMER_INTEGRATION.md` and `docs/PUBLISHING_DISABLED.md` require it; the consumer pins `rev = "c72dd58..."`; `scripts/preflight_build_integrity.py` rejects `git`, `path`, and registry-index dependency keys in this manifest |
| Signer, relayer auth, and deposit-wallet identity carry distinct types | Three newtypes -- `RelayerAuthIdentity`, `DepositWalletOwner`, `DepositWalletAddress` -- with private fields and compile-time assertions that they gain no `Display`, `Serialize`, `Deserialize`, or `Deref` (`tests/public_api_boundary_test.rs`). `DepositWalletIdentityConfig`, `IdentityOverlap`, and `IdentityConfigSummary` complete the surface but are not newtypes. This is role typing, not value separation: see the limits below |
| No live claim before the gate | `docs/LIVE_VALIDATION_DECISION.md` marker is `blocked`, audited by `tests/live_validation_decision_test.rs`; README feature-status table states live validation was not performed |
| Wire format compared against docs or an official SDK | `docs/DEPOSIT_WALLET_SOURCE_MATRIX.md` and `tests/fixtures/deposit_wallet/PROVENANCE.md` pin the source for every fixture |
| Unknown, ambiguous, or partial responses are not treated as success | Ambiguity produces an `AmbiguousCandidateReport`, never a success outcome; the type and its getters are pinned in `tests/public_api_boundary_test.rs` and exercised in `src/deposit_wallet/http/tests.rs` |
| No CLOB implementation in this crate | `tests/no_clob_surface_test.rs`, 24 tests; see below |
| Legacy Safe/Proxy behaviour not silently reused for the deposit-wallet flow | See below |
| Fixture test for every signing, calldata, or serialization change | `tests/deposit_wallet_signing_test.rs`, `tests/calldata_batch_composition_test.rs`, and the golden calldata fixtures |
| Live-capable change requires an enable flag, dry-run evidence, and a rollback path | Mutation gate defaults to deny; `RelayerMutationMode::DryRun`; `disable_mutation` is one-way for the client and its clones |
| Dependency bump checked beyond the lockfile | `docs/RELEASE_PROVENANCE.md` dependency posture section records the `ethers` default-features change and the TLS backend it required |
| Accepted advisories documented with a re-review condition | `docs/accepted-advisories.toml`, enforced against `.cargo/audit.toml` by the preflight and by `tests/release_provenance_test.rs` |

## Validation at the audited commit

A packet cannot name the commit that contains it. The line below is filled in
by the last commit on this branch, and the results under it were produced at
that commit; until it names a forty-character SHA, this packet is a draft and
not audit evidence.

    audited-commit: PENDING

### Crate

| Command | Result |
| --- | --- |
| `python3 -I scripts/preflight_build_integrity.py` | exit 0 |
| `cargo test --workspace --all-features` | 508 passed, 0 test binaries failed |
| `cargo audit --deny warnings` | exit 0 |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | 0 warnings |
| `cargo fmt --all --check` | exit 0, and no evidence: `rustfmt.toml` sets `disable_all_formatting` |
| `cargo doc --workspace --all-features --no-deps` | 0 warnings |
| `git diff --check` | clean |

### Consumer

Run in an isolated git worktree. The developer's own working tree was not
touched.

| Command | Result |
| --- | --- |
| `cargo fmt --all --check` | exit 0, and no evidence: `rustfmt.toml` sets `disable_all_formatting` |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | 0 warnings |
| `cargo test --workspace --all-features` | 37 passed, 0 failed |

This run pins the fork at `c72dd58`, the PBRSDK-22a commit, not the commit
audited above. PBRSDK-30 asks for consumer validation at the final commit, and
that is not what this shows. Re-pinning is a change to the consumer repository,
which an agent may propose but not merge, so the gap is recorded rather than
closed: the consumer's own pull request has to move to the fork commit a human
decides to merge.

## Secret scan

No value is reproduced here.

| Pattern | Result |
| --- | --- |
| 65-byte signature shape (`0x` + 130 hex) | 32 occurrences: 31 across 15 fixture files, every one declared synthetic in `tests/fixtures/deposit_wallet/PROVENANCE.md`, and one repeating `0x1111...` placeholder in `src/deposit_wallet/requests.rs` |
| 32-byte key shape (`0x` + 64 hex) | Fixtures, plus the two CREATE2 init-code hashes in `src/contracts.rs` and repeating test patterns in `src/deposit_wallet/calldata/ctf.rs` |
| Bearer token, API key, or secret literal | none |
| `env::var` in `src/` | none |

Every host that appears in the repository is a public Polymarket endpoint, a
public documentation site, a public RPC provider, a loopback address, or an
RFC-reserved example address, with one exception: `Cargo.toml` declares
`repository = "https://git.dongwontuna.net/..."`, a self-hosted git mirror.
It carries no credential and the crate sets `publish = false`, so the name is
never published to a registry. It is recorded here rather than changed, because
`Cargo.toml` is schema-pinned by the preflight and the choice belongs to the
repository owner.

## CLOB and legacy separation

`tests/no_clob_surface_test.rs` recursively audits every `src/**/*.rs` file for
CLOB markers and function declarations, with only the path- and count-pinned
exceptions that ADR-0021 documents. It passes with 24 tests.

The boundary grep in `docs/REVIEW_CHECKLIST.md` is not expected to return
nothing. Its hits are `try_build_wallet_batch_request_with_signature`, the
validating builder, and `DepositWalletBatchRequest`, a validated output type
whose fields are private. The unvalidated
`build_wallet_batch_request_with_signature` is absent from the public surface.

`SAFE_INIT_CODE_HASH` and `PROXY_INIT_CODE_HASH` are read only by
`src/builder/derive.rs`, the legacy derivation path. No file under
`src/deposit_wallet/` reads either, so Safe and Proxy derivation does not reach
the deposit-wallet flow.

## Live mutation

Live validation was not performed. `docs/LIVE_VALIDATION_DECISION.md` records
the status as `blocked`, its fourteen evidence rows are unfilled, and no relayer
mutation request has been sent from this fork.

The reason is recorded rather than implied: the gate requires an operator
decision, funded credentials, and an accepted rollback plan, and none of those
is an agent's to supply. `docs/MANUAL_LIVE_GATE_RUNBOOK.md` is the procedure an
operator would follow, and `tests/live_gate_runbook_test.rs` holds it to its
own stop conditions.

## Limits on the identity and secret claims

Role typing is not value separation. `DepositWalletIdentityConfig::try_new`
rejects a zero address and an owner whose derived wallet does not match, and it
reports `AuthEqualsOwner`, `AuthEqualsDepositWallet`, and
`OwnerEqualsDepositWallet` as observations; it does not reject them. Policy is
the caller's. `request_context()` converts back to two raw `Address` values, and
the older public surface still accepts raw addresses without going through the
typed config: `RelayerKeyAuth::new`, `RelayerReadPermit::for_owner`, and
`RelayerMutationPermit::try_new` take an `Address` and do not reject the zero
address. So the roles carry distinct types, and a caller who avoids the typed
config can still give two roles the same value.

The secret claim is likewise narrower than "nothing anywhere". The
deposit-wallet client allowlists its host, refuses redirects, bounds bodies, and
redacts its `Debug` and error output. The preserved legacy `RelayClient` does
not: `set_url` accepts any URL and the client then sends an auth header to
`/submit` on it, and nonce, transaction, and submit responses are logged whole
through `debug!(raw_response = %text, ...)`, with HTTP error bodies carried into
`RelayerError::Api`. What is established is that the committed tree holds no
production credential, and that the reviewed deposit-wallet surface redacts what
it emits. A deployment that enables `debug` logging on the legacy path should
treat that path's output as sensitive.

## What this packet does not prove

- It does not prove the relayer accepts any request this fork builds. Every
  fixture is derived from official SDK source and documentation, not from a
  recorded live response.
- It does not prove the consumer wired the adapter correctly. The adapter lives
  in another repository; this audit checks only that nothing else imports the
  fork.
- It does not prove the resolved TLS backend count, feature unification under a
  different consumer feature set, or HTTPS connectivity. No test opens an
  outbound socket.
- It does not prove the absence of vulnerabilities. It proves that
  `cargo audit --deny warnings` passes against the advisory database on the
  scan date, with eight accepted advisories each carrying a re-review condition.
- It does not prove the code behaves correctly against funded state. Nothing
  here has moved value.
