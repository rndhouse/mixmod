#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if [[ "${1:-}" == "--" ]]; then
  shift
fi

if [[ "$#" -eq 0 ]]; then
  cat >&2 <<'EOF'
usage: scripts/deepswe_run_with_worker.sh [--] <benchmark command> [args...]

Starts the llama.cpp OpenAI-compatible worker, blocks until it is ready,
runs the benchmark command with MIXMOD_OPENCODE_BASE_URL set, then tears
the worker down. Set MIXMOD_KEEP_LLAMA_WORKER=1 to leave it running.
EOF
  exit 2
fi

cleaned_up=0
cleanup() {
  local status=$?
  if [[ "$cleaned_up" == "1" ]]; then
    exit "$status"
  fi
  cleaned_up=1
  trap - EXIT INT TERM
  if [[ "${MIXMOD_KEEP_LLAMA_WORKER:-0}" == "1" ]]; then
    echo "TEARDOWN_SKIP reason=keep_worker"
  else
    "${script_dir}/deepswe_teardown_llama_worker.sh" || true
  fi
  exit "$status"
}
trap cleanup EXIT INT TERM

"${script_dir}/deepswe_setup_llama_worker.sh"

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

# shellcheck disable=SC1090
source "$state_env"
export MIXMOD_OPENCODE_BASE_URL="$LLAMA_WORKER_BASE_URL"

printf 'BENCH_START worker_base_url=%s command=' "$MIXMOD_OPENCODE_BASE_URL"
printf '%q ' "$@"
printf '\n'
set +e
"$@"
code=$?
set -e
echo "BENCH_DONE exit=${code}"
exit "$code"
