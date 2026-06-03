# Design

## Approach

This is a docs-only, purely additive change. The autofix agent must CREATE the missing file `docs/CODEX_PUSH_SMOKE.md` with a short description of the Codex Review autofix push smoke. No source code, workflows, schemas, or configuration are touched, and no existing file is modified or deleted.

## Review Expectations

Stage01 reviewers should inspect `pr-head` and find that the OpenSpec task requiring `docs/CODEX_PUSH_SMOKE.md` is incomplete because the file is absent from the PR tree.

## Design And Routing Expectations

- Stage02 should route the missing docs artifact to design/fix.
- Stage03 should produce a closed plan whose `edit_sequence` is limited to creating `docs/CODEX_PUSH_SMOKE.md`, with acceptance criteria derived from the OpenSpec requirement and inspection evidence from `pr-head`.
- Stage04 should approve the executable, docs-only, additive plan.

## Autofix Expectations

- Stage05 should prepare a fix task whose allowed files are limited to `docs/CODEX_PUSH_SMOKE.md`.
- The fix is a single new file (create), expressed as a structured edit with an empty `old_str`.
- Stage06 semantic safety should approve the benign additive docs change.
- Stage07 should push the commit that adds `docs/CODEX_PUSH_SMOKE.md`.

## Constraints

- Only `docs/CODEX_PUSH_SMOKE.md` may be created.
- No deletions and no edits to existing files.
