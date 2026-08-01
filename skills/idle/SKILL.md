---
name: idle
description: Self-directed scan agent. Reads architecture config, delegates the full architectural audit to /architect review, runs cargo clippy on the next round-robin crate, and files findings as GitHub issues with stable fingerprints. Claude-required — the CLI bootstraps the tmux session and sends this skill.
cli: conductr idle [--repo-path <path>] [--dry-run]
tools: Read, Bash, WebFetch
model: opus
---

# Idle Scan

Run a self-directed maintenance pass: architecture audit + module clippy scan
→ file findings as GitHub issues.

## Invocation

| Form | Command |
|------|---------|
| Claude slash command | `/idle` |
| CLI (spawns this session) | `conductr idle` |

The CLI form opens or reuses the `conductr-idle` tmux session, starts Claude
if needed, and sends `/idle`. Both forms must stay in sync (parity rule).

## Workflow

### Phase 0 — Self-update (deterministic)

Keep the installed `conductr` binary current before scanning:

```bash
conductr self-update
```

This resolves the conductr checkout from the registry, fast-forwards the branch
it is installed from (`develop`), and re-runs `cargo install` — but only if it
has not already done so in the last 24h (rate-limited via a state file), so
running it every idle pass rebuilds at most daily. It is a no-op on `--dry-run`
beyond printing the plan, and never blocks the scan: if the fetch/pull/install
fails it logs one line and phase 1 continues. Pass `--force` to update
immediately regardless of the interval.

### Phase 1 — Read configuration

Read `.conductr` from the repo root:

```toml
[architecture]
style     = "hexagonal"
reference = ".claude/base.md"

[idle]
last_module = "..."   # last crate scanned; empty on first run
last_run    = "..."   # ISO-8601 timestamp of last completed pass
```

`style` and `reference` drive phase 2. `[idle]` is state that is updated
after a successful non-dry-run pass.

### Phase 2 — Architecture audit (delegated)

Delegate the full architectural audit to the architect skill:

```bash
conductr architect review
```

This opens or reuses the `conductr-architect` session, starts Claude if
needed, and runs `/architect review`. That skill reads `.claude/base.md`
for the pattern and rules (defaulting to hexagonal if absent), checks
CLI–skill parity, and emits `Architecture` findings. Findings flow back
to phase 5.

Do not duplicate any of the architect's checks here — the delegation is
intentional. Wait for the architect session to return findings before
proceeding.

### Phase 2.5 — Security audit (delegated)

Delegate a source-level security review to the architect skill:

```bash
conductr architect security-review
```

This opens or reuses the `conductr-architect` session, starts Claude if
needed, and runs `/architect security-review`. That skill scans for
hardcoded secrets, dependency advisories, auth/AuthZ gaps, input
validation holes, logging hygiene issues, and framework footguns.
Findings use `FindingSeverity::Security` and flow into phase 5 with the
`security` label.

Do not duplicate the security scan here — the delegation is intentional.
If the architect session is already busy from phase 2, skip this phase
and log a note; do not block phase 3.

### Phase 3 — Module pick + clippy scan (Rust-only)

**Probe:** Check for `Cargo.toml` at the repo root:

```bash
test -f Cargo.toml && echo "rust" || echo "skip"
```

If `Cargo.toml` is absent, log one line:
```
idle: no Cargo.toml at repo root — skipping clippy scan (non-Rust repo)
```
and skip to phase 4.

**Pick the next crate.** Read `[idle].last_module` and advance one step in
the round-robin list:

```
conductr-orchestrate → conductr-pod → conductr-tasks → conductr-instance
→ conductr-schedule → conductr-mail → conductr-setup → (wrap to start)
```

If `last_module` is empty or unknown, start at `conductr-orchestrate`.

**Run clippy** on the picked crate:

```bash
cargo clippy -p <crate> --message-format=json 2>/dev/null
```

Parse the JSON output. For each `compiler-message` at level `warning`,
create a `Finding`:

- **Title**: `quality: \`<crate>\` clippy warning in <file>:<line>`
- **Severity**: `Quality`
- **Fingerprint**: `clippy/<crate>/<lint>/<file>:<line>`
- **Body**: rendered clippy output + acceptance criteria

Deduplicate by fingerprint within this run.

### Phase 4 — Module coverage scan (Rust-only)

**Probe:** Check for `Cargo.toml` at the repo root:

```bash
test -f Cargo.toml && echo "rust" || echo "skip"
```

If `Cargo.toml` is absent, log one line:
```
idle: no Cargo.toml at repo root — skipping coverage scan (non-Rust repo)
```
and skip to phase 5.

For the same crate picked in phase 3, run line-coverage analysis:

```bash
cargo llvm-cov --json -p <crate>
```

If `cargo llvm-cov` is not installed, log a warning and skip this phase — the
rest of the pass continues unaffected.

Parse the JSON output. For each file under `<crate>/src/`:

- Skip files matched by `[idle] coverage_exclude` globs (default: empty).
- Flag any file whose line coverage is below `[idle] coverage_threshold`
  (default: `0.6` = 60%).
- Create a `Finding` with:
  - **Title**: `coverage: \`<crate>/<rel-path>\`: <pct>% below threshold (<n> uncovered lines)`
  - **Severity**: `Coverage`
  - **Fingerprint**: `coverage/<crate>/<rel-path>` — stable across runs for dedup
  - **Body**: top 5 uncovered line ranges, current coverage %, threshold, and
    acceptance criteria ("add tests so coverage ≥ \<threshold\>%")

Example `.conductr` configuration:
```toml
[idle]
coverage_threshold = 0.6
coverage_exclude   = ["src/main.rs", "src/bin/**"]
```

### Phase 5 — File issues

Collect all findings from phases 2, 2.5, 3, and 4. For each finding (up to
`--max-issues`, default 5):

1. Check for an open issue with an identical title:
   ```bash
   gh issue list --state open --json title | jq -r '.[].title'
   ```
2. If a duplicate exists, skip.
3. Otherwise create the issue:
   ```bash
   gh issue create \
     --title "<title>" \
     --body "<body with fingerprint comment>" \
     --label "idle-finding,<severity-label>"
   ```
   Labels: `idle-finding` + one of `architecture`, `quality`, `coverage`, or `security`.
4. Embed the fingerprint as an HTML comment in the body:
   `<!-- conductr-idle-fingerprint: <fingerprint> -->`

Ensure the labels exist first:
```bash
gh label create idle-finding --color d4c5f9 --description "Auto-filed by conductr idle" --force
gh label create architecture  --color e4e669 --description "Architecture rule violation"  --force
gh label create quality       --color bfd4f2 --description "Code quality finding"          --force
gh label create coverage      --color 0075ca --description "Test coverage finding"         --force
gh label create security      --color ee0701 --description "Security finding"               --force
```

### Phase 6 — Persist state

After phase 5 succeeds on a non-dry-run pass (phases 3 and 4 may have been
skipped for non-Rust repos; that is not an error):

```bash
# Update [idle] block in .conductr
last_module = "<crate scanned in phase 3>"
last_run    = "<ISO-8601 timestamp>"
```

Print a summary:
```
idle: <N> finding(s) total, <M> filed, module=<crate>
```

## Flags

| Flag | Default | Effect |
|------|---------|--------|
| `--dry-run` | off | Print rendered findings; create no issues; don't advance `[idle]`. |
| `--max-issues` | 5 | Cap created issues per pass. |

## Important

- Never implement fixes yourself — idle files issues; orchestrate and the
  GitHub Action bot implement them.
- The `--dry-run` flag is passed through from the CLI; honour it in every
  phase that writes state or creates issues.
- If `conductr architect review` is already running (session busy), skip
  phase 2 and proceed with phases 3–4 using only clippy findings.
- Phases are independent: a clippy failure does not block issue filing of
  architecture findings, and vice versa.
