# Idle

Idle is the peer of [orchestrate](../crates/conductr-orchestrate). Both are
top-level processes the binary drives on a cadence; orchestrate moves work
forward, idle is what runs **when there's nothing pressing to move**.

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

### 2. Architecture scan (deterministic)

For the declared `style`, run the corresponding rule checks against the
workspace. For `hexagonal`, that's the six rules in `.claude/base.md`; v1
mechanically checks three of them:

| Rule | Check | Source |
|------|-------|--------|
| 1 | Use-case crates must not depend on `conductr-adapters` or any connector crate. | Walk each crate's `[dependencies]` table. |
| 3 | Adapters must not depend on use-case crates. | Same. |
| 4 | `conductr-core` has no I/O dependencies (`tokio`, `reqwest`, `hyper`, etc.). | Denylist check against core's `[dependencies]`. |

Rules 2, 5, and 6 require source-level analysis (`use` paths, mock locations,
port-trait counts) and are deferred. Each violation becomes a `Finding` with
severity `Architecture`.

### 3. Module pick + scan

**Pick.** Round-robin through the use-case crates via `[idle].last_module`.
If the value is empty or unknown, start at the first crate.

**Deterministic scan.** Run `cargo clippy -p <crate> --message-format=json`
on the picked crate. Each clippy warning becomes a `Finding` with severity
`Quality`, fingerprinted as `clippy/{crate}/{lint}/{file:line}`.

**LLM scan.** If a `LocalAgent` is available (default: Ollama; bypass with
`--no-llm`), send the crate's source (capped at ~32 KB) with a prompt asking
for up to 5 refactor / efficiency / quality suggestions. Each suggestion
becomes a `Finding` with severity `Suggestion`, fingerprinted as
`llm/{crate}/{slug(title)}`.

### 4. File issues

For each `Finding` up to `--max-issues` (default 5):

- Skip if an open issue with an identical title already exists.
- Otherwise create via `ScmHost::create_issue` with labels
  `idle-finding` + (`architecture` | `quality` | `refactor`).
- Embed the fingerprint as an HTML comment in the body
  (`<!-- conductr-idle-fingerprint: ... -->`) for future fingerprint-based
  dedup.

Bodies include acceptance criteria so the next orchestrate pass can pick the
issue up and act on it without further triage.

After phase 4 succeeds, `[idle].last_module` advances to the next crate and
`last_run` is set to the current timestamp.

## Invocation

```
conductr idle [--repo-path <path>] [--dry-run] [--max-issues N] [--no-llm]
```

| Flag | Default | Effect |
|------|---------|--------|
| `--dry-run` | off | Print rendered findings; create no issues; don't advance `[idle]`. |
| `--max-issues` | 5 | Cap created issues per pass. |
| `--no-llm` | off | Skip phase-3 LLM scan; deterministic checks only. |

## Cadence

Configured under `[cadence] idle = "..."` in `.conductr`; `conductr cadence
sync` installs it. Default suggestion: `17 * * * *` (once an hour, offset
from the orchestrate slot at `*/30 * * * *` to avoid contention).

## Code location

By design, idle does not get its own crate. It's a thin composition of
existing primitives:

- `ScmHost::create_issue` (port) → `gh-cli` adapter.
- `LocalAgent::complete` (port) → `ollama` / `llamacpp` adapters.
- `cargo` shellouts for clippy.
- Filesystem reads for `.conductr` and each crate's `Cargo.toml`.

The implementation lives at `crates/conductr/src/idle.rs` (driving-adapter
layer in the binary). Pure helpers — clippy-JSON parsing, LLM-response
parsing, round-robin picker, architecture rule evaluator — sit alongside as
private functions with their own unit tests.

Adding new architectural styles (e.g., layered, onion) means adding a new
rule evaluator behind the `style` switch — not a new crate.

## Skills

- This doc is the canonical spec; the `idle` command implements it.
- Related: [orchestrate](./operations/) (the peer process that consumes
  the issues idle creates).
- Related: [cadence/orchestration-cycle.md](./cadence/orchestration-cycle.md)
  (idle is scheduled alongside, but doesn't share the orchestrate loop).
