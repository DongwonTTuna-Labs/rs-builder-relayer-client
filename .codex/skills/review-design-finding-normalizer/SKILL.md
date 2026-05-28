---
name: review-design-finding-normalizer
description: Normalize Codex PR review findings, unresolved inline threads, and advisory review history into compact design-planning inventory without deciding fixes.
---

# Review Design Finding Normalizer

Use this skill only inside the Codex PR review design stage.

## Contract

- Treat current user instruction, current PR body, current code, current diff, AGENTS.md, and current docs as authoritative.
- Treat old review comments, old resolve summaries, and old design plans as advisory memory only.
- Do not design a fix, choose an architecture, or write an edit sequence.
- Preserve stale/uncertain signals instead of silently trusting old comments.
- Normalize review input into concise inventory items that later stages can cluster.

## Method

1. Read the review context warning first.
2. Convert each allowed finding, unresolved thread, and relevant advisory-history item into a compact inventory item.
3. Assign each item one invariant/root-cause label when possible.
4. Record affected code surface and conflict keys, especially shared state machines, public APIs, tests, auth/signing, nonce, polling, and production gates.
5. Mark old or re-anchored material as advisory when it cannot be verified from current code/diff.

## Output Discipline

- Output JSON only.
- Keep Korean human-readable summaries.
- Keep file paths, identifiers, schema keys, and finding IDs in their original language.
- Do not include secrets, tokens, private keys, or full long comments.
