#!/usr/bin/env bash
#
# Codex CLI 공통 실행 래퍼.
# Codex 인증 정보를 보호한 상태로 `codex exec` 를 `--output-schema` 모드로
# 실행하고, 모델의 마지막 응답을 `$OUT_FILE` 로 저장한다.
#
# 필수 환경 변수:
#   PROMPT_FILE — Codex 에 전달할 프롬프트 본문 (stdin 으로 들어감)
#   SCHEMA_FILE — Codex 의 `--output-schema` 에 전달할 JSON Schema
#   OUT_FILE    — Codex 의 마지막 메시지를 기록할 경로
#   RUNNER_TEMP — 임시 디렉토리 (CODEX_HOME 을 만들 장소)
#
# 옵션:
#   LOG_FILE          — Codex stdout+stderr 를 흘려보낼 파일 (기본: $RUNNER_TEMP/codex-run.log)
#   EXTRA_CODEX_FLAGS — 추가 CLI 플래그 (예: 특정 axis 용 reasoning_effort 오버라이드)
#
# self-hosted runner 의 공급망 보호:
#   LD_PRELOAD 로 codex 인증 파일에 대한 무단 접근을 차단하는 가드 라이브러리를 끼워 넣는다.
set -euo pipefail

: "${PROMPT_FILE:?PROMPT_FILE is required}"
: "${SCHEMA_FILE:?SCHEMA_FILE is required}"
: "${OUT_FILE:?OUT_FILE is required}"
: "${RUNNER_TEMP:?RUNNER_TEMP is required}"
: "${GITHUB_WORKSPACE:?GITHUB_WORKSPACE is required}"

# CODEX_WORKSPACE 는 review 대상 (= PR head) 의 체크아웃 경로. v2 pipeline 의
# 모든 review job 은 base-ref 와 PR head 를 분리 체크아웃하고 후자를 여기에
# 둔다. 비어 있으면 GITHUB_WORKSPACE 로 폴백 (single-checkout / 로컬 테스트).
CODEX_CD_DIR="${CODEX_WORKSPACE:-$GITHUB_WORKSPACE}"
test -d "$CODEX_CD_DIR"

GUARD_LIB="/opt/codex-runner/libcodex-deny-auth.so"
test -r "$GUARD_LIB"

export CODEX_AUTH_GUARD_PRELOAD="$GUARD_LIB"
export LD_PRELOAD="$GUARD_LIB"

# self-hosted runner 에는 `CODEX_HOME=/home/runner/.codex` 가 미리 정의되어 있어서
# `${CODEX_HOME:-...}` 식으로 fallback 을 두면 ln 이 자기 자신을 가리키게 되어 실패한다.
# 반드시 RUNNER_TEMP 아래에 새 CODEX_HOME 을 만들고 인증 파일을 symlink 한다.
CODEX_HOME_DIR="$RUNNER_TEMP/codex-home"
mkdir -p "$CODEX_HOME_DIR"
ln -sf /home/runner/.codex/auth.json "$CODEX_HOME_DIR/auth.json"
export CODEX_HOME="$CODEX_HOME_DIR"
export CODEX_AUTH_GUARD_PATH="$CODEX_HOME_DIR/auth.json"

LOG_FILE="${LOG_FILE:-$RUNNER_TEMP/codex-run.log}"

codex login status

# Codex CLI 의 종료 코드 자체로 분기하고 싶으므로 일시적으로 set +e.
# stdout / stderr 는 LOG_FILE 에 모아두고 본체 로그에는 흘리지 않는다 (PR 컨텍스트 유출 방지).
set +e
# shellcheck disable=SC2086  # EXTRA_CODEX_FLAGS 는 의도적으로 word split 한다
codex --enable use_legacy_landlock --ask-for-approval never exec \
  --ephemeral \
  --ignore-user-config \
  --ignore-rules \
  --model gpt-5.5 \
  -c 'model_reasoning_effort="xhigh"' \
  ${EXTRA_CODEX_FLAGS:-} \
  --cd "$CODEX_CD_DIR" \
  --sandbox read-only \
  --output-schema "$SCHEMA_FILE" \
  --output-last-message "$OUT_FILE" \
  --color never \
  - < "$PROMPT_FILE" > "$LOG_FILE" 2>&1
status=$?
set -e

if [ "$status" -ne 0 ]; then
  echo "Codex exec failed with exit status $status." >&2
  if [ -f "$LOG_FILE" ]; then
    SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    echo "::group::codex_exec log tail (last 80 lines, secrets redacted)" >&2
    tail -n 80 "$LOG_FILE" | PYTHONPATH="$SCRIPT_DIR" python3 -c '
import sys
from codex_redaction import redact
for line in sys.stdin:
    sys.stdout.write(redact(line))
' >&2 || true
    echo "::endgroup::" >&2
  fi
  exit "$status"
fi

test -s "$OUT_FILE"
