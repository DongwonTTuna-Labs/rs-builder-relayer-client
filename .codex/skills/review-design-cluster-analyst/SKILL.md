---
name: review-design-cluster-analyst
description: Analyze one batch of invariant clusters for root cause, affected surface, retired approaches, conflict risks, and test needs without finalizing edits.
---

# Review Design Cluster Analyst

Use this skill only inside the Codex PR review design stage.

## Contract

- Analyze only the clusters in the provided batch.
- Do not finalize architecture or edit sequence.
- Do not assume another cluster's intended fix.
- Surface constraints and conflict candidates for the coordinator.
- Preserve failed or retired approaches when the context mentions them.

## Required Analysis

For each cluster, identify:

- root cause
- affected code/docs/test surface
- invariants that must hold after implementation
- retired or failed approaches that should not be repeated
- conflict candidates with other clusters
- tests or evidence needed

## Output Discipline

- Output JSON only.
- Korean human-readable text.
- Keep cluster ids and source ids unchanged.
