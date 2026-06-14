---
name: begin
description: Cadence configurator — write default cadence entries or add a skill to .conductr [cadence], then run cadence sync to install cron entries. Function-only: no tmux, no Claude.
cli: conductr begin [<skill> [<schedule>]] [--repo <path>] [--dry-run]
---

# Begin

Configure the cadence schedule for a repository. `begin` is a **function-only** command: it reads and writes `.conductr [cadence]`, then delegates to `cadence sync` to install the cron entries. It never creates tmux sessions or starts Claude.

## Two forms

### Form 1: `conductr begin` (no arguments)

Writes sensible defaults into `.conductr [cadence]` if they are not already present, then runs `cadence sync`.

Defaults written (if absent):

```toml
[cadence]
orchestrate = "*/30 * * * *"
idle        = "*/30 * * * *"
```

If `.conductr` does not exist, `begin` initialises it first (deriving `project_tag` and `repo` from the git remote).

### Form 2: `conductr begin <skill> [schedule]`

Adds a single skill to `[cadence]` with the given cron expression, then runs `cadence sync`.

```bash
conductr begin orchestrate "0 */4 * * *"   # every 4 hours
conductr begin idle "17 * * * *"            # at :17 past each hour
conductr begin architect "0 8 * * 1"        # mondays at 08:00
```

If `<skill>` is already present in `[cadence]`, the existing schedule is kept unchanged and `cadence sync` is still run (to refresh any drift).

Default schedule when `[schedule]` is omitted: `*/30 * * * *`.

## Skill validation

`<skill>` must be a recognised top-level `conductr` subcommand. Invalid names are rejected immediately with a helpful error listing valid options.

## Cron lines produced by `cadence sync`

After `begin` writes entries, `cadence sync` installs cron lines of the form:

```cron
# conductr-cron: <project_tag>-orchestrate
*/30 * * * * bash -lc 'conductr orchestrate --repo <owner/repo> --once' >> ~/.local/share/conductr/orchestrate.log 2>&1

# conductr-cron: <project_tag>-idle
*/30 * * * * bash -lc 'conductr idle' >> ~/.local/share/conductr/idle.log 2>&1
```

Each CLI command is invoked **directly** — there is no `conductr begin` in the cron line. Commands that are Claude-required (like `architect` and `idle`) handle their own tmux + Claude bootstrap when they run.

## Flags

| Flag | Effect |
|------|--------|
| `--repo <path>` | Path to the repo root (defaults to current directory). |
| `--dry-run` | Print what would change without writing anything or running sync. |

## Related

- `conductr cadence sync` — install/refresh cron entries from `.conductr [cadence]`.
- `conductr cadence status` — show installed schedules and drift.
- `conductr cadence remove` — uninstall all scheduled entries for this project.
- `operations/idle.md` — specification for the `idle` command.
- `operations/cadence/orchestration-cycle.md` — the orchestration loop cadence.
