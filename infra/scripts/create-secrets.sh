#!/usr/bin/env bash
# create-secrets.sh — Interactive helper to create the encrypted secrets.age bundle
# Run locally. Rotate by re-running. Output is git-safe (age scrypt-encrypted).
#
# Usage: ./create-secrets.sh [output-path]
#   default output: ../secrets.age

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
INFRA_DIR="$(dirname "$SCRIPT_DIR")"
OUTPUT_FILE="${1:-$INFRA_DIR/secrets.age}"

command -v age >/dev/null 2>&1 || { echo "Install age first: brew install age" >&2; exit 1; }

echo "═══════════════════════════════════════════════════════"
echo " Infra Secret Packager"
echo "═══════════════════════════════════════════════════════"
echo ""
echo "Packages secrets into $OUTPUT_FILE (age, passphrase-encrypted)."
echo "Safe to commit. You only need to remember the passphrase."
echo ""

TMPFILE=$(mktemp)
trap 'rm -f "$TMPFILE"' EXIT

cat > "$TMPFILE" <<HEADER
# Infra Secrets
# Generated: $(date -u +%Y-%m-%dT%H:%M:%SZ)
# Re-run create-secrets.sh to rotate.

HEADER

# ─── GitHub PAT (required) ──────────────────────────────────────────────────
echo "─── GitHub Personal Access Token (classic) ───"
echo ""
echo "  Create at: https://github.com/settings/tokens/new"
echo ""
echo "  Required scopes:"
echo "    - repo              (PR checkout / push)"
echo "    - workflow          (if modifying CI)"
echo ""
echo "  Recommended: 90-day expiry, rotate via this script."
echo ""
read -rsp "  GitHub PAT: " GITHUB_PAT
echo ""
[[ -n "$GITHUB_PAT" ]] || { echo "GitHub PAT is required." >&2; exit 1; }
echo "GITHUB_PAT='$GITHUB_PAT'" >> "$TMPFILE"

# ─── Git identity (optional) ────────────────────────────────────────────────
echo ""
echo "─── Git Identity on the VM (Enter to skip) ───"
read -rp "  Name: " GIT_NAME
read -rp "  Email: " GIT_EMAIL
[[ -n "$GIT_NAME"  ]] && echo "GIT_USER_NAME='$GIT_NAME'"   >> "$TMPFILE"
[[ -n "$GIT_EMAIL" ]] && echo "GIT_USER_EMAIL='$GIT_EMAIL'" >> "$TMPFILE"

# ─── Ad-hoc extras ──────────────────────────────────────────────────────────
echo ""
echo "─── Additional Secrets (optional) ───"
echo "  Enter KEY=VALUE pairs, one per line. Empty line to finish."
while true; do
  read -rp "  > " EXTRA
  [[ -z "$EXTRA" ]] && break
  echo "$EXTRA" >> "$TMPFILE"
done

# ─── Encrypt ────────────────────────────────────────────────────────────────
echo ""
echo "Set an age passphrase (you'll enter this when bootstrapping each VM):"
echo ""
age --passphrase --output "$OUTPUT_FILE" "$TMPFILE"

cat <<EOF

═══════════════════════════════════════════════════════
 Encrypted to: $OUTPUT_FILE
 Safe to commit (it's encrypted).
 Apply to VM: ./setup-secrets.sh
═══════════════════════════════════════════════════════
EOF
