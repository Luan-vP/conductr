# Operations

The doctrine that governs how `conductr` orchestrates work. These docs are the
authoritative statement of process — both humans and agents read them.

The orchestration loop in `crates/conductr-orchestrate` and the
[`orchestrate`](../skills/orchestrate/SKILL.md) skill implement the rules
described here.

## Layout

- [`cadence/`](./cadence/) — **when** things happen. The orchestration cycle,
  polling intervals, timeouts, when to re-classify, when to stop.
- [`operations/`](./operations/) — **how** PRs flow. Review, merge, branch
  hygiene, dependency resolution, ARN convention, safety invariants.
- [`idle.md`](./idle.md) — the peer of orchestrate. What the binary does
  when no work is pressing: architecture scan, round-robin module review,
  findings filed as issues for the next orchestrate pass.

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
| `orchestrate`      | [`skills/orchestrate/SKILL.md`](../skills/orchestrate/SKILL.md) | [`crates/conductr-orchestrate`](../crates/conductr-orchestrate) |
| `architect`        | [`skills/architect/architect.md`](../skills/architect/architect.md) | invoked by `orchestrate` before triggering batches |
| `pod`              | [`skills/pod/SKILL.md`](../skills/pod/SKILL.md) | [`crates/conductr-pod`](../crates/conductr-pod) |

The binary executes the Rust algorithm in `crates/conductr-orchestrate`; the
markdown skill file is the authoritative spec that the Rust port mirrors.
