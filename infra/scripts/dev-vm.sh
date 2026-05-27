#!/usr/bin/env bash
# dev-vm.sh — Unified CLI for managing the dev orchestrator VM
# Usage: dev-vm <command> [options]
#
# Commands:
#   create       Provision a new VM with cloud-init
#   start        Start a stopped/deallocated VM
#   stop         Stop and deallocate the VM (pay only for disk)
#   ssh          SSH into the VM
#   status       Show VM state, IP, and uptime
#   destroy      Delete the VM and all associated resources
#   sync-env     Upload .env.dev to the VM
#   sync-repo    Clone or pull the repo on the VM

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
INFRA_DIR="$(dirname "$SCRIPT_DIR")"
PROVIDERS_DIR="$INFRA_DIR/providers"

# --- Configuration ---

CONFIG_FILE="${DEV_VM_CONFIG:-$HOME/.dev-vm.conf}"

# Load config if it exists
if [[ -f "$CONFIG_FILE" ]]; then
  # shellcheck source=/dev/null
  source "$CONFIG_FILE"
fi

# Provider: azure or gcp (env var overrides config file)
PROVIDER="${DEV_VM_PROVIDER:-${PROVIDER:-gcp}}"

# VM settings (can be overridden in config)
VM_NAME="${DEV_VM_NAME:-dev-orchestrator}"
VM_USER="${DEV_VM_USER:-dev}"
SSH_KEY="${DEV_VM_SSH_KEY:-$HOME/.ssh/id_ed25519}"
REPO_URL="${DEV_VM_REPO_URL:-git@github.com:palindrom/trauma-free-world-mono.git}"
REPO_DIR="${DEV_VM_REPO_DIR:-/home/$VM_USER/repo}"
ENV_FILE="${DEV_VM_ENV_FILE:-$INFRA_DIR/../.env.dev}"

# --- Provider loading ---

load_provider() {
  local provider_script="$PROVIDERS_DIR/$PROVIDER.sh"
  if [[ ! -f "$provider_script" ]]; then
    echo "Error: Unknown provider '$PROVIDER'. Expected file: $provider_script" >&2
    echo "Available providers:" >&2
    ls "$PROVIDERS_DIR"/*.sh 2>/dev/null | xargs -I{} basename {} .sh | sed 's/^/  /' >&2
    exit 1
  fi
  # shellcheck source=/dev/null
  source "$provider_script"
}

# --- Commands ---

cmd_create() {
  echo "Creating VM '$VM_NAME' with provider '$PROVIDER'..."

  if [[ ! -f "$SSH_KEY.pub" ]]; then
    echo "Error: SSH public key not found at $SSH_KEY.pub" >&2
    echo "Generate one with: ssh-keygen -t ed25519" >&2
    exit 1
  fi

  # Inject the SSH public key into cloud-init
  local cloud_init="$INFRA_DIR/cloud-init.yaml"
  local tmp_cloud_init
  tmp_cloud_init=$(mktemp)
  SSH_PUB_KEY=$(cat "$SSH_KEY.pub")
  sed "s|\${SSH_PUBLIC_KEY}|$SSH_PUB_KEY|" "$cloud_init" > "$tmp_cloud_init"

  provider_create "$tmp_cloud_init"
  rm -f "$tmp_cloud_init"

  echo ""
  echo "VM created. Cloud-init is running (~5 min)."
  echo "Check progress with: dev-vm ssh -- tail -f /var/log/cloud-init-output.log"
  echo "Completion marker: dev-vm ssh -- ls /home/$VM_USER/.cloud-init-complete"
}

cmd_start() {
  echo "Starting VM '$VM_NAME'..."
  provider_start
  echo "VM started. Waiting for SSH..."
  wait_for_ssh
  echo "VM is ready."
}

cmd_stop() {
  echo "Stopping and deallocating VM '$VM_NAME'..."
  provider_stop
  echo "VM deallocated. You're only paying for disk storage now."
}

cmd_ssh() {
  local ip ssh_flags=()
  # Accept leading ssh flags (-t, -tt, -A) before the remote command
  while [[ $# -gt 0 && "$1" =~ ^-(t|tt|A)$ ]]; do
    ssh_flags+=("$1")
    shift
  done
  ip=$(provider_get_ip)
  if [[ -z "$ip" ]]; then
    echo "Error: Could not get VM IP. Is the VM running?" >&2
    echo "Try: dev-vm start" >&2
    exit 1
  fi
  ssh -o StrictHostKeyChecking=accept-new -i "$SSH_KEY" "${ssh_flags[@]}" "$VM_USER@$ip" "$@"
}

cmd_status() {
  provider_status
}

cmd_destroy() {
  echo "WARNING: This will permanently delete VM '$VM_NAME' and all its data."
  read -rp "Are you sure? (yes/no): " confirm
  if [[ "$confirm" != "yes" ]]; then
    echo "Aborted."
    exit 0
  fi
  provider_destroy
  echo "VM destroyed."
}

cmd_sync_env() {
  if [[ ! -f "$ENV_FILE" ]]; then
    echo "Error: .env.dev not found at $ENV_FILE" >&2
    echo "Set DEV_VM_ENV_FILE or place it at $ENV_FILE" >&2
    exit 1
  fi

  local ip
  ip=$(provider_get_ip)
  if [[ -z "$ip" ]]; then
    echo "Error: Could not get VM IP. Is the VM running?" >&2
    exit 1
  fi

  echo "Uploading .env.dev to VM..."
  scp -o StrictHostKeyChecking=accept-new -i "$SSH_KEY" "$ENV_FILE" "$VM_USER@$ip:$REPO_DIR/.env.dev"
  cmd_ssh chmod 600 "$REPO_DIR/.env.dev"
  echo "Done. .env.dev uploaded to $REPO_DIR/.env.dev"
}

cmd_sync_repo() {
  local ip
  ip=$(provider_get_ip)
  if [[ -z "$ip" ]]; then
    echo "Error: Could not get VM IP. Is the VM running?" >&2
    exit 1
  fi

  echo "Syncing repo on VM..."
  cmd_ssh bash -c "'
    if [[ -d $REPO_DIR/.git ]]; then
      cd $REPO_DIR && git fetch --all && git pull
    else
      git clone $REPO_URL $REPO_DIR
    fi
  '"
  echo "Repo synced at $REPO_DIR"
}

# --- Helpers ---

wait_for_ssh() {
  local ip
  local max_attempts=30
  local attempt=0

  while (( attempt < max_attempts )); do
    ip=$(provider_get_ip 2>/dev/null || true)
    if [[ -n "$ip" ]]; then
      if ssh -o StrictHostKeyChecking=accept-new -o ConnectTimeout=5 -i "$SSH_KEY" "$VM_USER@$ip" true 2>/dev/null; then
        return 0
      fi
    fi
    attempt=$((attempt + 1))
    sleep 5
  done

  echo "Error: SSH not available after $((max_attempts * 5))s" >&2
  exit 1
}

usage() {
  cat <<'EOF'
Usage: dev-vm <command> [options]

Commands:
  create       Provision a new VM with cloud-init
  start        Start a stopped/deallocated VM
  stop         Stop and deallocate the VM (pay only for disk)
  ssh [args]   SSH into the VM (extra args passed to ssh)
  status       Show VM state, IP, and uptime
  destroy      Delete the VM and all associated resources
  sync-env     Upload .env.dev to the VM
  sync-repo    Clone or pull the repo on the VM

Configuration:
  Provider and settings are read from ~/.dev-vm.conf or env vars.
  See README.md for details.

Environment variables:
  DEV_VM_PROVIDER   azure|gcp (default: azure)
  DEV_VM_NAME       VM name (default: dev-orchestrator)
  DEV_VM_SSH_KEY    Path to SSH private key (default: ~/.ssh/id_ed25519)
  DEV_VM_ENV_FILE   Path to .env.dev file
  DEV_VM_CONFIG     Path to config file (default: ~/.dev-vm.conf)
EOF
}

# --- Main ---

load_provider

COMMAND="${1:-}"
shift || true

case "$COMMAND" in
  create)    cmd_create "$@" ;;
  start)     cmd_start "$@" ;;
  stop)      cmd_stop "$@" ;;
  ssh)       cmd_ssh "$@" ;;
  status)    cmd_status "$@" ;;
  destroy)   cmd_destroy "$@" ;;
  sync-env)  cmd_sync_env "$@" ;;
  sync-repo) cmd_sync_repo "$@" ;;
  help|-h|--help) usage ;;
  *)
    echo "Error: Unknown command '$COMMAND'" >&2
    usage >&2
    exit 1
    ;;
esac
