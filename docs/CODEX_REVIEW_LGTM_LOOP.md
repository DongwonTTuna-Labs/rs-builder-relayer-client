# Codex Review LGTM Loop

This document records the expected OpenSpec-backed Codex Review review-to-fix
loop for the `codex-review-lgtm-loop-smoke` change. It is documentation only:
it does not change Rust public API, workflow behavior, helper code, runner
configuration, secrets, signing, nonce handling, calldata, or any live relayer
execution behavior.

## Source Of Truth And Context Collection

When a PR title or body links the same-repo OpenSpec change path
`openspec/changes/codex-review-lgtm-loop-smoke`, Codex Review treats that
OpenSpec change as authoritative source material for review and fix planning.

The context collection step must collect the repository OpenSpec configuration
and the complete change artifact set from `pr-head`:

- `openspec/config.yaml`
- `openspec/changes/codex-review-lgtm-loop-smoke/proposal.md`
- `openspec/changes/codex-review-lgtm-loop-smoke/design.md`
- `openspec/changes/codex-review-lgtm-loop-smoke/tasks.md`
- `openspec/changes/codex-review-lgtm-loop-smoke/specs/codex-review-lgtm-loop/spec.md`

The collected artifacts define the required implementation target, acceptance
criteria, non-goals, and validation commands. For this smoke change, the
unchecked OpenSpec task requires the single missing documentation artifact
`docs/CODEX_REVIEW_LGTM_LOOP.md`.

## Review Routing And Inspection Evidence

Stage01 reviewers inspect the checked-out PR tree, not only the PR diff summary.
Their outputs must include repository inspection evidence with concrete
`pr-head` paths, the purpose of each inspection, and the observation learned
from that path. If a reviewer reports no finding, the output still needs
non-empty inspection evidence.

Stage01 through Stage04 are expected to compare the PR tree against the
OpenSpec-backed requirements. In this smoke, the relevant observation is that
`tasks.md` requires `docs/CODEX_REVIEW_LGTM_LOOP.md`, while the target document
is absent from the PR tree.

Stage02 routes that mismatch to design/fix as a docs-only OpenSpec task instead
of stopping at generic human review. Stage03 then produces an executable plan
constrained to `docs/CODEX_REVIEW_LGTM_LOOP.md`, with validation commands suited
to documentation-only work and acceptance criteria derived from the OpenSpec
artifacts. Stage04 approves the plan when there is no real mechanical blocker.

The Stage03 inspection evidence should name existing `pr-head` source paths such
as the OpenSpec proposal, design, tasks, spec, and config. The missing target
path belongs in observations, acceptance criteria, and edit sequencing; it is
not itself inspection evidence until the file exists.

## Autofix And Semantic Safety

Stage05 prepares the autofix task from the approved Stage03 and Stage04 plan.
For this smoke, the allowed file set is exactly:

- `docs/CODEX_REVIEW_LGTM_LOOP.md`

Stage06 and later autofix stages preserve semantic safety by keeping the patch
inside that allowed file set. The fix must not modify Rust code, public API,
GitHub Actions workflows, workflow helper scripts, runner setup, secrets,
dependency files, signing logic, nonce behavior, wire format, calldata, or live
relayer behavior.

If repository policy allows an autofix commit, Stage07 writes only the approved
documentation patch. PR mutation, issue creation, and review posting must use a
scoped GitHub App token. The workflow must not rely on broad write permissions
for `GITHUB_TOKEN`; the App token write-only principle is part of the smoke's
safety boundary.

If pushing is disabled or unavailable, the later fallback path should create an
idempotent issue that records the OpenSpec source, attempted stages, and the
required follow-up, without widening the patch scope.

## Stale Run Cancellation

Codex Review runs for the same PR should be mutually superseding. When a new
commit is pushed to the PR branch while an older Codex Review run is still in
progress, GitHub Actions should cancel the stale run.

The replacement run must continue from the latest PR head SHA, not an older
checkout, cached review result, or superseded plan. Review findings, Stage03
planning, Stage05 autofix preparation, and any write attempt are expected to be
