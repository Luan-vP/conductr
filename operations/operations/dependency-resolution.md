# Dependency resolution

How the orchestrator decides what's ready to move and what has to wait.

## Parsing dependencies

Issue bodies are scanned (case-insensitive) for any of:

- `depends on #N` / `depends on #N, #M`
- `blocked by #N`
- `after #N`
- `requires #N`
- Checklist items like `- [ ] #N must be done first`

The Local Map of an existing ARN comment (see
[`architecture-reference-notes.md`](./architecture-reference-notes.md)) is
also parsed — it can encode dependencies visually that aren't restated in
prose.

## Validation

The result is a directed graph. Before execution it must satisfy:

- **No cycles.** A cycle is reported and the user is asked to clarify.
- **No open out-of-batch dependencies.** A dependency on an issue outside the
  current batch is fine if that issue is already closed/merged. If it's still
  open and not in the batch, the orchestrator warns and asks before
  proceeding.

## Classification

Each open issue lands in exactly one bucket:

| Bucket                | Condition                                                            | Action                                                  |
| --------------------- | -------------------------------------------------------------------- | ------------------------------------------------------- |
| **Ready**             | All deps closed/merged, no open PR, not yet triggered                | Trigger implementation                                  |
| **PR open**           | Has an open PR with passing/pending CI                               | Run the PR lifecycle ([`pr-lifecycle.md`](./pr-lifecycle.md)) |
| **PR failing**        | Has an open PR with failing CI                                       | Failure playbook ([`safety.md`](./safety.md))           |
| **Blocked**           | One or more dependency issues are still open                         | Skip this pass                                          |
| **Triggered, waiting**| Triggered but no PR has appeared yet                                 | Poll                                                    |
| **Human**             | Has the `human` label                                                | Assign and skip ([`safety.md`](./safety.md))            |

Classification is recomputed after every merge, not just at the start of a
pass — a single merge can move several issues from `Blocked` to `Ready`.

## Batching

When the orchestrator is given an explicit list of issues (or a label), it
runs a topological sort and executes batch-by-batch:

```
Execution plan:
  Batch 1 (parallel): #A, #B
  Batch 2 (after batch 1): #C (depends on #A)
  Batch 3 (after batch 2): #D (depends on #B, #C)
```

Within a batch, all issues are triggered in parallel. Across batches, work is
strictly sequential — the next batch starts only when every PR in the
previous batch is merged into `develop`.

## Triggering

The trigger itself is a single comment per issue:

```
gh issue comment <N> --body "@claude please implement"
```

Issues with the `human` label are *never* triggered this way (see
[`safety.md`](./safety.md)). They stay in the dependency graph; downstream
batches wait for them to be closed manually.

## Skills

- [`crates/conductr-orchestrate`](../../crates/conductr-orchestrate) — DAG
  build, classifier, and batch executor.
- [`skills/orchestrate/SKILL.md`](../../skills/orchestrate/SKILL.md) §§
  "A2. Classify issues" and "B3. Build dependency graph".
