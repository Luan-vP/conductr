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
conductr setup spawn [<tag>] [--dry-run] [--include-pending] [--registry <path>]
```

- **No arguments** — provision every `status = "active"` project in the registry.
- **`<tag>`** — provision only the named project.
- **`--dry-run`** — print the plan without making any changes.
- **`--include-pending`** — also attempt pending entries (useful for testing the clone path).
- **`--registry <path>`** — override the registry path (default `~/.conductr`).

Per-project status icons in the output:
- `✓ already provisioned` — clone, `.conductr`, cron entry, and tmux session all present.
- `↺ provisioned this pass` — one or more steps were performed this run.
- `⚠ skipped (<reason>)` — clone failed; project skipped without aborting the pass.

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
