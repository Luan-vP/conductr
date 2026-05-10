# Architecture Base — `conductr`

Hexagonal (ports & adapters). The CLI binary and the in-Claude skills
both call the same use-case crates; those use-case crates depend only
on a shared core (types + ports); concrete connectors (tmux, beads,
Notion, gh, …) live behind feature flags in a single adapters crate.

## Layout

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

## Ports

A new connector adds an *adapter*, not a new port, until the port can
no longer represent it. The current ports:

| Port               | Implementations (current)            | Implementations (planned)                |
|--------------------|--------------------------------------|------------------------------------------|
| `IssueTracker`     | `beads` (br CLI)                     | `notion`, `linear`, `github-issues`      |
| `ScmHost`          | `gh-cli`                             | `github-rest`, `gitlab`                  |
| `TmuxAgent`        | `tmux` (local subprocess)            | `ssh-tmux` (remote)                      |
| `InstanceProvider` | `mock` (stub)                        | `aws`, `hetzner`, `digitalocean`, `local`|
| `Mailbox`          | `mail-fs`, `mail-github`             | `mail-slack`                             |

Mocks live alongside real adapters (feature `mock`) and are consumed by
use-case unit tests.

## Rules

1. **Use-case crates may not depend on `conductr-adapters` or any
   specific connector crate.** They depend on `conductr-core` only.
2. **The binary is the only place adapters are constructed and wired
   into use cases.** Selection is via flags / env / config.
3. **Adapters never depend on use-case crates.** They speak the port
   traits in `conductr-core::ports`.
4. **`conductr-core` has no I/O.** No `tokio::process`, `reqwest`,
   filesystem reads beyond `serde_json` on `&str`.
5. **Mocks belong in `conductr-adapters` behind the `mock` feature**,
   not in per-crate `tests/` modules.
6. **One trait per port.** Adding a new connector adds an adapter, not
   a new port.

## Driving adapters

The binary (`crates/conductr`) is one driving adapter. Each
`skills/<name>/SKILL.md` is another — but the skill *markdown* is not
a workspace member; it shells out to the binary. So adding a skill is
a documentation change, not a code change. New skills compose existing
CLI subcommands; they should not introduce new flows that the CLI does
not already expose.

## When to update this doc

- Adding or removing a port (rare).
- Adding or removing a use-case crate (medium).
- Adding an adapter feature flag (every time).
- Changing one of the six rules above (very rare; flag in PR).

A PR that changes the structural shape of the system (new arm, new
fold, a connection direction reversed) should also update this doc in
the same PR. PRs that only refactor within a single arm don't need to
touch it.
