#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

state_base="${MIXMOD_BENCH_STATE_DIR:-}"
if [[ -z "$state_base" ]]; then
  if [[ -n "${XDG_STATE_HOME:-}" ]]; then
    state_base="${XDG_STATE_HOME}/mixmod/bench-workers"
  elif [[ -n "${HOME:-}" ]]; then
    state_base="${HOME}/.local/state/mixmod/bench-workers"
  else
    state_base="/tmp/mixmod/bench-workers"
  fi
fi

worker_name="${MIXMOD_LLAMA_WORKER_NAME:-deepswe-qwen36-llama}"
state_dir="${MIXMOD_LLAMA_WORKER_STATE_DIR:-${state_base}/${worker_name}}"
state_env="${state_dir}/worker.env"
state_json="${state_dir}/worker.json"
log_path="${MIXMOD_LLAMA_WORKER_LOG:-${state_dir}/llama-server.log}"
command_path="${state_dir}/llama-server.command.txt"

export LLAMA_SERVER_HF_REPO="${LLAMA_SERVER_HF_REPO:-${MIXMOD_LLAMA_HF_REPO:-unsloth/Qwen3.6-27B-MTP-GGUF:Q4_K_M}}"
export LLAMA_SERVER_ALIAS="${LLAMA_SERVER_ALIAS:-${MIXMOD_LLAMA_MODEL_ALIAS:-qwen/qwen3.6-27b}}"
export LLAMA_SERVER_HOST="${LLAMA_SERVER_HOST:-${MIXMOD_LLAMA_HOST:-0.0.0.0}}"
export LLAMA_SERVER_PORT="${LLAMA_SERVER_PORT:-${MIXMOD_LLAMA_PORT:-8080}}"
export LLAMA_SERVER_CTX="${LLAMA_SERVER_CTX:-${MIXMOD_LLAMA_CTX_SIZE:-32768}}"

client_host="${LLAMA_SERVER_CLIENT_HOST:-127.0.0.1}"
base_url="${MIXMOD_OPENCODE_BASE_URL:-http://${client_host}:${LLAMA_SERVER_PORT}/v1}"
ready_url="${MIXMOD_LLAMA_READY_URL:-${base_url}/models}"
ready_timeout="${MIXMOD_LLAMA_READY_TIMEOUT_SECONDS:-900}"
ready_interval="${MIXMOD_LLAMA_READY_INTERVAL_SECONDS:-2}"

mkdir -p "$state_dir"

old_managed=""
old_pid=""
old_base_url=""
if [[ -f "$state_env" ]]; then
  # shellcheck disable=SC1090
  source "$state_env"
  old_managed="${LLAMA_WORKER_MANAGED:-}"
  old_pid="${LLAMA_WORKER_PID:-}"
  old_base_url="${LLAMA_WORKER_BASE_URL:-}"
fi

process_alive() {
  local pid="$1"
  [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null
}

ready_check() {
  python3 - "$ready_url" <<'PY' >/dev/null 2>&1
import sys
import urllib.request

url = sys.argv[1]
try:
    with urllib.request.urlopen(url, timeout=3) as response:
        sys.exit(0 if 200 <= response.status < 300 else 1)
except Exception:
    sys.exit(1)
PY
}

write_state() {
  local managed="$1"
  local pid="$2"
  local status="$3"
  local started_at="$4"

  {
    printf 'LLAMA_WORKER_MANAGED=%q\n' "$managed"
    printf 'LLAMA_WORKER_PID=%q\n' "$pid"
    printf 'LLAMA_WORKER_STATUS=%q\n' "$status"
    printf 'LLAMA_WORKER_BASE_URL=%q\n' "$base_url"
    printf 'LLAMA_WORKER_READY_URL=%q\n' "$ready_url"
    printf 'LLAMA_WORKER_LOG=%q\n' "$log_path"
    printf 'LLAMA_WORKER_COMMAND_LOG=%q\n' "$command_path"
    printf 'LLAMA_WORKER_STARTED_AT=%q\n' "$started_at"
    printf 'LLAMA_WORKER_MODEL=%q\n' "$LLAMA_SERVER_ALIAS"
  } >"${state_env}.tmp"
  mv "${state_env}.tmp" "$state_env"

  STATE_MANAGED="$managed" \
  STATE_PID="$pid" \
  STATE_STATUS="$status" \
  STATE_BASE_URL="$base_url" \
  STATE_READY_URL="$ready_url" \
  STATE_LOG="$log_path" \
  STATE_COMMAND_LOG="$command_path" \
  STATE_STARTED_AT="$started_at" \
  STATE_MODEL="$LLAMA_SERVER_ALIAS" \
  STATE_HF_REPO="$LLAMA_SERVER_HF_REPO" \
  STATE_HOST="$LLAMA_SERVER_HOST" \
  STATE_CONFIGURED_PORT="$LLAMA_SERVER_PORT" \
  STATE_CTX="$LLAMA_SERVER_CTX" \
  python3 - "$state_json" <<'PY'
import json
import os
import sys
from pathlib import Path

path = Path(sys.argv[1])
state = {
    "managed": os.environ["STATE_MANAGED"] == "true",
    "pid": int(os.environ["STATE_PID"]) if os.environ["STATE_PID"] else None,
    "status": os.environ["STATE_STATUS"],
    "base_url": os.environ["STATE_BASE_URL"],
    "ready_url": os.environ["STATE_READY_URL"],
    "log": os.environ["STATE_LOG"],
    "command_log": os.environ["STATE_COMMAND_LOG"],
    "started_at": os.environ["STATE_STARTED_AT"],
    "model": os.environ["STATE_MODEL"],
    "hf_repo": os.environ["STATE_HF_REPO"],
    "configured_host": os.environ["STATE_HOST"],
    "configured_port": int(os.environ["STATE_CONFIGURED_PORT"]),
    "ctx_size": int(os.environ["STATE_CTX"]),
}
path.write_text(json.dumps(state, indent=2, sort_keys=True) + "\n")
PY
}

write_command_log() {
  {
    printf 'started_at=%s\n' "$1"
    printf 'base_url=%s\n' "$base_url"
    printf 'ready_url=%s\n' "$ready_url"
    printf 'log=%s\n' "$log_path"
    printf 'LLAMA_SERVER_HF_REPO=%s\n' "$LLAMA_SERVER_HF_REPO"
    printf 'LLAMA_SERVER_ALIAS=%s\n' "$LLAMA_SERVER_ALIAS"
    printf 'LLAMA_SERVER_HOST=%s\n' "$LLAMA_SERVER_HOST"
    printf 'LLAMA_SERVER_PORT=%s\n' "$LLAMA_SERVER_PORT"
    printf 'LLAMA_SERVER_CTX=%s\n' "$LLAMA_SERVER_CTX"
    printf 'LLAMA_SERVER_EXTRA_ARGS=%s\n' "${LLAMA_SERVER_EXTRA_ARGS:-}"
    printf 'command='
    printf '%q ' bash "${script_dir}/llama_server_qwen36.sh"
    printf '\n'
  } >"$command_path"
}

if ready_check; then
  managed="false"
  pid=""
  if [[ "$old_managed" == "true" && "$old_base_url" == "$base_url" ]] && process_alive "$old_pid"; then
    managed="true"
    pid="$old_pid"
  fi
  started_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  write_command_log "$started_at"
  write_state "$managed" "$pid" "ready" "$started_at"
  echo "SETUP_READY reused=true managed=${managed} base_url=${base_url} state=${state_json} log=${log_path}"
  exit 0
fi

if [[ "$old_managed" == "true" && "$old_base_url" == "$base_url" ]] && process_alive "$old_pid"; then
  pid="$old_pid"
  managed="true"
  started_at="${LLAMA_WORKER_STARTED_AT:-$(date -u +%Y-%m-%dT%H:%M:%SZ)}"
else
  started_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  write_command_log "$started_at"
  : >"$log_path"
  bash "${script_dir}/llama_server_qwen36.sh" >>"$log_path" 2>&1 &
  pid="$!"
  managed="true"
  write_state "$managed" "$pid" "starting" "$started_at"
  echo "SETUP_START pid=${pid} base_url=${base_url} log=${log_path}"
fi

deadline=$((SECONDS + ready_timeout))
while (( SECONDS < deadline )); do
  if ready_check; then
    write_state "$managed" "$pid" "ready" "$started_at"
    echo "SETUP_READY reused=false managed=${managed} pid=${pid} base_url=${base_url} state=${state_json} log=${log_path}"
    exit 0
  fi
  if ! process_alive "$pid"; then
    write_state "$managed" "$pid" "exited_before_ready" "$started_at"
    echo "SETUP_FAILED pid=${pid} exited_before_ready log=${log_path}" >&2
    tail -n 40 "$log_path" >&2 || true
    exit 1
  fi
  sleep "$ready_interval"
done

write_state "$managed" "$pid" "timeout" "$started_at"
echo "SETUP_FAILED timeout=${ready_timeout}s pid=${pid} base_url=${base_url} log=${log_path}" >&2
tail -n 40 "$log_path" >&2 || true
exit 1
