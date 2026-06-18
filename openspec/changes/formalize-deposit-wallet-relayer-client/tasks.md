## 1. Change Artifact Authoring

- [ ] 1.1 Write `proposal.md` with the change reason, documented scope, impact, and non-goals for this documentation-only OpenSpec change.
- [ ] 1.2 Write `design.md` with the source-of-truth policy, divergence table, normative versus internal-detail policy, architecture overview, and traceability matrix skeleton.
- [ ] 1.3 Keep the change confined to formalizing current PR 37 behavior. No Rust source/tests/Cargo/docs outside the target change are modified.

## 2. Spec Delta Contract Groups

- [ ] 2.1 Author spec delta requirements and scenarios for Group E, endpoint and transport security plus credential redaction.
- [ ] 2.2 Author spec delta requirements and scenarios for Group A, mutation gate default-deny behavior and mutation permit validation.
- [ ] 2.3 Author spec delta requirements and scenarios for Group C, submit flow ordering, nonce lease handoff, and client construction facts.
- [ ] 2.4 Author spec delta requirements and scenarios for Group D+G, nonce read, transaction read, response proof checks, and transaction state classification.
- [ ] 2.5 Author spec delta requirements and scenarios for Group F, wire DTO serialization and EIP-712 DepositWallet Batch signing.
- [ ] 2.6 Author spec delta requirements and scenarios for Group B, owner mutation state, ambiguous submit handling, and manual reconciliation.

## 3. Traceability And Consistency

- [ ] 3.1 Complete the design traceability matrix so every requirement maps to code file lines and test or fixture evidence.
- [ ] 3.2 Check the traceability matrix covers all seven contract groups: Group E, Group A, Group C, Group D, Group G, Group F, and Group B.
- [ ] 3.3 Run a final consistency pass across proposal, design, and spec delta content so non-goals, divergences, and requirement wording agree.
- [ ] 3.4 Confirm the spec delta uses only ADDED Requirements and does not claim pUSD/CTF calldata, consumer adapter/live gate, CLOB order flow, poll loop policy, owner-signer constructor storage, or live production execution readiness.

## 4. OpenSpec Validation And Confinement

- [ ] 4.1 Run `openspec validate formalize-deposit-wallet-relayer-client --strict` and record a passing result.
- [ ] 4.2 Run `openspec show formalize-deposit-wallet-relayer-client` and confirm the rendered change shows the proposal, tasks, design, and spec delta as expected.
- [ ] 4.3 Verify confinement excluding `.omo/` scratch so changed deliverable paths stay under `openspec/changes/formalize-deposit-wallet-relayer-client/`.
- [ ] 4.4 Confirm `tasks.md` contains no copied instruction context or rules blocks and remains a documentation and verification checklist only.
