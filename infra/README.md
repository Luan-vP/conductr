# Cloud Dev Instance for Orchestrator

A stoppable cloud VM that runs the full Tilt stack (Docker + k3s + Cosmos emulator + backends + frontends + Playwright) for automated PR verification.

## Quick Start

```bash
# 1. Configure provider
echo 'PROVIDER=azure' > ~/.dev-vm.conf

# 2. Build encrypted secrets bundle (one-time; rotate by re-running)
./scripts/create-secrets.sh               # prompts for GitHub PAT, passphrase

# 3. Create the VM (~5 min for cloud-init)
./scripts/dev-vm.sh create

# 4. Upload .env.dev + decrypt secrets + gh auth on VM
./scripts/setup-secrets.sh --env-file /path/to/.env.dev   # prompts for age passphrase

# 5. SSH in and authenticate Claude (mints a VM-scoped token)
./scripts/dev-vm.sh ssh
claude setup-token     # interactive, browser flow
exit

# 6. Clone the repo
./scripts/dev-vm.sh sync-repo

# 7. SSH in and run Tilt
./scripts/dev-vm.sh ssh
cd ~/repo && tilt up
```

### Secrets model

- **`secrets.age`** (git-safe, scrypt-encrypted) holds the GitHub PAT + git
  identity + any ad-hoc `KEY=VALUE` pairs. Rotate by re-running
  `create-secrets.sh`.
- **`setup-secrets.sh`** uploads the ciphertext, decrypts on the VM into a
  shell variable (never to disk), runs `gh auth login --with-token` +
  `gh auth setup-git`, then discards the plaintext.
- **`.env.dev`** (app runtime secrets) is uploaded separately via scp — plain
  file at `~/repo/.env.dev` mode 600.
- **Claude auth** is done **on the VM** via `claude setup-token`. Each VM
  holds its own refresh token chain, so running Claude on the mac doesn't
  invalidate the VM's session.

## Commands

| Command | Description |
|---------|-------------|
| `dev-vm create` | Provision a new VM with cloud-init |
| `dev-vm start` | Start a stopped/deallocated VM (~30s) |
| `dev-vm stop` | Deallocate — pay only for disk (~$5/mo) |
| `dev-vm ssh [args]` | SSH into the VM |
| `dev-vm status` | Show state, IP, uptime |
| `dev-vm destroy` | Delete VM and disk permanently |
| `dev-vm sync-env` | Upload .env.dev to VM |
| `dev-vm sync-repo` | Clone or pull the repo on VM |
| `create-secrets.sh` | Build `secrets.age` locally (gh PAT + git identity) |
| `setup-secrets.sh` | Upload .env.dev, prime Claude, decrypt secrets + auth gh |

## Configuration

Settings are read from `~/.dev-vm.conf` or environment variables:

```bash
# ~/.dev-vm.conf
PROVIDER=azure              # azure or gcp

# VM settings
VM_NAME=dev-orchestrator
VM_USER=dev
SSH_KEY=~/.ssh/id_ed25519
REPO_URL=git@github.com:palindrom/trauma-free-world-mono.git

# Azure-specific
AZURE_RG=dev-orchestrator-rg
AZURE_LOCATION=eastus
AZURE_VM_SIZE=Standard_D4s_v5

# GCP-specific
GCP_PROJECT=my-project-id
GCP_ZONE=us-east1-b
GCP_MACHINE_TYPE=e2-standard-4
```

Environment variables use the `DEV_VM_` prefix (e.g., `DEV_VM_PROVIDER=gcp`).

## What Gets Installed

The VM is provisioned via cloud-init with:

- **Docker** — container runtime
- **k3s** — lightweight single-node Kubernetes
- **Tilt** — dev environment orchestration
- **Node.js 20** — frontend builds
- **Python 3.12 + uv** — backend
- **Claude Code CLI** — AI-assisted development
- **Playwright + Chromium** — browser-based verification
- **gh CLI** — GitHub operations

## Cost

| State | Monthly Cost |
|-------|-------------|
| Running 8h/day, 22 days/mo | ~$40 |
| Running 4h/day, 22 days/mo | ~$20 |
| Stopped (disk only) | ~$5 |
| Destroyed | $0 |

`dev-vm stop` deallocates compute resources. You only pay for disk storage (~$5/mo for 64GB Premium SSD).

## Orchestrator Integration

The orchestrate skill's `manual_verification` flow uses this VM:

1. `dev-vm start` — resume the VM
2. `dev-vm ssh` — connect
3. `gh pr checkout <N>` — check out the PR branch
4. `tilt up` — start all services
5. Run Playwright verification
6. `tilt down` — tear down services
7. `dev-vm stop` — deallocate when done

## Troubleshooting

**Cloud-init still running:**
```bash
dev-vm ssh -- tail -f /var/log/cloud-init-output.log
dev-vm ssh -- ls /home/dev/.cloud-init-complete  # exists when done
```

**Docker/k3s not working after start:**
```bash
dev-vm ssh -- sudo systemctl status docker
dev-vm ssh -- sudo systemctl status k3s
dev-vm ssh -- kubectl get nodes
```

**SSH connection refused:**
```bash
dev-vm status  # check if VM is running
dev-vm start   # start if stopped
```
