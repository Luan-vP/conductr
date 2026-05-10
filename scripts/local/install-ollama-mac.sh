#!/usr/bin/env bash
set -euo pipefail

# Install ollama on macOS via Homebrew. Idempotent — exits 0 if already installed.

if command -v ollama &>/dev/null; then
    echo "ollama already installed ($(ollama --version 2>/dev/null || echo 'version unknown'))"
    exit 0
fi

if ! command -v brew &>/dev/null; then
    echo "error: Homebrew not found. Install it from https://brew.sh then re-run." >&2
    exit 1
fi

echo "Installing ollama via Homebrew..."
brew install ollama

# Start the ollama server in the background (brew service or background process)
if brew services list &>/dev/null 2>&1; then
    brew services start ollama || ollama serve &>/dev/null &
else
    ollama serve &>/dev/null &
    echo "ollama server started in background (PID $!)."
fi

echo "ollama installed successfully."
