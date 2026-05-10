#!/usr/bin/env bash
set -euo pipefail

# Pull qwen3:27b via ollama. Idempotent — skips if the model is already present.

MODEL="qwen3:27b"

if ! command -v ollama &>/dev/null; then
    echo "error: ollama not found. Run: conductr local setup --provider ollama" >&2
    exit 1
fi

# Check if the model is already pulled
if ollama list 2>/dev/null | awk 'NR>1 {print $1}' | grep -qx "$MODEL"; then
    echo "$MODEL is already present in ollama — skipping pull."
    exit 0
fi

echo "Pulling $MODEL (approximately 16 GB — this may take a while)..."
ollama pull "$MODEL"
echo "$MODEL pulled successfully."
