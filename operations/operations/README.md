# PR operations

The processes around pull requests: classifying issues into work, triggering
implementation, reviewing, merging, keeping branches tidy, and the safety
invariants that constrain all of it.

The *when* lives in [`../cadence/`](../cadence/). Everything *how* lives here.

## Documents

- [`pr-lifecycle.md`](./pr-lifecycle.md) — the path a PR takes from "an issue
  was triggered" to "merged into `develop`".
- [`dependency-resolution.md`](./dependency-resolution.md) — parsing
  dependencies, building the DAG, classifying issues, batching.
- [`architecture-reference-notes.md`](./architecture-reference-notes.md) — the
  ARN convention that keeps parallel work coherent.
- [`safety.md`](./safety.md) — invariants (no force-push, no deploy, target
  the development branch), human-issue handling, and failure-mode playbooks.

## Skills

- [`crates/conductr-orchestrate`](../../crates/conductr-orchestrate) — Rust
  implementation of the orchestrate skill; runs the PR lifecycle end-to-end.
- [`skills/orchestrate/SKILL.md`](../../skills/orchestrate/SKILL.md) — the
  authoritative markdown spec the Rust code mirrors.
- [`skills/architect/architect.md`](../../skills/architect/architect.md) —
  the architect agent; produces ARNs, reviews PRs for architectural fit.
