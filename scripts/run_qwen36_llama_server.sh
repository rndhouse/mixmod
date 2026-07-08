#!/usr/bin/env bash
set -euo pipefail

model_repo="${MIXMOD_LLAMA_HF_REPO:-unsloth/Qwen3.6-27B-MTP-GGUF:Q4_K_M}"
model_alias="${MIXMOD_LLAMA_MODEL_ALIAS:-qwen/qwen3.6-27b}"
host="${MIXMOD_LLAMA_HOST:-0.0.0.0}"
port="${MIXMOD_LLAMA_PORT:-11434}"
ctx_size="${MIXMOD_LLAMA_CTX_SIZE:-8192}"
parallel="${MIXMOD_LLAMA_PARALLEL:-1}"

exec llama-server \
  -hf "${model_repo}" \
  -a "${model_alias}" \
  --jinja \
  --chat-template-kwargs '{"preserve_thinking":true}' \
  --host "${host}" \
  --port "${port}" \
  --ctx-size "${ctx_size}" \
  --parallel "${parallel}" \
  --no-mmap \
  -fa on \
  -kvu \
  --temp 0.6 \
  --top-p 0.95 \
  --top-k 20 \
  --min-p 0.00 \
  --spec-type draft-mtp \
  --spec-draft-n-max 6 \
  "$@"
