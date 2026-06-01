---
name: setup
description: Check and improve repository maturity, provision projects from the machine-wide registry, and configure the Claude GitHub App. Wraps `conductr setup status`, `conductr setup wizard`, `conductr setup install-claude-app`, and `conductr setup spawn`.
---

# setup

Thin skill stub that invokes the `conductr setup` subcommands verbatim.
All logic lives in the CLI binary (`conductr-setup` crate + spawn logic).

## Subcommands

### status

Report the maturity level of a repository.

```
conductr setup status [--repo <path>]
```

### wizard

Interactively bring a repository to a higher maturity level.

```
conductr setup wizard [--repo <path>] [--non-interactive] [--dry-run]
```

### install-claude-app

Open the Claude GitHub App install page for the repository.

```
conductr setup install-claude-app [--repo <path>]
```

### spawn

Provision all active projects in the machine-wide `~/.conductr` registry.
Idempotent: safe to run repeatedly.

```
conductr setup spawn [<tag>] [--dry-run] [--include-pending] [--registry <path>] [--command <cmd>] [--no-launch]
```

- **No arguments** — provision every `status = "active"` project in the registry.
- **`<tag>`** — provision only the named project.
- **`--dry-run`** — print the plan without making any changes.
- **`--include-pending`** — also attempt pending entries (useful for testing the clone path).
- **`--registry <path>`** — override the registry path (default `~/.conductr`).
- **`--command <cmd>`** — override the command booted into freshly-created sessions
  (default `claude --remote-control --permission-mode auto`).
- **`--no-launch`** — create sessions but leave them at a shell prompt.

Each project runs four idempotent steps, reported per-step in the output
(`+` done · `⋯` planned (dry-run) · `·` already present · `✗` error):

1. **clone** — git-clone the repo if its local `path` is missing.
2. **dot-conductr** — generate `.conductr` from registry `[defaults]` if absent.
3. **cadence** — `cadence sync` to install the host crontab markers.
4. **session** — create the `conductr-<tag>` tmux session if missing, then
   **launch** the boot command into it. An *existing* session is never
   relaunched. Freshly-created sessions come up with Remote Control enabled in
   `auto` permission mode, so they're immediately driveable by orchestrate.

Per-project summary line:
- `✓ already provisioned` — clone, `.conductr`, cron entry, and tmux session all present.
- `↺ provisioned this pass` — one or more steps were performed this run.
- `⚠ skipped (<reason>)` — clone failed; project skipped without aborting the pass.

Because every step is idempotent, `setup spawn` doubles as a health check — re-run
it to bring any dropped sessions back. `pod heal` builds on this same machinery,
adding a liveness pass that restarts sessions whose Claude has crashed.

## Registry schema

The machine-wide registry at `~/.conductr` is a TOML file:

```toml
[defaults]
human_assignee      = "Luan-vP"
local_provider      = "ollama"
cadence_orchestrate = "*/30 * * * *"
cadence_idle        = "17 * * * *"

[[projects]]
tag    = "conductr"
repo   = "Luan-vP/conductr"
path   = "/home/dev/developer/conductr"
status = "active"   # or "pending"
```

See `docs/registry.md` for the full schema reference.
