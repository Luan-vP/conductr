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
| `conductr-pod`          | Diagnose and heal the local Claude Code pod (tmux sessions on this host). Exposed via `conductr pod`. |
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
conductr begin                              [--repo <path>] [--dry-run]
conductr begin <skill> [schedule]           [--repo <path>] [--dry-run]
conductr orchestrate --repo owner/repo [--dry-run] [--once] [--poll-secs 60]
conductr instance    spin-up --name <name> | list
conductr schedule    validate <pattern.txt> | render <pattern.txt>
conductr tasks       list [--ready] | create <title> [-p N] | sync-to-notion --database <id>
conductr pod         diagnose|free|heal|save-state
conductr pod diagnose    [--pattern <substr>] [--all] [--json]
conductr pod free        [--pattern <substr>] [--all] [--include-attached] [--json]
conductr pod heal        [--pattern <substr>] [--all] [--dry-run] [--command <cmd>] [--json]
conductr pod save-state  [--pattern <substr>] [--all] [--dry-run] [--no-restart] [--command <cmd>]
conductr local       detect | setup [--provider <name>] [--dry-run]
conductr run-task    --prompt <text> | --prompt-file <path> [--provider <name>] [--model <name>]
```

### Pattern DSL

Set the bar duration and lay out bars (one whole note per bar):

```text
bar_duration 4h

| sleep:w |
| sleep:w |
| sleep:w[wake:e,sleep:e,wake:e,sleep:e,wake:e,sleep:e,wake:e,sleep:e] |
| wake:w |
| work:w |
| rest:w |
```

- `bar_duration` sets wall-clock time for one bar (default: 4h).
- `w h q e s t x` = whole, half, quarter, eighth, 16th, 32nd, 64th.
- With `bar_duration 4h`: whole = 4h, eighth = 30 min.
- `tag:value` is one beat; `tag:value[child,child,...]` is a subdivided beat.
- Validation enforces that subdivisions sum to the parent and all bars have the same total duration.
- A day is **6 bars** (`6 × 4h = 24h`).

`conductr schedule render examples/conductor_life_day.pattern` produces:

```text
Pattern: 4/4 time, q=1h
──────────────────────────────────────────────
        0s  +4h          w  sleep
        4h  +4h          w  sleep
        8h  +30m         e  wake
     8h30m  +30m         e  sleep
        9h  +30m         e  wake
     9h30m  +30m         e  sleep
       10h  +30m         e  wake
    10h30m  +30m         e  sleep
       11h  +30m         e  wake
    11h30m  +30m         e  sleep
       12h  +4h          w  wake
       16h  +4h          w  work
       20h  +4h          w  rest
```

### Begin (cadence configurator)

`conductr begin` is a **function-only** command that configures `.conductr [cadence]`
and then runs `cadence sync` to install the cron entries. It never touches tmux or
starts Claude.

**Form 1 — no arguments:** write defaults and sync.

```bash
conductr begin
```

Ensures `orchestrate` and `idle` are present in `.conductr [cadence]` (with the
default schedule `*/30 * * * *`), then runs `cadence sync` to install the cron lines.
If `.conductr` does not exist it is initialised first.

**Form 2 — add a specific skill:**

```bash
conductr begin orchestrate "0 */4 * * *"   # every 4 hours
conductr begin idle        "17 * * * *"    # at :17 past each hour
conductr begin architect   "0 8 * * 1"     # mondays at 08:00
```

Adds `<skill>` to `[cadence]` with the given cron expression (default `*/30 * * * *`
if omitted), then syncs. The skill must be a recognised top-level `conductr` command.

After running `begin`, the installed cron lines invoke each CLI command directly:

```cron
# conductr-cron: conductr-orchestrate
*/30 * * * * bash -lc 'conductr orchestrate --repo Luan-vP/conductr --once' >> ~/.local/share/conductr/orchestrate.log 2>&1

# conductr-cron: conductr-idle
*/30 * * * * bash -lc 'conductr idle' >> ~/.local/share/conductr/idle.log 2>&1
```

Commands that are Claude-required (like `architect` and `idle`) handle their own
tmux + Claude bootstrap when invoked from cron.

Use `--dry-run` to preview changes without writing:

```bash
conductr begin --dry-run
# plan: would write defaults (orchestrate, idle) to .conductr [cadence] if absent
# plan: would run cadence sync
```

**Relationship to #19 (tempo):** use your project's tempo bucket to pick
the cron interval — e.g. a 30-minute tempo maps to `*/30 * * * *`.

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

### Pod (diagnose / free / heal / save-state)

`conductr` treats the set of `claude` agents running in tmux on the local host
as a "pod". The `conductr pod` parent command groups the subcommands that
inspect and recover it:

- `conductr pod diagnose` — for each tmux session whose name contains `claude`,
  capture the rendered pane and classify the agent as `idle`, `working`,
  `crashed`, or `unknown`. Pass `--all` to inspect every tmux session,
  `--pattern <s>` to override the name filter, and `--json` for
  machine-readable output.
- `conductr pod free` — find an idle Claude Code session and print its tmux
  attach command. Exits non-zero if no idle session is found.
- `conductr pod heal` — restart any session diagnosed as `crashed` by typing
  `claude` (or `--command <cmd>`) into its pane. Use `--dry-run` to see the
  plan first.
- `conductr pod save-state` — graceful pod restart. For each session it writes a
  `[thread-recovery:<session>]` issue to beads (priority 2 by default,
  labelled `thread-recovery,<session>`) carrying the last user message and
  pane tail, then sends `/exit` followed by the relaunch command. Pass
  `--no-restart` to capture without restarting, or `--dry-run` to plan
  without writing. Output is a JSON manifest the
  [`pod` skill](skills/pod/SKILL.md) consumes to mirror the
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

To bring any repository to **L5 Orchestrated**:

```bash
conductr setup wizard            # step through the maturity checklist
conductr setup status            # verify: should report L5 Orchestrated
```

The wizard automates fixable checks (CI workflow, `.gitignore`, CODEOWNERS,
`.github/workflows/claude.yml`) and prints a URL for the one manual step
(installing the Claude GitHub App).

Once the repo reaches L5, run `conductr begin` at the repo root. It will
initialise `.conductr` (if absent), write default cadence entries, and install
the cron lines in one shot:

```bash
conductr begin
```

This installs cron entries that invoke `conductr orchestrate` and `conductr idle`
directly. **This repo** uses:

```toml
# .conductr [cadence]
orchestrate = "*/30 * * * *"
idle        = "*/30 * * * *"
```

Which produces cron lines of the form:

```cron
*/30 * * * * bash -lc 'conductr orchestrate --repo Luan-vP/conductr --once' >> ~/.local/share/conductr/orchestrate.log 2>&1
*/30 * * * * bash -lc 'conductr idle' >> ~/.local/share/conductr/idle.log 2>&1
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
