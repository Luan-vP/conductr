# Operations

The doctrine that governs how `conductr` orchestrates work. These docs are the
authoritative statement of process — both humans and agents read them.

The orchestration loop in `crates/conductr-orchestrate` and the `orchestrate`
skill (originally vendored from [`poorchestrator`](../vendor/poorchestrator))
implement the rules described here.

## Layout

- [`cadence/`](./cadence/) — **when** things happen. The orchestration cycle,
  polling intervals, timeouts, when to re-classify, when to stop.
- [`operations/`](./operations/) — **how** PRs flow. Review, merge, branch
  hygiene, dependency resolution, ARN convention, safety invariants.

## Reading order

If you're new to the project, read in this order:

1. [cadence/orchestration-cycle.md](./cadence/orchestration-cycle.md) — the
   beat that everything else dances to.
2. [operations/dependency-resolution.md](./operations/dependency-resolution.md)
   — how issues are classified and ordered.
3. [operations/pr-lifecycle.md](./operations/pr-lifecycle.md) — what happens
   to each PR from open to merged.
4. [operations/architecture-reference-notes.md](./operations/architecture-reference-notes.md)
   — how parallel work stays coherent.
5. [operations/safety.md](./operations/safety.md) — the invariants that
   constrain everything above.

## Skills

The skills referenced throughout these docs live in two places:

| Skill              | Location                                       | Implementation                                  |
| ------------------ | ---------------------------------------------- | ----------------------------------------------- |
| `orchestrate`      | [`vendor/poorchestrator/SKILL.md`](../vendor/poorchestrator/SKILL.md) | [`crates/conductr-orchestrate`](../crates/conductr-orchestrate) (Rust port) |
| `architect`        | [`vendor/poorchestrator/architect.md`](../vendor/poorchestrator/architect.md) | invoked by `orchestrate` before triggering batches |
| `conductr-pod`     | [`skills/conductr-pod/SKILL.md`](../skills/conductr-pod/SKILL.md) | [`crates/conductr-pod`](../crates/conductr-pod) |

The orchestrate skill is loaded as inert reference material (see the security
note in the top-level README); the binary executes the ported Rust algorithm,
not the markdown.
