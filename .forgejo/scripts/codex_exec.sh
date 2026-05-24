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
#   RUNNER_TEMP — 임시 디렉토리
#
# 옵션:
#   LOG_FILE          — Codex stdout+stderr 를 흘려보낼 파일 (기본: $RUNNER_TEMP/codex-run.log)
#   EXTRA_CODEX_FLAGS — 추가 CLI 플래그 (예: 특정 axis 용 reasoning_effort 오버라이드)
#
# self-hosted runner 의 공급망 보호:
#   LD_PRELOAD 로 codex 인증 파일에 대한 무단 접근을 차단하는 가드 라이브러리를 끼워 넣는다.
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

GUARD_LIB="${CODEX_AUTH_GUARD_LIB:-/opt/codex-runner/libcodex-deny-auth.so}"
test -r "$GUARD_LIB"

CODEX_HOME_DIR="${CODEX_HOME:-/home/runner/.codex}"
AUTH_FILE="${CODEX_AUTH_FILE:-$CODEX_HOME_DIR/auth.json}"
AUTH_FILE_MODE=""
AUTH_LOCK_FD=""

acquire_auth_lock() {
  local lock_file lock_dir
  lock_file="${CODEX_AUTH_LOCK_FILE:-/var/lib/codex-runner/locks/codex-auth.lock}"
  lock_dir="$(dirname "$lock_file")"
  if ! mkdir -p "$lock_dir" 2>/dev/null; then
    lock_file="$RUNNER_TEMP/codex-auth.lock"
  fi
  exec 9>"$lock_file"
  flock 9
  AUTH_LOCK_FD="9"
}

release_auth_lock() {
  if [ "${AUTH_LOCK_FD:-}" = "9" ]; then
    flock -u 9 2>/dev/null || true
    AUTH_LOCK_FD=""
  fi
}

protect_auth_source() {
  AUTH_FILE_MODE="$(stat -c '%a' "$AUTH_FILE" 2>/dev/null || printf '600')"
  LD_PRELOAD= chmod 000 "$AUTH_FILE"
  release_auth_lock
}

restore_auth_source() {
  if [ -n "${AUTH_FILE_MODE:-}" ] && [ -e "$AUTH_FILE" ]; then
    LD_PRELOAD= chmod "$AUTH_FILE_MODE" "$AUTH_FILE" 2>/dev/null || true
  fi
  release_auth_lock
}

acquire_auth_lock

if [ -e "$AUTH_FILE" ]; then
  current_auth_mode="$(stat -c '%a' "$AUTH_FILE" 2>/dev/null || printf '600')"
  if [ "$current_auth_mode" = "0" ] || [ "$current_auth_mode" = "000" ]; then
    LD_PRELOAD= chmod 600 "$AUTH_FILE"
  fi
fi

if [ ! -s "$AUTH_FILE" ]; then
  echo "Codex auth missing at $AUTH_FILE." >&2
  echo "Run scripts/codex-login-one.sh for this runner before running Codex review jobs." >&2
  exit 1
fi

python3 - "$AUTH_FILE" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
data = json.loads(path.read_text(encoding="utf-8"))
tokens = data.get("tokens") or {}
if data.get("auth_mode") != "chatgpt" or not tokens.get("refresh_token"):
    raise SystemExit(f"Codex auth at {path} is not valid ChatGPT-managed auth.")
print(f"Codex auth ready: auth_mode={data.get('auth_mode')} last_refresh={data.get('last_refresh')}")
PY

RUNTIME_CODEX_HOME="$(mktemp -d "$RUNNER_TEMP/codex-home.XXXXXX")"
cleanup_runtime_codex_home() {
  restore_auth_source
  if [ -n "${RUNTIME_CODEX_HOME:-}" ] && [ -d "$RUNTIME_CODEX_HOME" ]; then
    # The auth guard is intentionally active for Codex subprocesses, but cleanup
    # must be able to remove the temporary auth copy after the review finishes.
    LD_PRELOAD= chmod -R u+rwX "$RUNTIME_CODEX_HOME" 2>/dev/null || true
    LD_PRELOAD= rm -rf -- "$RUNTIME_CODEX_HOME" 2>/dev/null || true
  fi
}
trap cleanup_runtime_codex_home EXIT
cp "$AUTH_FILE" "$RUNTIME_CODEX_HOME/auth.json"
if [ "${CODEX_AUTH_DELETE_SOURCE_AFTER_RUNTIME_COPY:-}" = "1" ]; then
  LD_PRELOAD= rm -f -- "$AUTH_FILE"
  release_auth_lock
else
  protect_auth_source
fi
chmod 700 "$RUNTIME_CODEX_HOME"
chmod 400 "$RUNTIME_CODEX_HOME/auth.json"
export CODEX_HOME="$RUNTIME_CODEX_HOME"
export CODEX_AUTH_GUARD_PRELOAD="$GUARD_LIB"
# The runner auth guard denies CODEX_AUTH_GUARD_PATH and any path ending in
# /.codex/auth.json or /auth.json, so the original runner auth file remains
# blocked even though the model-facing CODEX_HOME points at this runtime copy.
export CODEX_AUTH_GUARD_PATH="$RUNTIME_CODEX_HOME/auth.json"
export LD_PRELOAD="$GUARD_LIB"

LOG_FILE="${LOG_FILE:-$RUNNER_TEMP/codex-run.log}"

# The model process never needs repository, Forgejo bot, or Actions artifact
# credentials. Keep those tokens out of Codex subprocess environment even when
# the runner injects them for surrounding checkout/artifact steps.
drop_codex_subprocess_tokens

codex login status

AUTH_MONITOR_READY="$RUNNER_TEMP/codex-auth-monitor.ready"
AUTH_MONITOR_LOG="$RUNNER_TEMP/codex-auth-monitor.log"
start_auth_open_monitor() {
  rm -f -- "$AUTH_MONITOR_READY" "$AUTH_MONITOR_LOG"
  if [ "${CODEX_AUTH_MONITOR_TEST_FAIL_READY:-}" = "1" ]; then
    (
      echo "simulated auth monitor failure" >&2
      exit 1
    ) >"$AUTH_MONITOR_LOG" 2>&1 &
    auth_monitor_pid=$!
  else
  LD_PRELOAD= python3 - "$RUNTIME_CODEX_HOME/auth.json" "$AUTH_MONITOR_READY" >"$AUTH_MONITOR_LOG" 2>&1 <<'PY' &
import ctypes
import os
import pathlib
import sys

auth_path = pathlib.Path(sys.argv[1])
ready_path = pathlib.Path(sys.argv[2])
libc = ctypes.CDLL("libc.so.6", use_errno=True)
inotify_init1 = libc.inotify_init1
inotify_init1.argtypes = [ctypes.c_int]
inotify_init1.restype = ctypes.c_int
inotify_add_watch = libc.inotify_add_watch
inotify_add_watch.argtypes = [ctypes.c_int, ctypes.c_char_p, ctypes.c_uint32]
inotify_add_watch.restype = ctypes.c_int

IN_OPEN = 0x00000020
IN_CLOEXEC = 0o2000000

fd = inotify_init1(IN_CLOEXEC)
if fd < 0:
    raise OSError(ctypes.get_errno(), "inotify_init1 failed")
watch = inotify_add_watch(fd, os.fsencode(auth_path), IN_OPEN)
if watch < 0:
    raise OSError(ctypes.get_errno(), "inotify_add_watch failed")
ready_path.write_text("ready", encoding="utf-8")
os.read(fd, 4096)
os.close(fd)
PY
  auth_monitor_pid=$!
  fi
  for _ in $(seq 1 100); do
    if [ -s "$AUTH_MONITOR_READY" ]; then
      return 0
    fi
    if ! kill -0 "$auth_monitor_pid" 2>/dev/null; then
      break
    fi
    sleep 0.02
  done
  echo "Codex auth open monitor did not become ready." >&2
  if [ -f "$AUTH_MONITOR_LOG" ]; then
    cat "$AUTH_MONITOR_LOG" >&2
  fi
  return 1
}

stop_auth_open_monitor() {
  if [ -n "${auth_monitor_pid:-}" ] && kill -0 "$auth_monitor_pid" 2>/dev/null; then
    kill "$auth_monitor_pid" 2>/dev/null || true
    wait "$auth_monitor_pid" 2>/dev/null || true
  fi
}

seal_runtime_auth_after_first_open() {
  # Current Codex CLI may reopen auth.json during a run. Keep this as an
  # opt-in probe only; the default protection boundary is the clean Landlock
  # workspace plus the auth guard. When enabled, remove the runtime auth copy
  # immediately after Codex has opened it so model-requested tools cannot make
  # it readable again by unsetting LD_PRELOAD or changing file mode.
  if [ "${CODEX_SEAL_RUNTIME_AUTH_AFTER_OPEN:-0}" != "1" ]; then
    return 0
  fi
  for _ in $(seq 1 500); do
    if [ -n "${auth_monitor_pid:-}" ] && ! kill -0 "$auth_monitor_pid" 2>/dev/null; then
      wait "$auth_monitor_pid"
      auth_monitor_pid=""
      LD_PRELOAD= rm -f -- "$RUNTIME_CODEX_HOME/auth.json" 2>/dev/null || true
      return 0
    fi
    if [ -n "${codex_pid:-}" ] && ! kill -0 "$codex_pid" 2>/dev/null; then
      return 0
    fi
    sleep 0.02
  done
  echo "Codex auth open monitor did not observe Codex reading runtime auth." >&2
  return 1
}

# Codex CLI 의 종료 코드 자체로 분기하고 싶으므로 일시적으로 set +e.
# stdout / stderr 는 LOG_FILE 에 모아두고 본체 로그에는 흘리지 않는다 (PR 컨텍스트 유출 방지).
start_auth_open_monitor
set +e
# shellcheck disable=SC2086  # EXTRA_CODEX_FLAGS 는 의도적으로 word split 한다
codex --enable use_legacy_landlock --ask-for-approval never exec \
  --ephemeral \
  --ignore-user-config \
  --ignore-rules \
  --skip-git-repo-check \
  --model gpt-5.5 \
  -c 'model_reasoning_effort="xhigh"' \
  -c 'sandbox_workspace_write.network_access=false' \
  ${EXTRA_CODEX_FLAGS:-} \
  --cd "$CODEX_CD_DIR" \
  --sandbox read-only \
  --output-schema "$SCHEMA_FILE" \
  --output-last-message "$OUT_FILE" \
  --color never \
  - < "$PROMPT_FILE" > "$LOG_FILE" 2>&1 &
codex_pid=$!
if ! seal_runtime_auth_after_first_open; then
  kill "$codex_pid" 2>/dev/null || true
  wait "$codex_pid" 2>/dev/null || true
  exit 1
fi
wait "$codex_pid"
status=$?
stop_auth_open_monitor
LD_PRELOAD= rm -f -- "$RUNTIME_CODEX_HOME/auth.json" 2>/dev/null || true
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
