---
name: review-design-invariant-clusterer
description: Cluster normalized Codex review inventory by root cause and invariant for a later design coordinator.
---

# Review Design Invariant Clusterer

Use this skill only inside the Codex PR review design stage.

## Contract

- Cluster by root cause or invariant, not by reviewer axis, file order, or comment order.
- Do not create final architecture or edit sequence.
- Make conflict candidates explicit so the coordinator can resolve them globally.
- Prefer fewer, coherent clusters over many comment-shaped clusters.

## Cluster Heuristics

- Group items that must be fixed by the same state transition, API contract, validation rule, test fixture, or production gate.
- Split items when their fixes can be implemented independently without touching the same state/API/test contract.
- Give every cluster a stable id: `cluster-1`, `cluster-2`, ...
- Include source finding IDs and advisory thread IDs where available.

## Output Discipline

- Output JSON only.
- Korean summaries are required.
- Do not invent missing source IDs.
- Do not decide final edits.
