#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=scripts/mixmod_bench_cmd.sh
source "${script_dir}/mixmod_bench_cmd.sh"

run_mixmod_bench worker setup-llama
