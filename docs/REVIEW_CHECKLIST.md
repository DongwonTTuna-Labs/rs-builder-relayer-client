# REVIEW_CHECKLIST.md

## General

- [ ] PR is not merged by the agent.
- [ ] No private keys, API keys, auth headers, passphrases, or production signatures are committed.
- [ ] `cargo fmt --all --check` passes.
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings` passes.
- [ ] `cargo test --workspace --all-features` passes.
- [ ] `git diff --check` passes.
- [ ] Any exception to repo rules is justified by documented no-viable-alternative evidence, not convenience.
- [ ] Any rule exception documents failed alternatives, the underlying limitation, consumer impact, and migration/rollback path.
- [ ] Claims in the PR body are backed by command output, fixture evidence, official SDK/doc comparison, or explicitly marked residual risk.
- [ ] Setup/docs/behavior/live-capable changes are not mixed without an explicit scope justification.

## Failure Handling

- [ ] New fixes document the root cause with `who`, `what`, `when`, `why`, and `how` when they address a failure.
- [ ] The change resolves the root cause instead of adding an unbounded retry, sleep, silent fallback, lint allow, broad mock, or test deletion.
- [ ] Any temporary workaround has a removal condition, owner, bound, and risk note.
- [ ] Repeated failures have a regression test, fixture, or documented reason why one cannot be added.

## Fork Scope

- [ ] The change belongs in this relayer crate, not in the consumer CLOB adapter or strategy code.
- [ ] Upstream Safe/Proxy behavior is not silently reused for deposit-wallet `WALLET` flow.
- [ ] New deposit-wallet code lives under `src/deposit_wallet/` or clearly related support modules.
- [ ] Legacy Safe/Proxy helpers remain clearly separated.

## Deposit-Wallet Flow

- [ ] `WALLET-CREATE` is implemented separately from Safe deploy.
- [ ] `WALLET` batch is implemented separately from Safe/Proxy execute.
- [ ] `/nonce?type=WALLET` is fetched fresh before signing.
- [ ] EIP-712 DepositWallet Batch domain/message/signature are fixture-tested.
- [ ] `POST /submit` request body has exact `type = "WALLET"` or `type = "WALLET-CREATE"` shape.
- [ ] Bounded transaction polling treats New/Executed/Mined as pending, Confirmed as the only success, Failed/Invalid as immediate typed failures, and Unknown as reconciliation-required.
- [ ] Polling enforces validated attempt/interval bounds, exact fixed doubling with a maximum cap, no sleep after the final attempt, and cancellation priority before reads and interval waits.
- [ ] Exhausted, Cancelled, unknown, and ambiguous results do not authorize nonce fetch, signing, duplicate submit, or WALLET-CREATE lifecycle re-entry.
- [ ] Deployment lifecycle checks `/deployed` before any mutation, short-circuits when already deployed, and defaults operationally to the explicit `Predeployed` policy.
- [ ] Missing deployment is blocked without an explicit matching mutation permit, and a closed live latch produces no submit HTTP request.
- [ ] WALLET-CREATE readiness is single-shot: Confirmed alone is `Ready`; New/Executed/Mined are pending; Failed/Invalid/Unknown/ambiguous evidence is never success or resubmit authority.
- [ ] Pending owners enter live mutation only through `OwnerMutationRegistry::gate`; transaction id and payload hash are retained and bound polling results are recorded back into the durable intent row.
- [ ] `execute_wallet_batch` validates mutation permit, read permit, deadline, signer, resource limits, and derived wallet before fetching the WALLET nonce.
- [ ] Fresh nonce fetch is immediately followed by local EIP-712 signing with no intervening HTTP await, then the existing validated request builder and permit-gated submit path are reused.
- [ ] DryRun still fetches and records the fresh nonce but sends no POST; a closed Live latch may allow that read but blocks the POST.
- [ ] A second same-owner/chain live execute is rejected while Preparing, Submitted, or AmbiguousNoId, before nonce fetch, signing, or HTTP; different owner/chain scopes remain independent.
- [ ] Confirmed and bound TransactionFailed/Invalid results reopen the owner authoritatively; otherwise only explicit epoch-matched `ReconciliationEvidence` may reconcile or adopt, while unknown, ambiguous, exhausted, cancelled, mismatched, and stale results keep or leave the lock unchanged.
- [ ] DryRun creates no mutation intent, and dropping a lease never implicitly releases or deletes its record.
- [ ] Known-id recovery polls the stored id and never submits; id-less recovery reports candidates only, and no heuristic candidate match automatically adopts or releases an intent.
- [ ] Transaction adoption is evidence-bound, epoch-fenced, AmbiguousNoId-only, and returns the row to Submitted for authoritative expected-type polling.
- [ ] Manual reconciliation covers unresolved restart recovery only with evidence; Preparing is never released until the operator proves no same-owner work is still in flight.

## Identity And Security

- [ ] Relayer API key owner, wallet owner signer, and deposit wallet/funder are separate config/API fields.
- [ ] Tests prove relayer auth identity may differ from owner signer identity.
- [ ] Secret-bearing types do not leak through `Debug`, logs, errors, snapshots, or fixtures.
- [ ] Reconciliation evidence Debug exposes only text lengths, decision, and timestamp; reports omit auth material, raw bodies, and unknown provider state labels.
- [ ] Mutation audit artifacts replace reconciliation reference/summary text with decision, stored UTF-8 byte lengths, and timestamp, including for records restored through Deserialize.
- [ ] Signer backend Display, Debug, and source-chain material is discarded on signing failure; only the fixed redacted signing error is returned.
- [ ] Production dependency instructions use pinned git `rev`, not branch.
- [ ] Dependency changes review public API, transitive crypto/signing crates, and HTTP/TLS impact where applicable.

## pUSD / CTF Operations

- [ ] No legacy USDC.e/direct CTF helper is used for current pUSD-native deposit-wallet operations without explicit review.
- [ ] pUSD approval calldata has a golden test.
- [ ] conditional token approval calldata has a golden test.
- [ ] merge/redeem calldata follows current adapter route and has golden tests.

## Consumer Integration

- [ ] Consumer imports are limited to `pm-adapters/relayer_http` or runtime wiring.
- [ ] No fork-specific DTO leaks into consumer domain/strategy/risk/actor state.
- [ ] Official Rust CLOB SDK remains responsible for CLOB order path.
- [ ] Live relayer mutation remains gated until all fork acceptance tests and operator approval are recorded.
- [ ] Consumer-impacting changes document migration path, rollback path, and any unavailable rollback condition.
- [ ] New public relayer APIs document their production capability boundary, including any method that is intentionally disabled for production URLs.
- [ ] Low-level public reads remain limited to `GET /deployed`, `GET /nonce`, and `GET /transaction`; the gated report adds only permit-first, query-free `GET /transactions`, and mutation still reaches only the two reviewed permit-bound primitive `POST /submit` methods.
- [ ] Polling reuses only the verified expected-type `GET /transaction` path, keeps WALLET and WALLET-CREATE isolated, and treats API/HTTP/temporary-absence errors as transient only within the finite policy.
- [ ] `Retry-After` does not alter the PBRSDK-10 schedule; bounded retries of all API statuses, including auth-related 4xx responses, are documented as an observability tradeoff.
- [ ] The deployment policy is chosen explicitly at every call; `Predeployed` is the normal consumer choice and `DepositWalletDeploymentPolicy` has no `Default` implementation.
- [ ] Lifecycle validation delegates to the existing deployed-read and permit-gated submit paths without duplicating or weakening owner/factory/chain/source checks.
- [ ] Public WALLET transaction reads reject WALLET-CREATE responses, and deployment readiness rejects WALLET responses.
- [ ] Every production read takes an owner- and chain-scoped `RelayerReadPermit` and rejects mismatch before input validation, URL construction, or HTTP I/O.
- [ ] A successful deployed read is not treated as submit readiness; readiness still requires `STATE_CONFIRMED` and the mutation/operator gates.
- [ ] `DepositWalletRelayerClient::new` is default-deny for live mutation; only `new_with_mutation_enabled` starts enabled, and the constructor never replaces a scoped `Live` permit.
- [ ] Every submit requires a permit scoped to mode, operation, owner, chain, and unexpired Unix time with bounded evidence and operator-approval references; mismatch or expiry fails before HTTP.
- [ ] A signed batch whose deadline is equal to or earlier than the current time fails before HTTP.
- [ ] `execute_wallet_batch` takes the signer by generic method argument; the client stores no signer or private key, and relayer auth identity remains independent.
- [ ] `DryRun` sends no HTTP, remains independent of the live latch, and exposes redacted review evidence without auth headers, signatures, full calldata, or a full replayable submit body.
- [ ] Operator review of `DryRun` evidence is followed by a freshly created scoped `Live` permit; dry-run authority is not reused as live authority.
- [ ] `disable_mutation` is a shared one-way latch across all client clones, exposes no re-enable method, blocks later live submits, and leaves reads and valid `DryRun` submissions available.
- [ ] Invalid/partial submit responses, post-dispatch transport failures, and oversized 2xx responses require reconciliation before any resubmission.
- [ ] `MutationIntentRecord` has no raw signature, auth header, private key, calldata, or replayable-body field; its Debug redacts owner and hashes transaction id.
- [ ] `MutationIntentRecord.reconciliation` has `#[serde(default)]`; evidence Deserialize follows the documented trusted-store model and does not silently revalidate persisted text.
- [ ] Manual reconciliation/adoption reload after one CAS miss, repeat epoch and state checks, retry once, and return an explicit error on a second miss.
- [ ] The synchronous durable store implements atomic `try_begin` and epoch/revision CAS as fast local operations; blocking I/O is isolated outside the async executor path.
- [ ] `InMemoryMutationIntentStore` is used only for tests/development and is never presented as restart-safe or live-capable.
- [ ] A durable-store restart exercise proves an unresolved owner remains blocked; PBRSDK-24/25 live qualification is not inferred from the in-memory test.
- [ ] The PR does not claim complete live readiness: manual reconciliation/reporting/audit export does not qualify a durable store, logging collector, automated candidate selection/resubmit, or PBRSDK-24/25 gates.

## Mutation Audit And Observability

- [ ] `poll_attempts` uses `#[serde(default)]`; matching Confirmed adds one, Exhausted with a present state adds its attempts with saturation, and Exhausted without state plus Cancelled remain no-op.
- [ ] Unknown state text is replaced with `<unrecognized relayer state>` independently at poll-record write time and artifact export time.
- [ ] `MutationIntentAuditArtifact` schema v1 exports every intent status without mutating the record; owner is shortened and the fixed omission marker is present.
- [ ] Artifact JSON retains the operator-usable transaction id, while artifact Debug and tracing contain only its `sha3:0x...` sanitized token.
- [ ] Artifact reconciliation contains only decision, `value.len()` UTF-8 byte lengths, and recorded time; operator reference/summary text never appears.
- [ ] `DepositWalletDryRunEvidence` owns pre-submit selector/call summaries, the audit artifact owns persisted lifecycle evidence, and `payload_keccak256` is their correlation key.
- [ ] The `polymarket_relayer::mutation_intent` target emits redacted owner, chain id, and epoch on every event plus only the ADR-0015 event-specific fields; nonce and raw transaction ids are never logged.
- [ ] Registry resolved events are emitted only after CAS `Ok(true)` and lease events only after successful persistence; stale/no-op transitions emit no false resolution.
- [ ] Sentinel regression covers all reviewed public Debug types, artifact JSON, success/failure errors, captured async tracing, actual local signature/calldata/body, and malicious persisted reconciliation/state text.
- [ ] No dependency, metric, OTel path, collector, live request, credential, or new harness was added for observability.

## Public API Boundary

- [ ] `tests/public_api_boundary_test.rs` passes.
- [ ] `cargo doc --workspace --all-features --no-deps` succeeds and rustdoc shows the reviewed `0.2.0` crate-root/deposit-wallet boundary.
- [ ] Crate-root and `deposit_wallet` public exports are explicit; no wildcard public re-export is introduced.
- [ ] `RelayerReadPermit` and the three reviewed HTTP read methods are present in the audited public surface.
- [ ] `RelayerPollPolicy`, `RelayerPollOutcome`, `poll_wallet_transaction`, and `poll_deposit_wallet_deployment` are explicitly present in the audited public surface.
- [ ] `DepositWalletDeploymentPolicy`, `DepositWalletDeploymentStatus`, `DepositWalletReadiness`, and both lifecycle methods are present in the audited public surface.
- [ ] The read audit covers three public methods plus one crate-internal expected-type helper, all permit-bound; file-local primitive counts remain unchanged while combined production source exposes two submit primitives plus one intent-gated submit wrapper.
- [ ] The polling audit fixes both signatures, including policy, read permit, and `cancel: impl Future<Output = ()> + Send`, without changing the low-level read-permit count or adding a public `submit_*` method.
- [ ] Paused polling tests use a timeout-free polling client and plain loopback accept/read futures; exact virtual elapsed time contains only polling sleeps, with CI/runner timeout documented as the server hang guard.
- [ ] Mutation permit/evidence/outcome types, `new_with_mutation_enabled`, `disable_mutation`, `submit_wallet_create`, and `submit_signed_wallet_batch` are present in the audited public surface.
- [ ] `execute.rs` still contains exactly one reviewed generic-signer `execute_wallet_batch`; combined source contains that primitive plus one intent-gated wrapper, both with read/mutation permits.
- [ ] `submit.rs` still contains exactly two public `submit_*` primitives, `lifecycle.rs` still contains exactly two public async methods, and their existing assertions are unchanged.
- [ ] Combined production source contains exactly three `submit_*`, two `execute_wallet_batch`, and two `ensure_deposit_wallet_deployment` signatures, with permit arguments on all wrapper layers.
- [ ] `MutationIntentStore`, `TryBeginOutcome`, `InMemoryMutationIntentStore`, `MutationIntentRecord`, `MutationIntentStatus`, `OwnerMutationRegistry`, `MutationIntentLease`, and `IntentGatedClient` are explicitly re-exported at all three public boundaries.
- [ ] `ReconciliationDecision`, `ReconciliationEvidence`, `IntentReconcileOutcome`, `AmbiguousCandidate`, and `AmbiguousCandidateReport` are explicitly re-exported at all three public boundaries with private fields and reviewed getters.
- [ ] `MutationIntentAuditArtifact`, `ReconciliationSummary`, and `MUTATION_AUDIT_ARTIFACT_SCHEMA_VERSION` are explicitly re-exported at all three public boundaries; the schema constant and `export_audit_artifact` signature remain pinned.
- [ ] `reconcile_manually`, `adopt_transaction`, `reconcile_by_polling`, and `report_ambiguous_candidates` retain their reviewed owner/epoch/evidence/policy/permit/cancel signatures.
- [ ] The store surface has no generic `save`; begin generation is store-issued atomically and every later write is epoch/revision CAS with overflow fail-closed.
- [ ] `record_poll_outcome` and `record_terminal_failure` both require an explicit polled transaction id and cannot resolve a mismatched or non-Submitted record.
- [ ] `build_wallet_batch_request_with_signature` is not restored as a public crate-root or `deposit_wallet` helper.
- [ ] `DepositWalletBatchRequest` remains a validated output type, not a public construction surface with public submit-body fields.
- [ ] Legacy Safe/Proxy APIs stay reference/compatibility surface and are not reused for deposit-wallet `WALLET-CREATE` or `WALLET` flows without wire-level proof.
- [ ] CLOB order/sign/cancel/post behavior remains out of this crate; no CLOB module, example, public import, or order-posting API is added here.
- [ ] Boundary grep audit is attached to the PR evidence:
  `grep -R "pub use .*::\\*\\|pub mod clob\\|pub use clob\\|build_wallet_batch_request_with_signature\\|DepositWalletBatchRequest" -n src tests docs README.md`
