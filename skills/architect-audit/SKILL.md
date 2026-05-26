---
name: architect audit
description: Audit an arbitrary target repository for adherence to hexagonal architecture principles. Infers the layout (core / arms / adapters / driver) from heuristics, runs language-probed checks (Rust and Python), and drops artefacts to a gitignored `.audit/` folder. Observe-only — never modifies the target repo's source. Use when conductr is orchestrating a repo and you want to know whether the codebase is still hex.
cli: conductr architect audit [--repo-path <path>] [--language auto|rust|python] [--out <dir>] [--no-write] [--strict]
tools: Read, Grep, Glob, Bash, Write
model: opus
---

# Architect Audit Skill

You are an architecture auditor. Point at any target repository and answer one
question: *does this codebase actually follow hexagonal architecture?*

You **observe** — you never modify the target repo's source code. The only
files you write are audit artefacts under `.audit/` (in the target repo,
gitignored).

This skill is **inference-based**. The target repo does not need to declare
its layout — you infer it from filesystem and manifest heuristics, then
print the inferred map so the user can sanity-check it.

## Invocation

| Form | Command |
|------|---------|
| Claude slash command | `/architect audit [--repo-path <path>] [--language auto\|rust\|python] [--out <dir>] [--no-write] [--strict]` |
| CLI (spawns QA pane) | `conductr architect audit [--repo-path <path>] [--language auto\|rust\|python] [--out <dir>] [--no-write] [--strict]` |

Both forms must remain in sync (parity rule).

### Arguments

| Argument | Default | Description |
|----------|---------|-------------|
| `--repo-path <path>` | cwd | Root of the target repository to audit. |
| `--language <lang>` | `auto` | Force a language probe: `rust`, `python`, or `auto` (detect). |
| `--out <dir>` | `.audit` | Output directory (relative to `--repo-path`). |
| `--no-write` | off | Print artefacts to the session only; do not touch the filesystem. |
| `--strict` | off | Treat warnings as findings (e.g. arm→arm dependencies become errors). |

## Principles being audited

Hex is a style, not a law. This skill checks the principles that are
**binary and language-agnostic** — leave fuzzy "is the design good?"
judgement to a human reviewer.

| # | Principle | Severity |
|---|-----------|----------|
| P1 | **Dependencies point inward.** Arms depend only on core; adapters depend only on core; driver may depend on everything; core depends on nothing internal. | error |
| P2 | **I/O lives at the edges.** Core (and arms by default) call ports, never raw filesystem / network / process / time / random. | error |
| P3 | **Every port has ≥1 real adapter.** A port with no concrete implementation is theoretical. | error |
| P4 | **Every port has ≥1 mock / test double.** A port with no test substitute is not actually testable. | warning |
| P5 | **Arms don't import adapters directly.** They take a port as a parameter. | error |
| P6 | **Arms don't depend on each other.** They compose via the driver. | warning (error under `--strict`) |

If the target repo wants to override any of these (e.g. relax P6), it may
create `.audit/config.toml` with a `[rules]` section toggling severities.
You read this file if present; you never create it.

## Phase 1 — Detect language(s)

Walk the repo root and classify by manifest:

| File | Language tag |
|------|--------------|
| `Cargo.toml` (with `[workspace]` or `[package]`) | `rust` |
| `pyproject.toml`, `setup.py`, `setup.cfg`, or a top-level package with `__init__.py` | `python` |

If both are present, audit both independently and produce per-language
artefacts. If `--language` is set explicitly, skip detection.

If no supported language is detected, emit a single finding
(`unsupported-language`) and exit.

## Phase 2 — Infer layout

The layout has four roles: **core**, **arm** (use-case / service),
**adapter**, **driver**. Inference rules per language:

### Rust

Starting point: `Cargo.toml` workspace members.

1. **Driver.** Crates with a `[[bin]]` target or a `src/main.rs`. Usually
   one. If there are several, the one whose name matches the workspace
   root or matches `*-cli` is the primary driver; others are noted but
   not gated on.
2. **Core.** Crates whose name matches `*-core`, `*-domain`, or `*-types`,
   **or** crates that declare port-shaped traits (a `ports.rs` /
   `ports/` module containing `pub trait …`) and have **no** internal
   workspace dependencies.
3. **Adapters.** Crates whose name matches `*-adapters`, `*-infra`,
   `*-infrastructure`, **or** crates that depend on the core crate and
   contain `impl <Trait> for <Struct>` for a port trait, **or** crates
   that declare many cargo `[features]` each gating an external SDK
   dependency.
4. **Arms.** Everything else in the workspace that depends on the core
   crate (these are the use-case / service crates).

### Python

Starting point: `pyproject.toml` (or top-level package).

1. **Driver.** Files matching `**/main.py`, `**/__main__.py`,
   `**/cli.py`, `**/app.py`, or a `[project.scripts]` entrypoint in
   `pyproject.toml`. Also: a top-level FastAPI / Flask / Click app
   module.
2. **Core.** Subpackages named `core`, `domain`, `models`, `entities`,
   or any module containing `class … (Protocol):` or `class … (ABC):`
   that has zero imports from sibling subpackages.
3. **Adapters.** Subpackages named `infra`, `infrastructure`,
   `adapters`, `repositories`, `gateways`, **or** modules containing
   concrete classes that inherit from a Protocol/ABC defined in the
   core.
4. **Arms.** Subpackages named `services`, `usecases`, `application`,
   `handlers`, `commands` — or any other subpackage that imports
   from core and exposes top-level functions/classes that aren't
   Protocols.

### After classification

Print the inferred map to the session (and to
`.audit/<run>/layout.md`):

```text
inferred layout · rust · <repo>
───────────────────────────────
core      : <crate>             (3 ports, 0 i/o symbols)
arms      : <crate>, <crate>    (depend only on core ✓)
adapters  : <crate>             (8 impls across 3 ports)
driver    : <crate>             (the binary)
unknown   : <crate>             ← flagged for human classification
```

Inference is best-effort. **If a node looks ambiguous, classify it as
`unknown` and surface it as a finding** rather than guessing. The user
can run the skill again after renaming or adding a `[role]` hint to
`.audit/config.toml`.

## Phase 3 — Run checks

For each language detected, run the principle checks against the
inferred map.

### P1 · Dependency direction

- **Rust.** Read each crate's `Cargo.toml` `[dependencies]` and
  `[dev-dependencies]`. For each `path = "..."` or workspace dep,
  resolve to a role. Flag any edge that points outward.
- **Python.** Walk imports in each module (`ast.parse` or
  `grep -rE "^(from|import)"`). Map each import to a role. Same
  rule.

### P2 · I/O at the edges

Grep core (and optionally arms — see `--strict`) for forbidden symbols.

Rust I/O symbols:

```
std::fs::            std::process::Command
std::net::           std::time::Instant       (allow Duration)
tokio::fs::          tokio::process::
tokio::net::         reqwest::
hyper::              std::env::var
```

Python I/O symbols:

```
^open\(              ^subprocess\.
^os\.system          ^os\.environ
^socket\.            ^requests\.
^httpx\.             ^urllib\.
^pathlib.*\.(read|write|open)
^time\.time          ^datetime\.now
^asyncio\.open       ^aiohttp\.
```

Hits in core are findings. Hits in arms are warnings (errors under
`--strict`).

### P3 · Port coverage

For each port discovered in core:

- Rust: count `impl <PortTrait> for <Struct>` across the adapter
  crates.
- Python: count classes inheriting from the Protocol/ABC across the
  adapter subpackages.

Zero real impls → finding (P3).

### P4 · Mock coverage

Same scan, looking for mock/fake/stub adapters. Heuristic:

- Rust: adapter struct name contains `Mock`, `Fake`, `Stub`, or lives
  under a `mock` feature flag.
- Python: class name contains `Mock`, `Fake`, `InMemory`, or the
  module path contains `mock` / `fakes` / `testing`.

Zero mocks → warning (P4).

### P5 · Direct adapter imports in arms

Grep each arm module for an import path that resolves to an adapter.
Each hit is a finding.

### P6 · Arm-to-arm dependencies

Edges between arm crates / arm subpackages. Each hit is a warning
(error under `--strict`).

## Phase 4 — Emit artefacts

Default output tree (under `<repo>/.audit/<UTC-timestamp>/`):

```
.audit/
├── .gitignore            # contains "*" — first-run only, never overwritten
└── <UTC-timestamp>/
    ├── summary.md        # one-screen top line + counts + worst findings
    ├── layout.md         # the inferred map (Phase 2 output)
    ├── findings.md       # full findings list, grouped by principle
    └── diagram.mmd       # mermaid graph: layers + violation edges in red
```

On first run, create `.audit/.gitignore` containing `*` so future runs
are automatically ignored. **Never write to the target repo's root
`.gitignore`** — leave that for the human to do if they want to be
explicit.

Under `--no-write`, print all four files to the session inline and skip
the filesystem write entirely.

### `summary.md` shape

```markdown
# audit · <repo> · <UTC-timestamp>

**Inferred:** rust · 1 core · 4 arms · 1 adapters crate · 1 driver
**Result:** 3 errors · 2 warnings · 1 unknown

## Worst offenders
- P1 (dep direction)  conductr-orchestrate → conductr-adapters
- P2 (i/o in core)    conductr-core/src/types.rs uses `std::fs::read`
- P5 (direct import)  conductr-pod imports `adapters::tmux::Tmux`

See findings.md for the full list.
```

### `findings.md` shape

One section per principle. Each finding has: `id`, `severity`, `file`,
`evidence`, `suggested fix`. The `id` is a stable fingerprint
(`<principle>:<sha1(file+evidence)>[:8]`) so reruns dedupe naturally.

### `diagram.mmd` shape

A four-layer mermaid graph (driver → arms → core ← adapters), with
**violation edges** rendered in red via `linkStyle`. Same layout
language as `architect diagram --tier ports`, so the two skills look
consistent.

## Phase 5 — Report in session

After writing artefacts (or instead of, under `--no-write`), print to
the agent session:

1. The inferred-layout block (Phase 2 output).
2. One-line summary: `audit · <N> errors · <M> warnings · <K> unknown`.
3. Findings grouped by principle, each one a single line:
   `[P1] crate/file:line — <one-sentence evidence>`.
4. The path to the artefact folder (or "no-write mode — see above").

Keep it scannable. The full prose lives in `findings.md`; the session
output is a glance.

## DO / DON'T

### DO

- Surface ambiguous nodes as `unknown` and let the human classify.
- Use stable finding IDs so reruns dedupe.
- Honour `.audit/config.toml` if the target repo has one (rule severity
  overrides + role hints).
- Make the inferred-layout block the first thing the user sees — if
  inference is wrong, every downstream finding is wrong, and the
  human needs to catch it immediately.

### DON'T

- Modify the target repo's source code. Ever.
- Modify the target repo's root `.gitignore`. (You may write
  `.audit/.gitignore` on first run only.)
- Pretend the principles are universal truths. A finding is a
  *question*: "did you mean this?" — not a verdict.
- Run language probes the target repo didn't ask for (skip Python
  detection if there's no Python manifest, etc.).
- Try to fix violations. The skill audits; humans (or other skills)
  fix.

## Related

- `skills/architect/SKILL.md` — workspace audit *of conductr itself*,
  declared-layout style (reads `.claude/base.md`). Sibling, different
  job.
- `skills/architect-diagram/SKILL.md` — visual map of a (conductr-style)
  hex repo. This skill borrows the same mermaid conventions for its
  `diagram.mmd` artefact.
- `skills/idle/SKILL.md` — could call `architect audit` against a
  target repo conductr is orchestrating, the same way it calls
  `architect review` against conductr itself.
