# conductr

Scheduling and orchestration for agents and people.

> *"Don't be a dictator, be a conductor."*

A Rust workspace that bundles four concerns into one CLI:

| Crate                   | Concern                                                                 |
|-------------------------|-------------------------------------------------------------------------|
| `conductr`              | Binary CLI that dispatches to the others.                               |
| `conductr-orchestrate`  | Port of [poorchestrator] — drive `@claude` GitHub-issue implementation in dependency order.  |
| `conductr-instance`     | Cloud instance spin-up & SSH (port of `agentic`; **stubbed**).          |
| `conductr-pod`          | Diagnose and heal the local Claude Code pod (tmux sessions on this host). |
| `conductr-schedule`     | Time patterns described in **musical notation** (the seed concept).     |
| `conductr-tasks`        | Task tracking via [beads_rust] (`br`, local SQLite + JSONL) and Notion. |

[poorchestrator]: https://github.com/Luan-vP/poorchestrator
[beads_rust]: https://github.com/Dicklesworthstone/beads_rust

## Submodules (vendored references)

```
vendor/
├── poorchestrator   # MIT — orchestrate skill we ported
└── beads_rust       # MIT — `br` source, used as a CLI subprocess
```

`Luan-vP/agentic` is private; this sandbox lacks GitHub auth so it has not been
added here. To add it locally:

```bash
git submodule add git@github.com:Luan-vP/agentic.git vendor/agentic
git submodule update --init --recursive
```

> **Security note.** Submodules are pinned to a commit SHA and `vendor/*` is
> *not* a Cargo workspace member, so cargo never compiles or runs build.rs from
> these paths. They're inert reference material. The poorchestrator markdown
> describes auto-merging behaviour that is prompt-injection-prone; we ported
> the algorithm into Rust code rather than loading the markdown into an LLM.

## Build

```bash
cargo build --workspace
cargo test  --workspace
```

## CLI

```text
conductr begin       --tag <tag> [--repo <owner/repo>] [--cwd <path>] [--continuous] [--dry-run]
conductr orchestrate --repo owner/repo [--dry-run] [--once] [--poll-secs 60]
conductr instance    spin-up --name <name> | list
conductr schedule    validate <pattern.txt> | render <pattern.txt>
conductr tasks       list [--ready] | create <title> [-p N] | sync-to-notion --database <id>
conductr diagnose    [--pattern <substr>] [--all] [--json]
conductr heal        [--pattern <substr>] [--all] [--dry-run] [--command <cmd>] [--json]
conductr save-state  [--pattern <substr>] [--all] [--dry-run] [--no-restart] [--command <cmd>]
```

### Pattern DSL

Anchor the day to a quarter-note duration and lay out beats per bar:

```text
time_signature 6/4
quarter_duration 4h

| sleep:q | sleep:q | sleep:q[wake:t,sleep:t,wake:t,sleep:t,wake:t,sleep:t,wake:t,sleep:t] | wake:q | work:q | rest:q |
```

- `w h q e s t x` = whole, half, quarter, eighth, 16th, **32nd (demisemiquaver)**, 64th.
- `tag:value` is one beat; `tag:value[child,child,...]` is a subdivided beat.
- Validation enforces that subdivisions sum to the parent and bars sum to the time signature.

`conductr schedule render examples/conductor_life_day.pattern` produces:

```text
Pattern: 6/4 time, q=4h
──────────────────────────────────────────────
        0s  +4h          q  sleep
        4h  +4h          q  sleep
        8h  +30m         t  wake
     8h30m  +30m         t  sleep
        9h  +30m         t  wake
     9h30m  +30m         t  sleep
       10h  +30m         t  wake
    10h30m  +30m         t  sleep
       11h  +30m         t  wake
    11h30m  +30m         t  sleep
       12h  +4h          q  wake
       16h  +4h          q  work
       20h  +4h          q  rest
```

### Begin (cron entry point)

`conductr begin` is the single command you put in a cron line. It is
idempotent and safe to fire on a schedule.

```cron
# Trigger an orchestrate pass every 30 minutes for the conductr repo.
*/30 * * * * conductr begin --tag conductr --repo Luan-vP/conductr
```

What it does on each tick:

1. **Session lookup / create** — looks for a tmux session named
   `conductr-<tag>`. If absent, creates it with
   `tmux new-session -d -s conductr-<tag>`.
2. **Health check** — classifies the session via the same heuristics as
   `conductr diagnose`.
   - `working` → logs "already busy" and exits 0 (next tick will retry).
   - `crashed` → restarts Claude before sending the prompt.
   - `idle` or `created` → proceeds to send the orchestrate prompt.
3. **Orchestrate trigger** — sends
   `conductr orchestrate --repo <owner/repo> --once` as the user prompt.
   With `--continuous`, the `--once` flag is omitted and Claude drives its
   own polling loop.
4. **Exits 0** — whether it acted or skipped. Non-zero only on real errors
   (tmux not installed, bad config, etc.).

**Auto mode** is enabled via the `--dangerously-skip-permissions` CLI flag
passed to Claude Code on startup. This makes Claude auto-approve all tool
calls — the right choice for unattended cron operation.

**Remote control** is tmux-based: `conductr begin` uses
`tmux send-keys` to inject the orchestrate prompt into the session's active
pane, exactly as `conductr heal` and `conductr save-state` do.

**Lockfile** at `~/.conductr/begin-<tag>.lock` (PID-based) prevents two
cron ticks from racing on the same tag. A stale lock (PID dead) is silently
reaped and the tick proceeds.

Use `--dry-run` to inspect the current session state and see the plan
without touching tmux:

```bash
conductr begin --tag conductr --repo Luan-vP/conductr --dry-run
# plan: session 'conductr-conductr' does not exist
# plan: cwd = /home/user/conductr
# plan:  would create session 'conductr-conductr'
# plan:  would start Claude: `claude --dangerously-skip-permissions`
# plan:  would send: `conductr orchestrate --repo Luan-vP/conductr --once`
```

**Relationship to #19 (tempo):** use your project's tempo bucket to pick
the cron interval — e.g. a 30-minute tempo maps to `*/30 * * * *`.

**Relationship to #22 (tag namespacing):** `--tag` is currently a free
string; once #22 lands the convention will be documented there and `begin`
will consume whatever `--tag` semantics #22 defines.

### Orchestrate

`conductr orchestrate --repo owner/repo` shells out to the [`gh` CLI] to:
1. List open issues + open PRs.
2. Parse `depends on #N` / `blocked by #N` / `after #N` / `requires #N` /
   `- [ ] #N must be done first` from issue bodies.
3. Bucket each issue (Ready, PrOpen, PrFailing, Blocked, TriggeredWaiting,
   Human, AlreadyClosed).
4. Merge PRs whose CI is green; trigger `@claude please implement` on Ready
   issues; assign `human`-labelled issues; loop on `--poll-secs`.

Use `--dry-run` to print the plan without acting.

[`gh` CLI]: https://cli.github.com/

### Tasks

`conductr-tasks` shells out to [`br`][beads_rust] (no Rust path-dependency, so
cargo doesn't try to compile beads transitively) and exposes a Notion REST
client. `conductr tasks sync-to-notion --database <id>` reads `br list --json`
and pushes each task into a Notion database.

Set `NOTION_API_KEY` to a Notion integration token before sync.

### Pod (diagnose / heal)

`conductr` treats the set of `claude` agents running in tmux on the local host
as a "pod". Two subcommands inspect and recover it:

- `conductr diagnose` — for each tmux session whose name contains `claude`,
  capture the rendered pane and classify the agent as `idle`, `working`,
  `crashed`, or `unknown`. Pass `--all` to inspect every tmux session,
  `--pattern <s>` to override the name filter, and `--json` for
  machine-readable output.
- `conductr heal` — restart any session diagnosed as `crashed` by typing
  `claude` (or `--command <cmd>`) into its pane. Use `--dry-run` to see the
  plan first.
- `conductr save-state` — graceful pod restart. For each session it writes a
  `[thread-recovery:<session>]` issue to beads (priority 2 by default,
  labelled `thread-recovery,<session>`) carrying the last user message and
  pane tail, then sends `/exit` followed by the relaunch command. Pass
  `--no-restart` to capture without restarting, or `--dry-run` to plan
  without writing. Output is a JSON manifest the
  [`conductr-pod` skill](skills/conductr-pod/SKILL.md) consumes to mirror the
  recovery issues into Notion (the binary deliberately stops at beads).

Classification is purely textual: a session counts as alive iff the pane
shows the Claude Code TUI (banner / `❯` prompt / status footer). A session
counts as `working` iff a spinner glyph (`✻ …`) appears below the most
recent prompt; otherwise alive sessions are `idle`. Anything else (a shell
prompt, the `[Process completed]` footer) is `crashed`.

### Instance

The `conductr-instance` crate currently exposes the trait surface
(`InstanceManager` + `InstanceSpec` + `Provider::{Aws,Hetzner,DigitalOcean,Local}`)
and a `StubManager` that returns `NotImplemented`. The full provider impls
will be ported once `vendor/agentic` is wired up.

## Project layout

```text
.
├── Cargo.toml                       # workspace root
├── README.md
├── examples/
│   └── conductor_life_day.pattern
├── crates/
│   ├── conductr/                    # binary
│   ├── conductr-orchestrate/
│   ├── conductr-instance/
│   ├── conductr-pod/
│   ├── conductr-schedule/
│   └── conductr-tasks/
└── vendor/
    ├── poorchestrator/              # submodule
    └── beads_rust/                  # submodule
```
