#!/usr/bin/env bash
#
# Resolve PR metadata for the `/codex-review` issue_comment trigger.
#
# Reads one REST PR payload so head SHA, base ref, and base SHA are a single
# snapshot. Writes outputs for the reusable review workflow.
set -euo pipefail

: "${REPO:?REPO is required}"
: "${PR_NUMBER:?PR_NUMBER is required}"
: "${GITHUB_OUTPUT:?GITHUB_OUTPUT is required}"

json="$(gh api "repos/$REPO/pulls/$PR_NUMBER")"
head_sha="$(jq -r '.head.sha // ""' <<<"$json")"
base_ref="$(jq -r '.base.ref // ""' <<<"$json")"
base_sha="$(jq -r '.base.sha // ""' <<<"$json")"
is_draft="$(jq -r '.draft // false' <<<"$json")"
head_owner="$(jq -r '.head.repo.owner.login // ""' <<<"$json")"
head_repo="$(jq -r '.head.repo.name // ""' <<<"$json")"
author_login="$(jq -r '.user.login // ""' <<<"$json")"

if [[ ! "$head_sha" =~ ^[0-9a-f]{40}$ ]]; then
  echo "::error::Invalid PR head sha from GitHub API: $head_sha"
  exit 1
fi
if [[ ! "$base_sha" =~ ^[0-9a-f]{40}$ ]]; then
  echo "::error::Invalid PR base sha from GitHub API: $base_sha"
  exit 1
fi

if [[ "$is_draft" == "true" ]]; then
  echo "::warning::PR is draft; refusing to run via /codex-review."
  exit 1
fi

expected_owner="${REPO%/*}"
expected_repo="${REPO#*/}"
if [[ "$author_login" != "$expected_owner" ]]; then
  echo "::warning::Refusing to run /codex-review for PR author: $author_login"
  exit 1
fi
if [[ "$head_owner" != "$expected_owner" || "$head_repo" != "$expected_repo" ]]; then
  echo "::warning::Refusing to run /codex-review on fork PR ($head_owner/$head_repo)."
  exit 1
fi
if [[ "$base_ref" != "main" ]]; then
  echo "::warning::Refusing to run /codex-review on PR targeting non-main branch: $base_ref"
  exit 1
fi

{
  echo "head_sha=$head_sha"
  echo "base_ref=$base_ref"
  echo "base_sha=$base_sha"
} >> "$GITHUB_OUTPUT"
