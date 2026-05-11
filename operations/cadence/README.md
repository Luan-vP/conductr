# Cadence

The rhythm of orchestration: when to look, when to wait, when to repeat,
when to stop. Process steps live in [`../operations/`](../operations/).

## Documents

- [`orchestration-cycle.md`](./orchestration-cycle.md) — the
  survey → classify → execute → wait loop, with polling intervals and
  timeout windows.

## ADR

The vocabulary, tempo schema, algorithms, and coordination decisions are
recorded in [`docs/cadence.md`](../../docs/cadence.md).

## Skills

These cadences are implemented by:

- [`crates/conductr-orchestrate`](../../crates/conductr-orchestrate) — Rust
  implementation of the loop.
- [`skills/orchestrate/SKILL.md`](../../skills/orchestrate/SKILL.md) —
  the orchestrate skill; cadence rules are restated under § "Workflow /
  Mode A" and § "Error Handling".
