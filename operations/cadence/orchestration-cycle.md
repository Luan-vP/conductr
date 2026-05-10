# Orchestration cycle

The orchestrator runs in a continuous loop. Each pass looks at the *current*
state of all open issues and PRs, decides what can move forward, and does it.

## The four phases

1. **Survey.** Read all open issues (with bodies and labels) and all open PRs
   (with branch names, bodies, and check status). One snapshot per pass.
2. **Classify.** Sort each issue into one of the buckets defined in
   [`../operations/dependency-resolution.md`](../operations/dependency-resolution.md).
3. **Execute.** Clear the path first (review and merge unblocking PRs), then
   trigger newly-unblocked issues. See
   [`../operations/pr-lifecycle.md`](../operations/pr-lifecycle.md).
4. **Wait, then repeat.** Poll for new PRs from triggered issues. As each one
   arrives, run it through the PR lifecycle. Re-classify after each merge —
   merging an issue often unblocks others.

## Timing

| Event                         | Cadence    |
| ----------------------------- | ---------- |
| Poll for new PRs after a trigger | every **60 s** |
| Per-issue PR-arrival timeout  | **30 min** |
| Bot-no-response check window  | **10 min** before inspecting `gh run list --workflow claude.yml` |
| Re-classify the landscape     | after **every merge** |

These intervals are not user-configurable today. They were chosen so the
orchestrator stays responsive without hammering the GitHub API on a large repo.

## Termination

The loop ends when one of the following is true:

- All issues in scope are closed (or merged via PR).
- Every remaining issue is `Blocked` (waiting on either a `human` issue or an
  out-of-batch dependency) and there is nothing left to merge.
- The user interrupts.
- A **cycle stall** is detected: a full pass made no progress (nothing to
  trigger, nothing to merge, everything waiting). The orchestrator reports the
  stall and asks the user how to proceed rather than spinning.

## Reporting

After every pass the orchestrator prints a short status block:

```
Cycle complete
==============
Merged: PR #P1 (for #C)
Triggered: #A, #B
Waiting: #A (PR pending), #B (PR pending)
Blocked: #D (waiting on #A, #B)
Human action needed: #E [human] (assigned to @<resolved-assignee>)
```

This block is part of the contract — humans and agents both read it to follow
along, and downstream tooling can parse it.

## Skills

- [`crates/conductr-orchestrate`](../../crates/conductr-orchestrate) —
  the orchestration loop.
- [`skills/orchestrate/SKILL.md`](../../skills/orchestrate/SKILL.md)
  §§ "Workflow / Mode A: auto-mode" and "Error Handling".
