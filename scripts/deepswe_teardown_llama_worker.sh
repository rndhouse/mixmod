#!/usr/bin/env bash
set -euo pipefail

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

if [[ ! -f "$state_env" ]]; then
  echo "TEARDOWN_SKIP reason=no_state state=${state_env}"
  exit 0
fi

# shellcheck disable=SC1090
source "$state_env"

pid="${LLAMA_WORKER_PID:-}"
managed="${LLAMA_WORKER_MANAGED:-false}"
log_path="${LLAMA_WORKER_LOG:-}"
timeout="${MIXMOD_LLAMA_TEARDOWN_TIMEOUT_SECONDS:-30}"

process_alive() {
  local value="$1"
  [[ -n "$value" ]] && kill -0 "$value" 2>/dev/null
}

write_status() {
  local status="$1"
  python3 - "$state_json" "$status" <<'PY' || true
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
status = sys.argv[2]
try:
    data = json.loads(path.read_text())
except Exception:
    data = {}
data["status"] = status
path.write_text(json.dumps(data, indent=2, sort_keys=True) + "\n")
PY
}

if [[ "$managed" != "true" || -z "$pid" ]]; then
  write_status "external_or_unmanaged"
  echo "TEARDOWN_SKIP reason=unmanaged pid=${pid:-none} log=${log_path:-none}"
  exit 0
fi

if ! process_alive "$pid"; then
  write_status "already_stopped"
  echo "TEARDOWN_DONE already_stopped pid=${pid} log=${log_path:-none}"
  exit 0
fi

kill "$pid" 2>/dev/null || true
deadline=$((SECONDS + timeout))
while (( SECONDS < deadline )); do
  if ! process_alive "$pid"; then
    write_status "stopped"
    echo "TEARDOWN_DONE pid=${pid} log=${log_path:-none}"
    exit 0
  fi
  sleep 1
done

kill -KILL "$pid" 2>/dev/null || true
sleep 1
if process_alive "$pid"; then
  write_status "kill_failed"
  echo "TEARDOWN_FAILED pid=${pid} log=${log_path:-none}" >&2
  exit 1
fi

write_status "killed"
echo "TEARDOWN_DONE killed=true pid=${pid} log=${log_path:-none}"
