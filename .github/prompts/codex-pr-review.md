# Codex PR Review Prompt

You are reviewing the currently checked-out pull request for `DongwonTTuna/rs-builder-relayer-client`.

Read and follow `AGENTS.md` before drawing conclusions. Also check `docs/TESTING.md` and `docs/REVIEW_CHECKLIST.md` when the change touches relayer behavior, signing, nonce handling, request serialization, deposit-wallet code, fixtures, or public API.

Do not edit files. Review only.

Focus on:

- correctness bugs, behavioral regressions, unsafe assumptions, and missing tests;
- secret exposure through logs, errors, fixtures, debug output, or snapshots;
- public API changes that are not conservative or not documented;
- venue-facing wire format, signing, nonce, transaction-state, or calldata changes without fixture or official SDK evidence;
- accidental claims that deposit-wallet live execution is ready before the required acceptance evidence exists.

Use the PR diff and nearby source context. Prefer `git diff origin/${GITHUB_BASE_REF}...HEAD` when the base ref is available.

Respond in Korean. Put findings first, ordered by severity. Include file paths and line references when possible. If you find no actionable issue, say that clearly and mention any residual test gap or risk.
