---
name: idle
description: Self-directed scan agent. Reads architecture config, delegates the full architectural audit to /architect review, runs a security audit via /architect security-review, runs cargo clippy on the next round-robin crate (Rust repos only), and files findings as GitHub issues with stable fingerprints. Claude-required — the CLI bootstraps the tmux session and sends this skill.
cli: conductr idle [--repo-path <path>] [--dry-run]
tools: Read, Bash, WebFetch
model: opus
---

# Idle Scan

Run a self-directed maintenance pass: architecture audit + security audit + module clippy scan
→ file findings as GitHub issues.

## Invocation

| Form | Command |
|------|---------|
| Claude slash command | `/idle` |
| CLI (spawns this session) | `conductr idle` |

The CLI form opens or reuses the `conductr-idle` tmux session, starts Claude
if needed, and sends `/idle`. Both forms must stay in sync (parity rule).

## Workflow

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
needed, and runs `/architect review`. That skill:
- Loads the architectural pattern (Pattern + Rules + Arms) from `.claude/base.md`.
- When `.claude/base.md` is absent or has no structured pattern sections, emits
  a single finding proposing the hexagonal default template — no other checks run.
- When the base file is present, checks all declared rules against the codebase.
- Runs `check_cli_skill_parity` only when the repo has both a CLI and a `skills/`
  surface (auto-detected); it is a no-op on repos that have neither.

Findings flow back to phase 5. Do not duplicate any of the architect's checks
here — the delegation is intentional. Wait for the architect session to return
findings before proceeding.

### Phase 2.5 — Security audit (delegated)

Delegate to the architect skill's security-review mode:

```bash
conductr architect security-review
```

This reuses the `conductr-architect` session and runs `/architect security-review`.
That skill performs a pure source-level scan for:
- Hardcoded secrets in committed files
- Vulnerable dependencies (`npm audit`, `cargo audit`, `pip-audit`, etc. — auto-detected from lockfiles)
- Auth/AuthZ surface gaps
- Input validation gaps
- Logging hygiene (sensitive data at info+ level)
- Common framework footguns (framework-aware; LLM judges relevance and severity)

Each finding has severity `Security`, fingerprinted as `security/<category>/<file:line>`.
Findings flow into phase 5 alongside architecture findings.

If the `conductr-architect` session is busy after phase 2, skip phase 2.5 and
proceed with phases 3–4.

### Phase 3 — Module pick + clippy scan

**Rust-only phase.** Check for `Cargo.toml` at the repo root:

```bash
test -f Cargo.toml || { echo "idle: no Cargo.toml — skipping clippy phase"; exit 0; }
```

If `Cargo.toml` is absent, log one line and skip to phase 4. Do not error.

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

### Phase 4 — Module coverage scan

**Rust-only phase.** Check for `Cargo.toml` at the repo root:

```bash
test -f Cargo.toml || { echo "idle: no Cargo.toml — skipping coverage phase"; exit 0; }
```

If `Cargo.toml` is absent, log one line and skip to phase 5. Do not error.

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

Collect all findings from phases 2, 2.5, 3, and 4. Ensure the required labels exist:

```bash
gh label create idle-finding  --color d4c5f9 --description "Auto-filed by conductr idle"      --force
gh label create architecture  --color e4e669 --description "Architecture rule violation"       --force
gh label create quality       --color bfd4f2 --description "Code quality finding"              --force
gh label create coverage      --color 0075ca --description "Test coverage finding"             --force
gh label create security      --color e11d48 --description "Security finding"                  --force
```

For each finding (up to `--max-issues`, default 5):

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

### Phase 6 — Persist state

After phase 5 succeeds on a non-dry-run pass:

```bash
# Update [idle] block in .conductr
last_module = "<crate scanned in phase 3, or unchanged if skipped>"
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
  phase 2 and proceed with phases 2.5–4 using only clippy/coverage findings.
- If `conductr architect security-review` is busy after phase 2, skip phase 2.5.
- Phases are independent: a clippy failure does not block issue filing of
  architecture findings, and vice versa.
- Phases 3 and 4 (clippy and coverage) are Rust-only. They skip gracefully when
  `Cargo.toml` is absent — no error, one logged line.
