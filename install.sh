#!/usr/bin/env bash
# Install the conductr CLI from source.
#
# Usage:
#   ./install.sh                               # from a local checkout
#   curl -fsSL https://raw.githubusercontent.com/Luan-vP/conductr/develop/install.sh | bash
#
# Env overrides:
#   CONDUCTR_REF      git ref to install when cloning (default: develop)
#   CONDUCTR_PROFILE  cargo profile (default: release)

set -euo pipefail

REPO_URL="https://github.com/Luan-vP/conductr.git"
REF="${CONDUCTR_REF:-develop}"
PROFILE="${CONDUCTR_PROFILE:-release}"

if [[ -t 1 ]]; then
  BOLD=$'\033[1m'; DIM=$'\033[2m'; RED=$'\033[31m'; GREEN=$'\033[32m'; YELLOW=$'\033[33m'; RESET=$'\033[0m'
else
  BOLD=''; DIM=''; RED=''; GREEN=''; YELLOW=''; RESET=''
fi

say()  { printf '%s%s%s\n' "$BOLD" "$*" "$RESET"; }
warn() { printf '%s%s%s\n' "$YELLOW" "$*" "$RESET" >&2; }
die()  { printf '%s%s%s\n' "$RED"   "$*" "$RESET" >&2; exit 1; }

require() {
  command -v "$1" >/dev/null 2>&1 || die "missing required tool: $1${2:+ ($2)}"
}

require git   "install via your package manager or https://git-scm.com"
require cargo "install Rust via https://rustup.rs"

if [[ -f "Cargo.toml" && -d "crates/conductr" ]]; then
  SOURCE_DIR="$PWD"
  say "Installing conductr from local checkout: ${DIM}${SOURCE_DIR}${RESET}"
else
  TMPDIR="$(mktemp -d -t conductr-install.XXXXXX)"
  trap 'rm -rf "$TMPDIR"' EXIT
  SOURCE_DIR="$TMPDIR/conductr"
  say "Cloning ${REPO_URL} (${REF}) into ${DIM}${SOURCE_DIR}${RESET}"
  git clone --depth=1 --branch "$REF" --recurse-submodules --shallow-submodules \
    "$REPO_URL" "$SOURCE_DIR"
fi

say "Building and installing (profile: ${PROFILE})"
(
  cd "$SOURCE_DIR"
  cargo install --path crates/conductr --profile "$PROFILE" --locked --force
)

CARGO_BIN="${CARGO_HOME:-$HOME/.cargo}/bin"
case ":$PATH:" in
  *":$CARGO_BIN:"*) ;;
  *) warn "warning: ${CARGO_BIN} is not on your PATH; add it to your shell profile" ;;
esac

if command -v conductr >/dev/null 2>&1; then
  printf '%s%s installed:%s %s\n' "$GREEN" "✓ conductr" "$RESET" "$(command -v conductr)"
  conductr --version 2>/dev/null || true
else
  warn "conductr binary installed to ${CARGO_BIN}/conductr but not visible on PATH yet"
fi

cat <<EOF

Next steps:
  conductr --help
  conductr setup wizard
EOF
