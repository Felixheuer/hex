#!/usr/bin/env bash
set -euo pipefail

# Runs the opt-in dictation corpus evaluation against Parakeet Unified English.
# Reads real and synthetic clips from AGENT_DESK_DICTATION_CORPUS_DIR and writes
# corpus_report.json inside that directory.

CORPUS_DIR="${AGENT_DESK_DICTATION_CORPUS_DIR:-/private/var/folders/4t/_jnrck017r5f_jttbc19b7s40000gn/T/opencode/hex-dictation-corpus}"
MODEL_PATH="${AGENT_DESK_TAIL_MODEL:-${CORPUS_DIR}/parakeet-unified-en-0.6b-Q8_0.gguf}"
SWEEP_LIMIT="${AGENT_DESK_CORPUS_SWEEP_LIMIT:-25}"
TARGET_DIR="${CARGO_TARGET_DIR:-/Users/kit/mac-remote.noindex/cargo-target/hex}"

if [[ ! -d "$CORPUS_DIR" ]]; then
  echo "error: corpus directory not found: $CORPUS_DIR" >&2
  exit 1
fi

if [[ ! -f "$MODEL_PATH" ]]; then
  DEFAULT_MODEL="$HOME/Library/Application Support/agent-desk/models/parakeet-unified-en-0.6b-Q8_0.gguf"
  if [[ -f "$DEFAULT_MODEL" ]]; then
    MODEL_PATH="$DEFAULT_MODEL"
  else
    echo "error: Unified English GGUF model not found at $MODEL_PATH" >&2
    exit 1
  fi
fi

export AGENT_DESK_DICTATION_CORPUS_DIR="$CORPUS_DIR"
export AGENT_DESK_TAIL_MODEL="$MODEL_PATH"
export AGENT_DESK_CORPUS_SWEEP_LIMIT="$SWEEP_LIMIT"
export CARGO_TARGET_DIR="$TARGET_DIR"

if command -v hivemind >/dev/null 2>&1; then
  exec hivemind run --kind cargo -- \
    cargo test --bin agent-desk -j 2 evaluate_dictation_corpus -- --ignored --nocapture --test-threads=1
else
  exec cargo test --bin agent-desk -j 2 evaluate_dictation_corpus -- --ignored --nocapture --test-threads=1
fi
