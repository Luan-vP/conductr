#!/usr/bin/env bash
# Cron entry point: play one orchestrate cycle on this repo.
# Replace with `conductr begin --tag conductr` once #21 lands (#TBD bootstrap).
set -euo pipefail

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LOG_DIR="${HOME}/.local/share/conductr"
mkdir -p "$LOG_DIR"

# Cron runs a stripped shell; pull in the user env so `cargo` and `gh` are on PATH
# and `gh` finds its auth token. Failures are non-fatal so we don't abort if a
# rcfile happens to error under -e.
set +e
[ -f "$HOME/.profile" ] && . "$HOME/.profile"
[ -f "$HOME/.bashrc" ] && . "$HOME/.bashrc"
set -e

cd "$REPO_DIR"
{
  printf '\n=== %s ===\n' "$(date -Iseconds)"
  cargo run --release --quiet --bin conductr -- \
    orchestrate --repo Luan-vP/conductr --once
} >> "$LOG_DIR/orchestrate.log" 2>&1
