```
        ..,,,,,..
    .,;;;;;;;;;;;;;;,.
  ,;;;'            `;;;;,       ,,
 ,;'                 ';;;;,    ;;;;
.;.;;;;,               ;;;;;.   ''
;;;;;;;;                ;;;;;
`;;;;;;'                ;;;;;
                        ;;;;'   ,,
                      .;;;;'   ;;;;
                     ,;;;'      ''
                   ,;;;'
                ,;;;'
            .;;;;'
        .,;;;''
    .,;;''
```

# conductr

Scheduling and orchestration for agents and people.

---

A Rust workspace that bundles four concerns into one CLI:

| Crate                   | Concern                                                                 |
|-------------------------|-------------------------------------------------------------------------|
| `conductr`              | Binary CLI; constructs and wires adapters into use-case crates.         |
| `conductr-core`         | Shared domain types and port traits (`IssueTracker`, `ScmHost`, `TmuxAgent`, `InstanceProvider`, `Mailbox`). No I/O. |
| `conductr-adapters`     | Feature-gated concrete connectors (`tmux`, `beads`, `notion`, `gh-cli`, `mail-fs`, `mail-github`, `mock`). Enable all with `--features full`. |
| `conductr-orchestrate`  | Drive `@claude` GitHub-issue implementation in dependency order. Mirrors [`skills/orchestrate/SKILL.md`](skills/orchestrate/SKILL.md). |
| `conductr-instance`     | Cloud instance spin-up & SSH (**stubbed**).                             |
| `conductr-pod`          | Diagnose and heal the local Claude Code pod (tmux sessions on this host). |
| `conductr-schedule`     | Time patterns described in **musical notation** (the seed concept).     |
| `conductr-tasks`        | Task tracking via [beads_rust] (`br`, local SQLite + JSONL) and Notion. |
| `conductr-mail`         | Agent scope dedup and parallel-synthesis substrate.                     |
| `conductr-setup`        | Project maturity model (L0–L5) and `conductr setup` wizard.             |

[beads_rust]: https://github.com/Dicklesworthstone/beads_rust

## Submodules (vendored references)

```
vendor/
└── beads_rust       # MIT — `br` source, used as a CLI subprocess
```

> **Security note.** Submodules are pinned to a commit SHA and `vendor/*` is
> *not* a Cargo workspace member, so cargo never compiles or runs build.rs
> from these paths.

## Setup

```bash
git clone --recurse-submodules https://github.com/Luan-vP/conductr.git
cd conductr && cargo build --workspace --release
./target/release/conductr --help
```

## Build

```bash
cargo build --workspace
cargo test  --workspace
```

To build with all adapters enabled:

```bash
cargo build --workspace --features full
```

## Architecture

`conductr` follows a **hexagonal (ports & adapters)** layout. The CLI binary
and in-Claude skills both call the same use-case crates; those crates depend
only on a shared core (types + port traits); concrete connectors live behind
feature flags in `conductr-adapters`.

```
              ┌────────────────────────────────────┐
  driving     │  crates/conductr   (binary, CLI)   │
              │  skills/*          (markdown)      │
              └─────────────────┬──────────────────┘
                                ▼
              ┌────────────────────────────────────┐
  use-cases   │  crates/conductr-orchestrate       │
  (arms)      │  crates/conductr-pod               │
              │  crates/conductr-tasks             │
              │  crates/conductr-instance          │
              │  crates/conductr-schedule  (pure)  │
              │  crates/conductr-mail              │
              │  crates/conductr-setup             │
              └─────────────────┬──────────────────┘
                                ▼
              ┌────────────────────────────────────┐
  core        │  crates/conductr-core              │
              │   ::types  (domain models)         │
              │   ::ports  (trait surface)         │
              └─────────────────┬──────────────────┘
                                ▼
              ┌────────────────────────────────────┐
  adapters    │  crates/conductr-adapters          │
  (folds)     │   feature: tmux                    │
              │   feature: beads                   │
              │   feature: notion                  │
              │   feature: gh-cli                  │
              │   feature: mail-fs                 │
              │   feature: mail-github             │
              │   feature: mock                    │
              └────────────────────────────────────┘
```

**Where to look when you want to…**

| Goal | Crate |
|------|-------|
| Add a Linear or Jira connector | `conductr-adapters` — add a new adapter behind a feature flag |
| Change save-state session classification | `conductr-pod` — the use-case logic |
| Rename a domain field (`Task`, `Issue`, `Diagnosis`, …) | `conductr-core::types` |
| Change the CLI surface | `conductr` — the binary |

Adapters are compiled only when their feature flag is set; use `--features full`
to enable all of them at once. Full architecture details, port definitions, and
the six design rules live in [`.claude/base.md`](.claude/base.md).

The process doctrine that governs orchestration — the cadence of the loop,
how PRs flow, how dependencies are resolved, the safety invariants — lives
in [`operations/`](operations/).

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
conductr local       detect | setup [--provider <name>] [--dry-run]
conductr run-task    --prompt <text> | --prompt-file <path> [--provider <name>] [--model <name>]
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

### Local providers

`conductr local` manages local AI provider installations (ollama, llama.cpp, Pi agent) on the current host.

#### Detect → setup loop

```bash
# 1. Check which providers are installed
conductr local detect
# PROVIDER     STATUS
# ollama       missing
# llamacpp     missing
# pi           missing

# 2. Preview what setup would do
conductr local setup --provider ollama --dry-run
# plan: would run /path/to/scripts/local/install-ollama-linux.sh

# 3. Run the install
conductr local setup --provider ollama
# running: /path/to/scripts/local/install-ollama-linux.sh
# ... (brew / curl install output)

# 4. Pull the default model (qwen3 27B)
bash scripts/local/pull-qwen3-27b.sh
# Pulling qwen3:27b (approximately 16 GB)...

# 5. Install all missing providers at once
conductr local setup
```

#### Available providers

| Provider | Binary checked | macOS script | Linux script |
|----------|---------------|-------------|-------------|
| `ollama` | `ollama` | Homebrew install + `ollama serve` | Official `curl \| sh` + systemd/background |
| `llamacpp` | `llama-server` | Homebrew `llama.cpp` | Clone + cmake build → `~/.local/bin` |
| `pi` | `pi` | placeholder (awaits #57) | placeholder (awaits #57) |

All install scripts are **idempotent** — re-running them exits 0 if the target is already installed.

#### Script location

`conductr local setup` discovers `scripts/local/` by walking up from the current directory.
Override with `CONDUCTR_SCRIPTS_DIR=/path/to/scripts/local`.

#### Running a task through a local provider

```bash
# Dispatch a prompt to ollama and print the response.
conductr run-task --provider ollama --prompt "summarise the current git diff"

# Use a prepared prompt file instead of inline text.
conductr run-task --provider ollama --prompt-file .conductr/prompts/triage.md

# Let conductr pick the provider automatically
# (first present from `conductr local detect`).
conductr run-task --prompt "say hi"

# Override provider via environment variable.
CONDUCTR_LOCAL_PROVIDER=llamacpp conductr run-task --prompt "say hi"

# Specify a custom ollama model.
conductr run-task --provider ollama --model llama3 --prompt "say hi"
```

Provider precedence (highest to lowest):

1. `--provider` CLI flag
2. `CONDUCTR_LOCAL_PROVIDER` environment variable
3. `[local].provider` in `.conductr`
4. First provider reported as `present` by `conductr local detect`

Set a project-level default by adding a `[local]` section to `.conductr`:

```toml
[local]
provider = "ollama"
model    = "qwen3:27b"   # ollama only; defaults to qwen3:27b when omitted
```

### Instance

The `conductr-instance` crate currently exposes the trait surface
(`InstanceManager` + `InstanceSpec` + `Provider::{Aws,Hetzner,DigitalOcean,Local}`)
and a `StubManager` that returns `NotImplemented`. Full provider impls are
future work.

## Bootstrap

To bring any repository to **L5 Orchestrated** and hand it off to `conductr begin`:

```bash
conductr setup wizard            # step through the maturity checklist
conductr setup status            # verify: should report L5 Orchestrated
```

The wizard automates fixable checks (CI workflow, `.gitignore`, CODEOWNERS,
`.github/workflows/claude.yml`) and prints a URL for the one manual step
(installing the Claude GitHub App).

Once the repo reaches L5, create `.conductr` at the repo root (see the file
in this repo for the canonical schema), then add a cron line:

```cron
0 */4 * * * conductr begin --tag <tag> --repo <owner/repo>
```

**This repo** is bootstrapped as:

```cron
0 */4 * * * conductr begin --tag conductr --repo Luan-vP/conductr
```

Project configuration lives in `.conductr` at the repo root. `[tempo]` entries
are appended after each successful orchestrate pass.

## Project layout

```text
.
├── .conductr                        # project config (tag, band, tempo log)
├── Cargo.toml                       # workspace root
├── README.md
├── .claude/
│   └── base.md                      # hexagonal architecture reference
├── docs/
├── examples/
│   └── conductor_life_day.pattern
├── operations/                      # process doctrine (cadence + PR ops)
├── scripts/
│   └── local/                       # idempotent provider install scripts (mac + linux)
├── skills/
├── crates/
│   ├── conductr/                    # binary (driving adapter)
│   ├── conductr-core/               # domain types + port traits (no I/O)
│   ├── conductr-adapters/           # feature-gated concrete adapters
│   ├── conductr-orchestrate/        # use-case: orchestrate GitHub issues
│   ├── conductr-instance/           # use-case: cloud instance management
│   ├── conductr-pod/                # use-case: diagnose/heal Claude pod
│   ├── conductr-schedule/           # use-case: musical-notation schedules
│   ├── conductr-tasks/              # use-case: task tracking
│   ├── conductr-mail/               # use-case: agent scope dedup + synthesis
│   └── conductr-setup/              # use-case: project maturity wizard
└── vendor/
    └── beads_rust/                  # submodule
```
