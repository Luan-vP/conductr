# Architecture Reference Notes (ARNs)

The main failure mode of parallel autonomous implementation is **interface
mismatch** — two agents build things that don't connect. ARNs prevent this by
making interfaces explicit per issue *before* implementation starts.

The architect agent
([`skills/architect/architect.md`](../../skills/architect/architect.md))
writes one ARN as a comment on each issue in a batch. Reviewers (human or
agent) check the resulting PR against its issue's ARN.

## Shape of an ARN

An ARN comment has two parts.

### Part 1 — Local Map

An ASCII dependency tree at the top of the comment, showing the **whole batch**
with the current issue marked `◄── YOU ARE HERE`:

```
## Architecture Reference Note

### Local Map
#A  Architecture ref (shared context)
 │
#B  Scaffold frontend
 ├── #C  API client ─────────────────────┐
 │    ├── #D  Parameter editor             │
 │    ├── #E  Preview panel ◄── YOU ARE HERE
 │    ├── #F  Fitness panel                ├── all parallel
 │    └── #G  Dashboard ◄─────────────────┘
 │         └── depends on #J
 ├── #H  App layout (parallel with #C)
 │
#J  Backend endpoint (no frontend deps)
 │
#K  Integration (depends on everything above)
```

Rules: real issue numbers and titles, full graph (not just neighbours),
parallel relationships annotated, distant branches collapsed with `...` if
the graph is large.

### Part 2 — Reference Note

Structured guidance for the implementing agent:

```markdown
### Scope
- **Modules affected**: directories/files this issue touches
- **New files**: files that will be created
- **Modified files**: files that will be changed (and what changes)

### Patterns to Follow
- Specific patterns from the codebase to replicate
- Naming, file organization, export patterns
- Reference existing code by path

### Interfaces & Contracts
- **Provides to others**: what this issue exposes that siblings depend on
- **Consumes from others**: what this issue needs from its dependencies
- **Shared types/interfaces**: data structures crossing issue boundaries
- Exact function signatures or component props at integration points

### Constraints
- What NOT to change (stability boundaries)
- Performance constraints
- Things the implementing agent might be tempted to do but shouldn't
- Prefer lean — no unnecessary back-compat shims, feature flags, or
  abstraction layers without an immediate need

### Testing Strategy
- What to test
- What existing tests must keep passing
- Integration points to verify

### Open Questions
- Decisions the implementing agent may need to make; suggest a default and
  flag for review
```

## When ARNs run

- **Always** for explicit batches via `--label` or an issue list, unless
  `--no-architect` is passed.
- **Conditionally** in auto-mode: if more than 3 unblocked issues exist and
  no ARNs are present, the architect runs first.

## During PR review

When reviewing a PR (manual or automated):

1. Read the linked issue's ARN.
2. Check the diff against the ARN's *Patterns*, *Interfaces & Contracts*, and
   *Constraints* sections.
3. If a sibling issue has been implemented in a way that requires an
   interface change, **update the affected ARNs immediately** and flag to the
   user before merging.

The ARN is a living document for the duration of the batch.

## Anti-patterns

- "Follow best practices" — vague is useless.
- Over-specifying *how* — the ARN gives the WHAT and WHY; the implementing
  agent owns the HOW.
- Assuming context from sibling ARNs — each ARN must stand alone.
- Adding back-compat shims, migration paths, or feature flags as
  dependencies between batched issues. Prefer direct change; keep the graph
  honest.

## Skills

- [`skills/architect/architect.md`](../../skills/architect/architect.md) —
  the architect agent definition.
- [`crates/conductr-orchestrate`](../../crates/conductr-orchestrate) — invokes
  the architect before triggering batches that need it.
