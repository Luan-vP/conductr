# Idle

Idle is the peer of [orchestrate](../crates/conductr-orchestrate). Both are
top-level processes the binary drives on a cadence; orchestrate moves work
forward, idle is what runs **when there's nothing pressing to move**.

Both are **Claude-required**: the CLI bootstraps a tmux session, starts Claude
if needed, and hands off to the skill. The skill is the workflow.

Where orchestrate's doctrine spans [`cadence/`](./cadence/) and
[`operations/`](./operations/), idle is small enough to live in one doc.

## Distinction from `Health::Idle`

`Health::Idle` and `conductr pod free` are **session-level**: a tmux pane in the
local pod has no work in flight. The `idle` *process* is **orchestrator-level**:
the workflow as a whole has nothing pressing to do, so the binary picks up
self-directed scan work to keep the system improving between orchestrate
passes. Same English word, different scope.

## What a pass does

Each idle pass executes four phases in order. Failures in any phase are
reported but don't abort later phases — partial output is still useful.

### 1. Read configuration

```
[architecture]
style     = "hexagonal"
reference = ".claude/base.md"

[idle]
last_module = "..."   # written after each pass
last_run    = "..."   # ISO-8601 timestamp
```

`style` and `reference` drive the architecture scan in phase 2. `[idle]` is
state — idle reads and rewrites this block on every non-dry-run pass.

### 2. Architecture audit (delegated)

The architectural audit delegates entirely to the **architect** skill:

```
conductr architect review
```

This opens or reuses the `conductr-architect` pod session, starts Claude if
needed, and sends `/architect review` to run the full hexagonal audit
(including `check_cli_skill_parity`). Rules 1–4 are checked via
dependency-graph analysis; rules 5–6 via source-level inspection. Findings
emitted by the architect skill flow into phase 4 for issue filing. Idle does
not duplicate this analysis inline.

### 3. Module pick + scan

**Pick.** Round-robin through the use-case crates via `[idle].last_module`.
If the value is empty or unknown, start at the first crate.

**Deterministic scan.** Run `cargo clippy -p <crate> --message-format=json`
on the picked crate. Each clippy warning becomes a `Finding` with severity
`Quality`, fingerprinted as `clippy/{crate}/{lint}/{file:line}`.

### 4. File issues

For each `Finding` up to `--max-issues` (default 5):

- Skip if an open issue with an identical title already exists.
- Otherwise create via `ScmHost::create_issue` with labels
  `idle-finding` + (`architecture` | `quality`).
- Embed the fingerprint as an HTML comment in the body
  (`<!-- conductr-idle-fingerprint: ... -->`) for future fingerprint-based
  dedup.

Bodies include acceptance criteria so the next orchestrate pass can pick the
issue up and act on it without further triage.

After phase 4 succeeds, `[idle].last_module` advances to the next crate and
`last_run` is set to the current timestamp.

## Invocation

```
conductr idle [--repo-path <path>] [--dry-run]
```

| Flag | Default | Effect |
|------|---------|--------|
| `--dry-run` | off | Print what would happen (session state, which command would be sent); create no session, send no commands. |

`conductr idle` bootstraps the `conductr-idle` tmux session, starts Claude
if needed, waits until Claude is idle, and sends `/idle`. The skill takes
over from there, driving phases 1–4 above via tool calls to `conductr`
subcommands and `cargo`.

## Cadence

Configured under `[cadence] idle = "..."` in `.conductr`; `conductr cadence
sync` installs it. Default suggestion: `17 * * * *` (once an hour, offset
from the orchestrate slot at `*/30 * * * *` to avoid contention).

## Code location

The CLI bootstrap lives at `crates/conductr/src/main.rs` (`run_idle`). Pure
helpers — clippy-JSON parsing and architecture rule evaluators — sit at
`crates/conductr/src/idle.rs`. These are invocable from the skill via
tool-shaped CLI calls and are the authoritative implementations for their
respective checks.

The skill specification lives at `skills/idle/SKILL.md`.

Adding new architectural styles (e.g., layered, onion) means adding a new
rule evaluator behind the `style` switch — not a new crate.

## Skills

- `skills/idle/SKILL.md` — the skill specification that runs inside the
  `conductr-idle` Claude session.
- Related: [orchestrate](./operations/) (the peer process that consumes
  the issues idle creates).
- Related: [cadence/orchestration-cycle.md](./cadence/orchestration-cycle.md)
  (idle is scheduled alongside, but doesn't share the orchestrate loop).
