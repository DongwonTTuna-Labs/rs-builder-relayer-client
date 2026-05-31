Review this pull request and return only codex.stage01.model_review.v1 JSON.
Use axes correctness, tests, performance, and domain. Write human-readable text in Korean.
Use stage00 lifecycle, thread inventory, and artifacts/pr-diff.patch before evaluating the current diff.
Stage07 pushes use checkout GITHUB_TOKEN credentials; they do not trigger a new workflow run, so stage08 reentry is produced in the same run.
The PR head checkout is in workspace; inspect review target files and diffs through that path.
Security hardening is out of scope; do not raise sandbox privilege findings unless they break stage contracts.
Do not edit files, push, post comments, or depend on legacy workflow scripts.
