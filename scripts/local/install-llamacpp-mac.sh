#!/usr/bin/env bash
set -euo pipefail

# Install llama.cpp on macOS via Homebrew. Idempotent — exits 0 if already installed.

if command -v llama-server &>/dev/null; then
    echo "llama-server already installed ($(llama-server --version 2>/dev/null || echo 'version unknown'))"
    exit 0
fi

if ! command -v brew &>/dev/null; then
    echo "error: Homebrew not found. Install it from https://brew.sh then re-run." >&2
    exit 1
fi

echo "Installing llama.cpp via Homebrew..."
brew install llama.cpp

echo "llama.cpp installed successfully."
echo ""
echo "Start the server with:"
echo "  llama-server --port 8080 --model <path-to-gguf-model>"
