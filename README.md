# conductr

Scheduling and orchestration for agents and people.

> *"Don't be a dictator, be a conductor."*

A Rust workspace that bundles four concerns into one CLI:

| Crate                   | Concern                                                                 |
|-------------------------|-------------------------------------------------------------------------|
| `conductr`              | Binary CLI that dispatches to the others.                               |
| `conductr-orchestrate`  | Port of [poorchestrator] — drive `@claude` GitHub-issue implementation in dependency order.  |
| `conductr-instance`     | Cloud instance spin-up & SSH (port of `agentic`; **stubbed**).          |
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
conductr orchestrate --repo owner/repo [--dry-run] [--once] [--poll-secs 60]
conductr instance    spin-up --name <name> | list
conductr schedule    validate <pattern.txt> | render <pattern.txt>
conductr tasks       list [--ready] | create <title> [-p N] | sync-to-notion --database <id>
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
│   ├── conductr-schedule/
│   └── conductr-tasks/
└── vendor/
    ├── poorchestrator/              # submodule
    └── beads_rust/                  # submodule
```
