#!/usr/bin/env bash
# setup-secrets.sh — One-time secrets upload to the dev VM
#   - Uploads .env.dev
#   - Primes Claude onboarding flags (so `claude setup-token` skips the TUI)
#   - Uploads + decrypts secrets.age and authenticates gh CLI on the VM
#
# Claude auth itself is done interactively on the VM via `claude setup-token`
# so each VM holds its own refresh token (no rotation conflict with the mac).
#
# Usage: ./setup-secrets.sh [options]

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
INFRA_DIR="$(dirname "$SCRIPT_DIR")"
DEV_VM_SH="$SCRIPT_DIR/dev-vm.sh"

ENV_FILE=""
SECRETS_FILE=""
SKIP_ONBOARDING=0
SKIP_ENV=0
SKIP_GH=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --env-file)         ENV_FILE="$2"; shift 2 ;;
    --secrets-file)     SECRETS_FILE="$2"; shift 2 ;;
    --skip-onboarding)  SKIP_ONBOARDING=1; shift ;;
    --skip-env)         SKIP_ENV=1; shift ;;
    --skip-gh)          SKIP_GH=1; shift ;;
    -h|--help)
      cat <<EOF
Usage: setup-secrets.sh [options]

Options:
  --env-file PATH       Path to .env.dev (default: auto-detect)
  --secrets-file PATH   Path to secrets.age (default: $INFRA_DIR/secrets.age)
  --skip-env            Skip uploading .env.dev
  --skip-onboarding     Skip priming ~/.claude.json onboarding flags
  --skip-gh             Skip gh auth sync from secrets.age

After this script, SSH into the VM and run:
  claude setup-token

to authenticate Claude Code with a token scoped to the VM.
EOF
      exit 0
      ;;
    *) echo "Unknown option: $1" >&2; exit 1 ;;
  esac
done

sync_env() {
  if [[ -z "$ENV_FILE" ]]; then
    for candidate in \
      "$SCRIPT_DIR/../../.env.dev" \
      "$HOME/.env.dev" \
      "./.env.dev"; do
      if [[ -f "$candidate" ]]; then
        ENV_FILE="$candidate"
        break
      fi
    done
  fi

  if [[ -z "$ENV_FILE" || ! -f "$ENV_FILE" ]]; then
    echo "Error: .env.dev not found${ENV_FILE:+ at $ENV_FILE}." >&2
    echo "Provide --env-file or place .env.dev in the project root." >&2
    exit 1
  fi

  echo "Uploading .env.dev..."
  "$DEV_VM_SH" sync-env
  echo "  .env.dev uploaded and secured (mode 600)"
}

# Pre-set onboarding flags so `claude setup-token` skips the interactive TUI.
prime_onboarding() {
  local version
  version=$(python3 -c "import json,os; p=os.path.expanduser('~/.claude.json'); d=json.load(open(p)) if os.path.exists(p) else {}; print(d.get('lastOnboardingVersion',''))" 2>/dev/null || true)

  echo "Priming onboarding flags on VM..."
  "$DEV_VM_SH" ssh "LAST_VERSION='$version' python3" <<'PY'
import json, os, pathlib
p = pathlib.Path.home() / ".claude.json"
d = json.loads(p.read_text()) if p.exists() else {}
d["hasCompletedOnboarding"] = True
v = os.environ.get("LAST_VERSION") or ""
if v:
    d["lastOnboardingVersion"] = v
p.write_text(json.dumps(d, indent=2))
PY
  echo "  ~/.claude.json: hasCompletedOnboarding=true${version:+, lastOnboardingVersion=$version}"
}

# Upload secrets.age, decrypt on the VM, authenticate gh. Plaintext never hits disk.
sync_gh() {
  [[ -n "$SECRETS_FILE" ]] || SECRETS_FILE="$INFRA_DIR/secrets.age"

  if [[ ! -f "$SECRETS_FILE" ]]; then
    echo "Skipping gh auth sync: $SECRETS_FILE not found."
    echo "  Create one with: $SCRIPT_DIR/create-secrets.sh"
    return 0
  fi

  echo "Uploading encrypted secrets to VM..."
  "$DEV_VM_SH" ssh 'cat > ~/.secrets.age && chmod 600 ~/.secrets.age' < "$SECRETS_FILE"

  echo "Decrypting on VM and authenticating gh (enter age passphrase)..."
  # -t allocates a PTY so `age` can prompt interactively.
  "$DEV_VM_SH" ssh -t bash -s <<'REMOTE'
set -e
if ! command -v age >/dev/null 2>&1; then
  echo "ERROR: age not installed on VM. Install with: sudo apt-get install -y age" >&2
  exit 1
fi

# Decrypt into a shell var (never touches disk)
SECRETS=$(age --decrypt ~/.secrets.age) || { echo "ERROR: decryption failed" >&2; exit 1; }
rm -f ~/.secrets.age
eval "$SECRETS"
unset SECRETS

if [[ -n "${GITHUB_PAT:-}" ]]; then
  echo "$GITHUB_PAT" | gh auth login --with-token
  gh auth setup-git
  gh auth status 2>&1 | sed 's/^/  /'
fi
[[ -n "${GIT_USER_NAME:-}"  ]] && git config --global user.name  "$GIT_USER_NAME"  && echo "  git user.name  = $GIT_USER_NAME"
[[ -n "${GIT_USER_EMAIL:-}" ]] && git config --global user.email "$GIT_USER_EMAIL" && echo "  git user.email = $GIT_USER_EMAIL"

echo "  gh auth + git identity configured"
REMOTE
}

if (( ! SKIP_ENV ));        then sync_env;        fi
if (( ! SKIP_ONBOARDING )); then prime_onboarding; fi
if (( ! SKIP_GH ));         then sync_gh;         fi

cat <<EOF

Secrets setup complete.

Next: SSH in and authenticate Claude:
  dev-vm ssh
  claude setup-token

Verify with:
  dev-vm ssh -- 'cat ~/repo/.env.dev | head -1'
  dev-vm ssh -- claude auth status
  dev-vm ssh -- gh auth status
EOF
