#!/usr/bin/env bash
set -euo pipefail

# Install ollama on Linux using the official install script. Idempotent — exits 0 if already installed.

if command -v ollama &>/dev/null; then
    echo "ollama already installed ($(ollama --version 2>/dev/null || echo 'version unknown'))"
    exit 0
fi

if ! command -v curl &>/dev/null; then
    echo "error: curl not found. Install curl and re-run." >&2
    exit 1
fi

echo "Installing ollama via official install script..."
curl -fsSL https://ollama.com/install.sh | sh

# Start ollama as a background service or foreground process
if command -v systemctl &>/dev/null && systemctl --version &>/dev/null 2>&1; then
    sudo systemctl enable --now ollama 2>/dev/null || {
        echo "systemctl start failed; starting ollama in background..."
        ollama serve &>/dev/null &
        echo "ollama server started in background (PID $!)."
    }
else
    ollama serve &>/dev/null &
    echo "ollama server started in background (PID $!)."
fi

echo "ollama installed successfully."
