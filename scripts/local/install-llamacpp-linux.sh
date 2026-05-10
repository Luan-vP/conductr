#!/usr/bin/env bash
set -euo pipefail

# Build and install llama.cpp on Linux from source. Idempotent — exits 0 if already installed.

if command -v llama-server &>/dev/null; then
    echo "llama-server already installed ($(llama-server --version 2>/dev/null || echo 'version unknown'))"
    exit 0
fi

INSTALL_DIR="${LLAMA_CPP_DIR:-$HOME/.local/llama.cpp}"
BIN="$INSTALL_DIR/build/bin/llama-server"

# Also check the symlink we create
if [ -f "$HOME/.local/bin/llama-server" ]; then
    echo "llama-server already installed at $HOME/.local/bin/llama-server"
    exit 0
fi

# Ensure build dependencies
missing_deps=()
for dep in cmake git; do
    command -v "$dep" &>/dev/null || missing_deps+=("$dep")
done
if ! command -v g++ &>/dev/null && ! command -v c++ &>/dev/null; then
    missing_deps+=("g++")
fi

if [ "${#missing_deps[@]}" -gt 0 ]; then
    echo "Installing build dependencies: ${missing_deps[*]}"
    if command -v apt-get &>/dev/null; then
        sudo apt-get install -y cmake build-essential git
    elif command -v dnf &>/dev/null; then
        sudo dnf install -y cmake gcc-c++ make git
    elif command -v yum &>/dev/null; then
        sudo yum install -y cmake gcc-c++ make git
    else
        echo "error: Could not detect package manager. Install cmake, g++, and git manually." >&2
        exit 1
    fi
fi

if [ -d "$INSTALL_DIR" ]; then
    echo "Updating existing llama.cpp checkout at $INSTALL_DIR..."
    cd "$INSTALL_DIR"
    git pull --ff-only
else
    echo "Cloning llama.cpp into $INSTALL_DIR..."
    git clone --depth=1 https://github.com/ggerganov/llama.cpp.git "$INSTALL_DIR"
    cd "$INSTALL_DIR"
fi

echo "Building llama.cpp (this may take several minutes)..."
cmake -B build -DLLAMA_BUILD_SERVER=ON -DCMAKE_BUILD_TYPE=Release
cmake --build build --config Release -j"$(nproc 2>/dev/null || echo 4)"

if [ ! -f "$BIN" ]; then
    echo "error: Build completed but llama-server binary not found at $BIN" >&2
    exit 1
fi

mkdir -p "$HOME/.local/bin"
ln -sf "$BIN" "$HOME/.local/bin/llama-server"

if echo ":${PATH}:" | grep -q ":$HOME/.local/bin:"; then
    echo "llama-server installed and on PATH."
else
    echo "llama-server installed to $HOME/.local/bin/llama-server"
    echo "Add to your shell profile: export PATH=\"\$HOME/.local/bin:\$PATH\""
fi

echo "llama.cpp installed successfully."
echo ""
echo "Start the server with:"
echo "  llama-server --port 8080 --model <path-to-gguf-model>"
