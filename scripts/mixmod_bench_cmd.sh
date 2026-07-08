#!/usr/bin/env bash
set -euo pipefail

mixmod_bench_script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
mixmod_bench_repo_root="$(cd "${mixmod_bench_script_dir}/.." && pwd)"

if [[ -n "${MIXMOD_BENCH_BIN:-}" ]]; then
  MIXMOD_BENCH_CMD=("${MIXMOD_BENCH_BIN}")
elif [[ -x "${mixmod_bench_repo_root}/target/debug/mixmod-bench" ]]; then
  MIXMOD_BENCH_CMD=("${mixmod_bench_repo_root}/target/debug/mixmod-bench")
elif [[ -x "${mixmod_bench_repo_root}/target/release/mixmod-bench" ]]; then
  MIXMOD_BENCH_CMD=("${mixmod_bench_repo_root}/target/release/mixmod-bench")
elif command -v mixmod-bench >/dev/null 2>&1; then
  MIXMOD_BENCH_CMD=("$(command -v mixmod-bench)")
else
  MIXMOD_BENCH_CMD=(cargo run --quiet --bin mixmod-bench --)
fi

run_mixmod_bench() {
  "${MIXMOD_BENCH_CMD[@]}" "$@"
}
