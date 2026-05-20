#!/usr/bin/env bash
#
# Prepare the PR head checkout as read-only review data.
#
# The PR checkout starts shallow and blobless. This script fetches the exact
# base and head snapshots with bounded history, deepens both sides until a
# merge base exists, then hydrates the final text diff that Codex may inspect
# without persisting credentials.
set -euo pipefail

: "${BASE_REF:?BASE_REF is required}"
: "${BASE_SHA:?BASE_SHA is required}"
: "${HEAD_SHA:?HEAD_SHA is required}"
: "${PR_NUMBER:?PR_NUMBER is required}"

REMOTE_NAME="${REMOTE_NAME:-origin}"
DEEPEN_STEP="${DEEPEN_STEP:-256}"
MAX_DEEPEN_ROUNDS="${MAX_DEEPEN_ROUNDS:-8}"

if ! git check-ref-format "refs/remotes/$REMOTE_NAME/$BASE_REF"; then
  echo "::error::Invalid base ref name: $BASE_REF"
  exit 1
fi
if [[ ! "$BASE_SHA" =~ ^[0-9a-f]{40}$ ]]; then
  echo "::error::Invalid base sha format: $BASE_SHA"
  exit 1
fi
if [[ ! "$HEAD_SHA" =~ ^[0-9a-f]{40}$ ]]; then
  echo "::error::Invalid head sha format: $HEAD_SHA"
  exit 1
fi
if [[ ! "$PR_NUMBER" =~ ^[0-9]+$ ]]; then
  echo "::error::Invalid PR number: $PR_NUMBER"
  exit 1
fi

git_with_auth() {
  if [[ -n "${GIT_AUTH_TOKEN:-}" ]]; then
    # Expanded when git invokes the credential helper.
    # shellcheck disable=SC2016
    git \
      -c credential.helper='!f() { echo username=x-access-token; echo "password=$GIT_AUTH_TOKEN"; }; f' \
      -c credential.useHttpPath=true \
      "$@"
  else
    git "$@"
  fi
}

base_ref="refs/remotes/$REMOTE_NAME/$BASE_REF"
git_with_auth fetch --filter=blob:none --depth="$DEEPEN_STEP" "$REMOTE_NAME" "$BASE_SHA"
git update-ref "$base_ref" "$BASE_SHA"
git_with_auth fetch --filter=blob:none --depth="$DEEPEN_STEP" "$REMOTE_NAME" "$HEAD_SHA"

if [[ "$(git rev-parse HEAD)" != "$HEAD_SHA" ]]; then
  echo "::error::Checked out head does not match workflow input HEAD_SHA."
  exit 1
fi

round=0
until git merge-base "$base_ref" HEAD >/dev/null 2>&1; do
  round=$((round + 1))
  if (( round > MAX_DEEPEN_ROUNDS )); then
    echo "::error::Could not find merge-base after $MAX_DEEPEN_ROUNDS deepen rounds."
    exit 1
  fi
  git_with_auth fetch --filter=blob:none --deepen="$DEEPEN_STEP" "$REMOTE_NAME" "$BASE_SHA"
  git_with_auth fetch --filter=blob:none --deepen="$DEEPEN_STEP" "$REMOTE_NAME" "$HEAD_SHA"
done

# Hydrate the final range diff without binary payloads:
# Codex review prompts contain API-provided patch excerpts, while this path only
# makes common git inspection commands work without lazy network fetches.
git_with_auth diff --no-ext-diff "$base_ref...HEAD" >/dev/null
git log --oneline "$base_ref..HEAD" >/dev/null

if ! GIT_NO_LAZY_FETCH=1 git diff --no-ext-diff "$base_ref...HEAD" >/dev/null; then
  echo "::error::Review range diff still requires lazy fetch."
  exit 1
fi
if ! GIT_NO_LAZY_FETCH=1 git log --oneline "$base_ref..HEAD" >/dev/null; then
  echo "::error::Review commit log still requires lazy fetch."
  exit 1
fi

if git config --local --name-only --get-regexp '^(http\..*\.extraheader|credential\.helper)$' >/dev/null 2>&1; then
  echo "::error::Credential configuration remained in review workspace."
  exit 1
fi
