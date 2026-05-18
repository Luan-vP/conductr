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

Each idle pass executes phases in order. Failures in any phase are reported
but don't abort later phases — partial output is still useful.

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
needed, and sends `/architect review` to run the full audit.

The audit is **language- and pattern-agnostic**:
- The skill reads `.claude/base.md` and extracts the declared Pattern, Rules,
  and Arms sections at runtime. It does not hardcode any rule set.
- When `.claude/base.md` is absent or contains no structured pattern sections,
  the audit emits **one finding** proposing the hexagonal (ports & adapters)
  template as the default starting point — no other checks run.
- `check_cli_skill_parity` runs only when the repo has both a CLI surface and a
  `skills/` directory. On repos that have neither, it is a no-op.

Findings emitted by the architect skill flow into phase 5 for issue filing.
Idle does not duplicate this analysis inline.

### 2.5. Security audit (delegated)

A source-level security scan, delegated to the **architect** skill:

```
conductr architect security-review
```

This reuses the `conductr-architect` session and sends `/architect security-review`.
The skill performs a pure static review (no penetration testing, fuzzing, or
dynamic analysis). It scans for:

- **Hardcoded secrets** — API keys, tokens, passwords, private keys in committed files.
- **Dependency hygiene** — runs `npm audit`, `cargo audit`, `pip-audit`, or equivalent,
  auto-detected from lockfile presence. Ecosystems not present in the repo are skipped.
- **Auth/AuthZ surface** — middleware, route guards, session handling gaps.
- **Input validation gaps** — user-facing surfaces without evident validation layers.
- **Logging hygiene** — sensitive data at info+ level.
- **Framework footguns** — framework-aware; LLM judges relevance and severity.

Findings carry severity `Security` and are fingerprinted as
`security/<category>/<file:line>`. They flow into phase 5 alongside architecture
findings. The `security` GitHub label is created by phase 5 if absent.

If `conductr-architect` is busy after phase 2, phase 2.5 is skipped.

### 3. Module pick + clippy scan (Rust-only)

**Probe:** if `Cargo.toml` is absent at the repo root, log one line and skip.
This makes phase 3 a no-op on non-Rust repos rather than an error.

**Pick.** Round-robin through the use-case crates via `[idle].last_module`.
If the value is empty or unknown, start at the first crate.

**Deterministic scan.** Run `cargo clippy -p <crate> --message-format=json`
on the picked crate. Each clippy warning becomes a `Finding` with severity
`Quality`, fingerprinted as `clippy/{crate}/{lint}/{file:line}`.

### 4. Module coverage scan (Rust-only)

**Probe:** if `Cargo.toml` is absent at the repo root, log one line and skip.
Same probe as phase 3 — both are Rust-only.

For the same crate picked in phase 3, run:

```
cargo llvm-cov --json -p <crate>
```

If `cargo llvm-cov` is not installed, log a warning and skip; the rest of
the pass continues unaffected.

**Threshold.** Any source file in `<crate>/src/` whose line coverage is below
`[idle] coverage_threshold` (default `0.6` = 60 %) is flagged as a `Finding`
with severity `Coverage`, fingerprinted as `coverage/{crate}/{rel-path}`.

**Exclusions.** Files matched by `[idle] coverage_exclude` glob patterns are
skipped. Binary entry points and generated code are the common use case:

```toml
[idle]
coverage_threshold = 0.6
coverage_exclude   = ["src/main.rs", "src/bin/**"]
```

**Coverage is a separate phase from clippy** because `cargo llvm-cov`
instruments and runs the full test suite — significantly slower than a
compile-only clippy pass. Keeping them separate means clippy findings are
still emitted even when `llvm-cov` is absent or slow.

### 5. File issues

For each `Finding` up to `--max-issues` (default 5):

- Skip if an open issue with an identical title already exists.
- Otherwise create via `ScmHost::create_issue` with labels
  `idle-finding` + (`architecture` | `quality` | `coverage` | `security`).
- Embed the fingerprint as an HTML comment in the body
  (`<!-- conductr-idle-fingerprint: ... -->`) for future fingerprint-based
  dedup.

Bodies include acceptance criteria so the next orchestrate pass can pick the
issue up and act on it without further triage.

After phase 5 succeeds, `[idle].last_module` advances to the next crate (or
stays unchanged if phases 3–4 were skipped) and `last_run` is set to the
current timestamp.

## Language- and ecosystem-agnostic design

Phases 2 and 2.5 (architecture and security audits) work on any repo that has
a `.claude/base.md`. The LLM-driven analysis adapts to whatever pattern is
declared — hexagonal, layered SPA, mobile monolith, or anything else.

Phases 3 and 4 (clippy and coverage) are Rust-only and skip gracefully on
non-Rust repos. Future tickets can extend these phases to other ecosystems:
- Module scan in JS/TS repos: `eslint --format json`
- Coverage in JS/TS repos: `vitest --coverage` or `jest --coverage`

Adding support for a new ecosystem means adding a probe for its manifest file
and a scanner for its tooling — not a new phase.

## Invocation

```
conductr idle [--repo-path <path>] [--dry-run]
```

| Flag | Default | Effect |
|------|---------|--------|
| `--dry-run` | off | Print what would happen (session state, which command would be sent); create no session, send no commands. |

`conductr idle` bootstraps the `conductr-idle` tmux session, starts Claude
if needed, waits until Claude is idle, and sends `/idle`. The skill takes
over from there, driving phases 1–5 above via tool calls to `conductr`
subcommands and `cargo`.

## Cadence

Configured under `[cadence] idle = "..."` in `.conductr`; `conductr cadence
sync` installs it. Default suggestion: `17 * * * *` (once an hour, offset
from the orchestrate slot at `*/30 * * * *` to avoid contention).

## Code location

The CLI bootstrap lives at `crates/conductr/src/main.rs` (`run_idle`). Pure
helpers — clippy-JSON parsing, coverage parsing, architecture rule evaluators,
and the `check_base_md` probe — sit at `crates/conductr/src/idle.rs`. These
are invocable from the skill via tool-shaped CLI calls and are the authoritative
implementations for their respective checks.

The skill specification lives at `skills/idle/SKILL.md`.

Adding new architectural styles (e.g., layered, onion) means declaring a new
Pattern + Rules in `.claude/base.md` — not adding a new crate. The architect
skill reads the base file at runtime.

## Skills

- `skills/idle/SKILL.md` — the skill specification that runs inside the
  `conductr-idle` Claude session.
- `skills/architect/SKILL.md` — architecture and security audit skills invoked
  by idle's phases 2 and 2.5.
- Related: [orchestrate](./operations/) (the peer process that consumes
  the issues idle creates).
- Related: [cadence/orchestration-cycle.md](./cadence/orchestration-cycle.md)
  (idle is scheduled alongside, but doesn't share the orchestrate loop).
