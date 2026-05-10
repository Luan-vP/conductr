# Cadence

The rhythm of orchestration: when to look, when to wait, when to repeat,
when to stop. Process steps live in [`../operations/`](../operations/).

## Documents

- [`orchestration-cycle.md`](./orchestration-cycle.md) — the
  survey → classify → execute → wait loop, with polling intervals and
  timeout windows.

## Skills

These cadences are implemented by:

- [`crates/conductr-orchestrate`](../../crates/conductr-orchestrate) — Rust
  port of the loop.
- [`vendor/poorchestrator/SKILL.md`](../../vendor/poorchestrator/SKILL.md) —
  the original orchestrate skill; cadence rules are extracted from §
  "Workflow / Mode A" and § "Error Handling".
