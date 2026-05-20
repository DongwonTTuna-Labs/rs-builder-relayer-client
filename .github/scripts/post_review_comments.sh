#!/usr/bin/env bash
#
# Codex PR Review v2 — deterministic post stage 의 진입점.
#
# Stage 3 에서 실행된다. LLM 은 호출하지 않는다.
# - inline 코멘트의 create / update / resolve 처리 (file:line:axis:title-hash 로 dedup)
# - sticky 종합 코멘트의 PATCH / POST
#
# 필수 환경 변수:
#   GH_TOKEN, GITHUB_REPOSITORY, PR_NUMBER, HEAD_SHA
#
# 옵션:
#   ART_DIR    — artifact 디렉토리 (기본: ./artifacts)
#   TEMPLATE   — sticky 코멘트 템플릿 (기본: .github/scripts/review-summary-template.md)
#   TRIGGER    — sticky 본문에 표시할 trigger 이름 (기본: unknown)
set -euo pipefail

: "${GH_TOKEN:?GH_TOKEN is required}"
: "${GITHUB_REPOSITORY:?GITHUB_REPOSITORY is required}"
: "${PR_NUMBER:?PR_NUMBER is required}"
: "${HEAD_SHA:?HEAD_SHA is required}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export PYTHONPATH="$SCRIPT_DIR${PYTHONPATH:+:$PYTHONPATH}"

python3 "$SCRIPT_DIR/post_review_comments.py"
