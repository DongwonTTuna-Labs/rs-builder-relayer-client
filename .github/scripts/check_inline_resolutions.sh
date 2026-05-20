#!/usr/bin/env bash
#
# Codex PR Review v2 — Stage 0 (resolve-check) 의 진입점.
#
# 필수 환경 변수:
#   GH_TOKEN, GITHUB_REPOSITORY, PR_NUMBER, GITHUB_WORKSPACE, RUNNER_TEMP
# 옵션:
#   ART_DIR (기본: ./artifacts), BATCH_SIZE (기본: 3), SNIPPET_RADIUS (기본: 15)
set -euo pipefail

: "${GH_TOKEN:?GH_TOKEN is required}"
: "${GITHUB_REPOSITORY:?GITHUB_REPOSITORY is required}"
: "${PR_NUMBER:?PR_NUMBER is required}"
: "${GITHUB_WORKSPACE:?GITHUB_WORKSPACE is required}"
: "${RUNNER_TEMP:?RUNNER_TEMP is required}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export PYTHONPATH="$SCRIPT_DIR${PYTHONPATH:+:$PYTHONPATH}"
python3 "$SCRIPT_DIR/check_inline_resolutions.py"
