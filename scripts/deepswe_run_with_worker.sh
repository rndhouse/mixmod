#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=scripts/mixmod_bench_cmd.sh
source "${script_dir}/mixmod_bench_cmd.sh"

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

run_mixmod_bench worker run-with-llama -- "$@"
