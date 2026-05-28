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

Each idle pass executes six phases in order. Failures in any phase are
reported but don't abort later phases — partial output is still useful.

Phases 3 and 4 are **Rust-only**: they are skipped with a single logged line
on repos that have no `Cargo.toml` at the root. This makes idle useful on
any repo that has a `.claude/base.md`, regardless of ecosystem.

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
needed, and sends `/architect review`. That skill reads `.claude/base.md`
for the architectural pattern and rules (defaulting to hexagonal if the file
is absent or unstructured), then audits the workspace against those rules.
It also runs `check_cli_skill_parity` when a `skills/` surface is present.
Findings emitted by the architect skill flow into phase 5 for issue filing.
Idle does not duplicate this analysis inline.

### 2.5. Security audit (delegated)

The source-level security review delegates to the **architect** skill:

```
conductr architect security-review
```

This opens or reuses the `conductr-architect` pod session (the same one used
by phase 2) and sends `/architect security-review`. The skill performs a
pure static analysis pass covering:

- **Hardcoded secrets** — committed API keys, tokens, passwords in source files.
- **Dependency hygiene** — `npm audit`, `cargo audit`, `pip-audit` or equivalent
  for whatever package managers the repo uses (auto-detected from lockfile presence).
- **Auth/AuthZ surface** — missing auth on routes, missing CSRF, unverified webhooks,
  missing rate limits on auth endpoints.
- **Input validation gaps** — user-controlled surfaces without evident validation.
- **Logging hygiene** — sensitive data logged at info+ level.
- **Framework footguns** — `dangerouslySetInnerHTML`, `eval`, `unsafe` without SAFETY
  comments, `subprocess.shell=True`, etc. (framework-aware; Claude judges relevance).

Findings use `FindingSeverity::Security` and the `security` label. The LLM is
expected to judge severity — not every `unsafe` block in a safe-systems-programming
codebase is a real finding.

If the architect session is still busy from phase 2 when phase 2.5 runs,
skip with a log line and continue to phase 3.

### 3. Module pick + clippy scan (Rust-only)

**Probe.** If `Cargo.toml` is absent at the repo root, log:
```
idle: no Cargo.toml at repo root — skipping clippy scan (non-Rust repo)
```
and skip to phase 4.

**Pick.** Round-robin through the use-case crates via `[idle].last_module`.
If the value is empty or unknown, start at the first crate.

**Deterministic scan.** Run `cargo clippy -p <crate> --message-format=json`
on the picked crate. Each clippy warning becomes a `Finding` with severity
`Quality`, fingerprinted as `clippy/{crate}/{lint}/{file:line}`.

### 4. Module coverage scan (Rust-only)

**Probe.** If `Cargo.toml` is absent at the repo root, log:
```
idle: no Cargo.toml at repo root — skipping coverage scan (non-Rust repo)
```
and skip to phase 5.

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

After phase 5 succeeds, `[idle].last_module` advances to the next crate and
`last_run` is set to the current timestamp.

### Ecosystem matrix

| Phase | conductr (Rust) | JS/TS repo | Python repo | No base.md |
|-------|----------------|------------|-------------|------------|
| 2. arch audit | ✓ | ✓ | ✓ | ✓ (hex default) |
| 2.5 security | ✓ | ✓ | ✓ | ✓ |
| 3. clippy | ✓ | skip | skip | ✓ |
| 4. coverage | ✓ | skip | skip | ✓ |
| 5. file issues | ✓ | ✓ | ✓ | ✓ |

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
