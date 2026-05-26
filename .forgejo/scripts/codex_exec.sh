#!/usr/bin/env bash
#
# Codex CLI 공통 실행 래퍼.
# codex-lb API key 로 `codex exec` 를 `--output-schema` 모드로 실행하고,
# 모델의 마지막 응답을 `$OUT_FILE` 로 저장한다.
#
# 필수 환경 변수:
#   PROMPT_FILE — Codex 에 전달할 프롬프트 본문 (stdin 으로 들어감)
#   SCHEMA_FILE — Codex 의 `--output-schema` 에 전달할 JSON Schema
#   OUT_FILE    — Codex 의 마지막 메시지를 기록할 경로
#   RUNNER_TEMP — 임시 디렉토리
#
# 옵션:
#   LOG_FILE          — Codex stdout+stderr 를 흘려보낼 파일 (기본: $RUNNER_TEMP/codex-run.log)
#   EXTRA_CODEX_FLAGS — 추가 CLI 플래그 (예: 특정 axis 용 reasoning_effort 오버라이드)
set -euo pipefail

drop_codex_subprocess_tokens() {
  unset GIT_AUTH_TOKEN
  unset GH_TOKEN
  unset GITHUB_TOKEN
  unset FORGEJO_BOT_TOKEN
  unset ACTIONS_RUNTIME_TOKEN
  unset ACTIONS_CACHE_URL
  unset ACTIONS_RESULTS_URL
  unset ACTIONS_RUNTIME_URL
  unset ACTIONS_ID_TOKEN_REQUEST_TOKEN
  unset ACTIONS_ID_TOKEN_REQUEST_URL
}

if [ "${CODEX_EXEC_TEST_DROP_TOKENS:-}" = "1" ]; then
  drop_codex_subprocess_tokens
  for name in \
    GIT_AUTH_TOKEN \
    GH_TOKEN \
    GITHUB_TOKEN \
    FORGEJO_BOT_TOKEN \
    ACTIONS_RUNTIME_TOKEN \
    ACTIONS_CACHE_URL \
    ACTIONS_RESULTS_URL \
    ACTIONS_RUNTIME_URL \
    ACTIONS_ID_TOKEN_REQUEST_TOKEN \
    ACTIONS_ID_TOKEN_REQUEST_URL
  do
    if [ "${!name+x}" = "x" ]; then
      echo "token variable still present: $name" >&2
      exit 1
    fi
  done
  exit 0
fi

: "${PROMPT_FILE:?PROMPT_FILE is required}"
: "${SCHEMA_FILE:?SCHEMA_FILE is required}"
: "${OUT_FILE:?OUT_FILE is required}"
: "${RUNNER_TEMP:?RUNNER_TEMP is required}"
: "${GITHUB_WORKSPACE:?GITHUB_WORKSPACE is required}"
: "${CODEX_LB_FORGEJO_RUNNER_API_KEY:?CODEX_LB_FORGEJO_RUNNER_API_KEY is required}"

# CODEX_WORKSPACE 는 Codex 실행 경로를 명시해야 하는 caller 용 override.
# Forgejo review job 은 raw PR checkout 대신 redacted context artifact 만 전달하므로
# CODEX_REQUIRE_CLEAN_WORKSPACE=1 일 때 RUNNER_TEMP 아래의 빈 workspace 를
# 만들어 Codex tool sandbox 의 working root 로만 사용한다.
if [ "${CODEX_REQUIRE_CLEAN_WORKSPACE:-}" = "1" ]; then
  CODEX_WORKSPACE="${CODEX_WORKSPACE:-$RUNNER_TEMP/codex-workspace}"
  case "$CODEX_WORKSPACE" in
    "$RUNNER_TEMP"/*) ;;
    *) echo "CODEX_WORKSPACE must be under RUNNER_TEMP when clean workspace is required." >&2; exit 1 ;;
  esac
  LD_PRELOAD= rm -rf -- "$CODEX_WORKSPACE"
  LD_PRELOAD= install -d -m 700 "$CODEX_WORKSPACE"
fi
CODEX_CD_DIR="${CODEX_WORKSPACE:-$GITHUB_WORKSPACE}"
test -d "$CODEX_CD_DIR"

LOG_FILE="${LOG_FILE:-$RUNNER_TEMP/codex-run.log}"

# The model process never needs repository, Forgejo bot, or Actions artifact
# credentials. Keep those tokens out of Codex subprocess environment even when
# the runner injects them for surrounding checkout/artifact steps.
drop_codex_subprocess_tokens

# Codex CLI 의 종료 코드 자체로 분기하고 싶으므로 일시적으로 set +e.
# stdout / stderr 는 LOG_FILE 에 모아두고 본체 로그에는 흘리지 않는다 (PR 컨텍스트 유출 방지).
set +e
# shellcheck disable=SC2086  # EXTRA_CODEX_FLAGS 는 의도적으로 word split 한다
codex --enable use_legacy_landlock --disable shell_tool --ask-for-approval never exec \
  --ephemeral \
  --ignore-user-config \
  --ignore-rules \
  --skip-git-repo-check \
  --model gpt-5.5 \
  -c 'model_provider="codex-lb"' \
  -c 'model_providers.codex-lb.name="OpenAI"' \
  -c 'model_providers.codex-lb.base_url="https://relay-ai.dongwontuna.net/backend-api/codex"' \
  -c 'model_providers.codex-lb.wire_api="responses"' \
  -c 'model_providers.codex-lb.env_key="CODEX_LB_FORGEJO_RUNNER_API_KEY"' \
  -c 'model_providers.codex-lb.supports_websockets=true' \
  -c 'model_providers.codex-lb.requires_openai_auth=true' \
  -c 'model_reasoning_effort="xhigh"' \
  -c 'shell_environment_policy.exclude=["CODEX_LB_FORGEJO_RUNNER_API_KEY"]' \
  -c 'sandbox_workspace_write.network_access=false' \
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
