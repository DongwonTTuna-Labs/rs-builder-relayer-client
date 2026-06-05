# Learnings

## 2026-06-05 Task: session-start
- Plan `codex-loop-serial-redesign` selected and active in `.omo/boulder.json`.
- Wave 1 is the only immediate parallel batch: Task 1 in RS repo and Task 2 in HSI checkout.
- Plan states RS Task 1 is a temporary noise-stop and must only touch the 3 adapter workflow files.
- Plan states HSI Task 2 must restore simple 5-job reusable topology from `727f713d`, with no matrix/fromJson fan-out.

## 2026-06-05 Task: 1-rs-revert-727
- PR #115 merge commit `f5eafd478627b033d7fac7e817d0fe61c06cdf9e` has first parent `dcb4185ec9acd999b1459c6d1e7b8c37a5d43ea3`; that parent is the source of truth for the temporary 727 adapter revert.
- Restoring the three RS adapter workflows from `dcb4185e` brings back `codex-loop-reusable.yml@727f713d2fc497b735ac17b575b74e282b09830e` and removes the abandoned `3540af40` pin.
- The restored pre-115 adapter shape uses `secrets: inherit` and no `actions: read`; later live-entry/explicit-secret changes are intentionally excluded for the noise-stop.
