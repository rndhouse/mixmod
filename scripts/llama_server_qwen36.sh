#!/usr/bin/env bash
set -euo pipefail

: "${LLAMA_SERVER_HF_REPO:=unsloth/Qwen3.6-27B-MTP-GGUF:Q4_K_M}"
: "${LLAMA_SERVER_ALIAS:=qwen/qwen3.6-27b}"
: "${LLAMA_SERVER_HOST:=0.0.0.0}"
: "${LLAMA_SERVER_PORT:=8080}"
: "${LLAMA_SERVER_CTX:=32768}"
: "${LLAMA_SERVER_TEMP:=0.6}"
: "${LLAMA_SERVER_TOP_P:=0.95}"
: "${LLAMA_SERVER_TOP_K:=20}"
: "${LLAMA_SERVER_MIN_P:=0.00}"
: "${LLAMA_SERVER_SPEC_DRAFT_N_MAX:=6}"
: "${LLAMA_SERVER_EXTRA_ARGS:=}"

exec llama-server \
  -hf "$LLAMA_SERVER_HF_REPO" \
  -a "$LLAMA_SERVER_ALIAS" \
  --jinja \
  --chat-template-kwargs '{"preserve_thinking":true}' \
  --host "$LLAMA_SERVER_HOST" \
  --port "$LLAMA_SERVER_PORT" \
  --no-mmap \
  -fa on \
  -kvu \
  --metrics \
  -c "$LLAMA_SERVER_CTX" \
  --temp "$LLAMA_SERVER_TEMP" \
  --top-p "$LLAMA_SERVER_TOP_P" \
  --top-k "$LLAMA_SERVER_TOP_K" \
  --min-p "$LLAMA_SERVER_MIN_P" \
  --spec-type draft-mtp \
  --spec-draft-n-max "$LLAMA_SERVER_SPEC_DRAFT_N_MAX" \
  $LLAMA_SERVER_EXTRA_ARGS
