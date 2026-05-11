# ADR: CLI–Skill Parity

**Status:** Adopted (original decision in #76; this document is the ADR record)

---

## Context

`conductr` exposes its functionality through two surfaces:

- **CLI** — `conductr <subcommand> [flags]`, implemented in `crates/conductr`
  with `clap` argument parsing.
- **Skills** — `skills/<name>/SKILL.md`, markdown files that Claude reads to
  know how to drive the same flows from inside a conversation.

Both surfaces must stay in sync: if the CLI gains a flag or a new subcommand
and the skill isn't updated, Claude will give wrong guidance. If the skill
describes flags that don't exist in clap, the user gets confusing errors.

The original ticket (#76) identified nine failure modes and resolved them. This
document captures the rule, its two flavors, the enforcement mechanism, and
the disposition of each problem.

---

## The Rule

> Every skill `skills/<name>/SKILL.md` whose effect is "run a conductr CLI
> command" must reference that command verbatim in the form a user would type
> it. The skill's frontmatter `name` matches the top-level CLI subcommand.
> Flags exposed by the skill are a subset of (or equal to) the flags clap
> exposes for that subcommand, and each flag has the same name, type, and
> meaning in both adapters. The shared invocation string is the contract
> between adapters; the surrounding prose is not.

**Parity is universal.** Every top-level CLI subcommand has a corresponding
`skills/<name>/SKILL.md`, and every skill that wraps a CLI command maps to
exactly one top-level subcommand. The `cli_parity: true` opt-in originally
proposed in #76 is obsolete — symmetry is unconditional for top-level commands.

---

## Two Flavors

### Claude-required (CLI wraps skill)

The CLI command's job is to get Claude into the right context, then hand off.
The binary spawns or reuses a tmux session, starts Claude if needed, and
invokes the skill. **The skill is the workflow.**

```
User types:  conductr begin
             └─► tmux + claude --continue
                               └─► /conductr-pod (skill takes over)
```

Examples: `begin`, any subcommand whose real work is multi-step inference,
context gathering, or decision-making that would be awkward to encode as
a pure Rust algorithm.

The skill's invocation string is the source of truth for what `conductr`
should ask Claude to do. When the CLI delegates, it passes the canonical
invocation string from the skill without paraphrase.

### Function-only (skill wraps CLI)

The CLI runs pure Rust, with optional single-shot `LocalAgent` calls for
tasks that are genuinely better done with inference (e.g. summarising a pane
buffer). The skill is a **thin shell-out wrapper**: it calls the binary and
formats the result for the user.

```
User types:  /conductr-pod  (or Claude invokes the skill)
             └─► conductr diagnose --json
             └─► conductr heal --dry-run
             └─► (skill interprets output, updates Notion, etc.)
```

Examples: `diagnose`, `heal`, `save-state`, `schedule`, `tasks`. The Rust
implementation is authoritative; the skill is the human-facing adapter that
calls it and does any post-processing Claude can add value to.

---

## Enforcement

One predicate, two consumers.

### Predicate

```rust
fn check_cli_skill_parity(workspace: &Workspace) -> Vec<Finding>
```

Located in `crates/conductr-orchestrate` (the workspace-analysis path). For
each top-level CLI subcommand:

1. Check that `skills/<subcommand>/SKILL.md` exists.
2. Check that the skill's frontmatter `name` matches the subcommand name.
3. Parse flags documented in the skill. For each flag, verify it appears in
   the clap definition for that subcommand with the same name, value type,
   and description.

Each violation is a `Finding` with `Severity::Architecture` (no new severity
variant). The finding body includes the subcommand name, the specific
violation, and a suggested fix.

### Consumer 1 — CI test

```rust
#[test]
fn cli_skill_parity_is_clean() {
    let ws = Workspace::open(".").unwrap();
    let findings = check_cli_skill_parity(&ws);
    assert!(findings.is_empty(), "{findings:#?}");
}
```

Lives in the workspace's test suite. Fails the build on any drift. This is
the primary enforcement gate — it runs on every push.

### Consumer 2 — `conductr architect` (workspace mode)

`conductr architect` calls the same predicate during its architecture scan
phase. Findings flow into the idle pass (see [`operations/idle.md`](../operations/idle.md))
and are filed as GitHub issues via `idle`'s issue-filing path. Severity maps to
`Architecture` findings in the issue label set.

This is a secondary enforcement gate. It catches drift on branches that
haven't been pushed to CI, and it creates actionable issues rather than just
failing.

---

## Current Skill–Subcommand Map

As of adoption, the mapping is:

| CLI subcommand  | Skill file                             | Flavor            |
|-----------------|----------------------------------------|-------------------|
| `begin`         | `skills/begin/SKILL.md` (to be added) | Claude-required   |
| `orchestrate`   | `skills/orchestrate/SKILL.md`          | Claude-required   |
| `diagnose`      | `skills/diagnose/SKILL.md` (to be added) | Function-only   |
| `free`          | `skills/free/SKILL.md` (to be added)  | Function-only     |
| `heal`          | `skills/heal/SKILL.md` (to be added)  | Function-only     |
| `save-state`    | `skills/save-state/SKILL.md` (to be added) | Function-only |
| `tasks`         | `skills/tasks/SKILL.md` (to be added) | Function-only     |
| `setup`         | `skills/setup/SKILL.md` (to be added) | Function-only     |
| `mail`          | `skills/mail/SKILL.md` (to be added)  | Function-only     |
| `local`         | `skills/local/SKILL.md` (to be added) | Function-only     |
| `cadence`       | `skills/cadence/SKILL.md` (to be added) | Function-only   |
| `schedule`      | `skills/schedule/SKILL.md` (to be added) | Function-only  |
| `run-task`      | `skills/run-task/SKILL.md` (to be added) | Function-only  |
| `instance`      | `skills/instance/SKILL.md` (to be added) | Claude-required (when unblocked) |

**Migration note:** `skills/conductr-pod/SKILL.md` currently covers
`diagnose`, `heal`, and `save-state` as a single omnibus skill. Under this
ADR, each subcommand gets its own skill file and the `conductr-pod` skill is
retired or narrowed to a meta-skill (a Claude-required "manage the pod"
workflow that calls the individual subcommand skills). The parity predicate
fires on the individual subcommand names, not on `conductr-pod`. The migration
is tracked separately from this ADR.

---

## Resolution of the Nine Problems

The original ticket (#76) listed nine failure modes. Their disposition:

**Problems 1, 2, 3, 5, 9 — accepted as resolved in #76.**

- Parity is on invocation syntax only; surrounding skill prose is not part of
  the contract.
- Skills may prompt the user before calling the CLI; that pre-call dialogue
  is the skill's business.
- Natural-language translation from user intent to CLI invocation is the
  skill's job; parity holds after translation, not before.
- Parity is one-way: the skill claims conformance to the CLI surface. The CLI
  does not reference the skill.
- Skills can do orchestrating glue (e.g. call `save-state`, parse output,
  then update Notion) around the canonical invocation. The contract is the
  invocation string, not the full workflow.

**Problem 4 (multi-step skills) — dissolves under universal parity.**

Even a skill that does multiple things (save-state → Notion update → report)
is invoked via a single top-level CLI subcommand. What the skill does
internally after the canonical invocation is up to it. The predicate checks
the invocation string and the flags; it does not constrain post-invocation
behaviour.

**Problem 6 (version skew) — deferred to v2.**

If a skill documents a flag that existed in an older CLI version but was
removed, the predicate catches it. If a skill documents a flag that doesn't
exist yet in the user's installed binary, the predicate passes (the skill
describes the current repo, not the installed binary). A `min_conductr_version:`
frontmatter field is the right fix; it will be added as a follow-up if version
skew causes real problems in practice.

**Problem 7 (MCP as a third adapter) — documents the Option B trigger.**

If MCP exposure enters the roadmap, the project moves from the
convention-test enforcement model (this ADR) to Option B: a single-source
registry where the invocation string is declared once and emitted to clap,
the skill, and the MCP tool descriptor from the same source. MCP is the
trigger for Option B, not drift count.

**Problem 8 (flag-description parity) — included in the predicate.**

The predicate checks flag names, value types, *and descriptions*. A flag
documented in the skill as `--dry-run  Preview without writing` but defined
in clap as `--dry-run  Plan only, no writes` is a finding.

---

## Option B: Single-Source Registry

Option B is the future move when the authoring cost of maintaining the
invocation string in both clap and SKILL.md becomes painful, or when MCP
exposure enters the roadmap.

In Option B, a registry file (e.g. `conductr-registry.toml`) is the single
source of truth for every subcommand's name, flags, types, and descriptions.
`clap` derives from the registry at build time. Skill templates are generated
from the registry. Drift is structurally impossible.

**This ADR does not adopt Option B.** The convention-test approach (predicate
+ CI + architect) is simpler to implement and sufficient for the current
adapter count (CLI + skills). Option B will be revisited when:

- The authoring cost of keeping both in sync dominates development time, OR
- MCP is added as a third adapter.

Drift *count* alone is not a trigger — the predicate already catches drift.

---

## Cross-references

- Closes the ADR portion of #76.
- Implementation of `check_cli_skill_parity`: see the parity predicate ticket.
- `conductr-pod` migration: see the `pod` parent ticket and `save-state` /
  `diagnose` / `heal` skill split tickets.
- `conductr architect` promotion to call the predicate: see the architect
  promotion ticket.
- `conductr begin` revision to be Claude-required: see the begin revision
  ticket.
- `idle` process filing Architecture findings: see [`operations/idle.md`](../operations/idle.md).
