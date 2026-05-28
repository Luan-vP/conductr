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

## Dispatch path (runner-aware)

When the orchestrator decides to dispatch a Ready issue it reads the **runner**
from the issue's labels (precedence: `runner:tmux` → `runner:web` → default `web`).

```
issue is Ready
    │
    ├─ runner = web  ──→  post @claude on the GitHub issue
    │                     GH Actions Claude Code picks it up
    │
    └─ runner = tmux ──→  check agent<n> slot pool
                              pool full (≥ max_parallel_beats)?
                              │  yes → defer to next cycle
                              │  no  → pick next free agent<n> slot
                              │        add conductr:in-flight label (atomic)
                              │        tmux new-session -d -s agent<n>
                              │        send: claude --dangerously-skip-permissions
                              │        send: /implementer --issue <n>
```

### Slot lifecycle

| Slot pool  | Spawned when          | Freed when                          | Cap config         |
| ---------- | --------------------- | ----------------------------------- | ------------------- |
| `agent<n>` | Implementation beat   | PR opens (in-flight label removed)  | `max_parallel_beats` |
| `qa<n>`    | PR opens (tmux issue) | PR closes / merges                  | `max_parallel_qa`    |
| `conductr` | Pod init              | Never (orchestrate-only slot)       | —                    |

The slot state machine is idempotent: if the orchestrator crashes mid-cycle the
next pass reconciles by comparing live tmux state with GitHub state.

### QA slot spawn

When a PR opens for a tmux-runner issue, the orchestrator checks whether a
`qa<n>` slot is available (`active_qa < max_parallel_qa`). If so it spawns a
new `qa<n>` session and sends `/qa --pr <number>` to start the review/test
skill.

### Stale-pane reconciliation (idle sweep)

The idle sweep calls `stale_agent_panes(sessions, in_flight_count)` to identify
`agent<n>` sessions whose corresponding work has completed (PR opened and
in-flight label was cleared). These sessions are killed before the next
orchestrate pass so their slot indices are reclaimed.

## Cron line shape (post-#196)

`cadence sync` wraps `orchestrate` and `idle` in `conductr pod ensure-session` so the
`conductr-<project_tag>` tmux session is created and has Claude running before the work begins.

```
# orchestrate
*/30 * * * * bash -lc 'conductr pod ensure-session <tag> --then "conductr orchestrate --repo owner/repo --once"' >> ~/.local/share/conductr/orchestrate.log 2>&1

# idle
*/30 * * * * bash -lc 'conductr pod ensure-session <tag> --then "/idle"' >> ~/.local/share/conductr/idle.log 2>&1
```

`ensure-session` behaviour:

| Session state   | Action                                              | Output token                    |
| --------------- | --------------------------------------------------- | ------------------------------- |
| Missing         | Create session, start Claude, wait for idle         | `session_missing_created`       |
| Existing idle   | No-op                                               | `session_reused`                |
| Existing crashed| Restart Claude, wait for idle                       | `session_missing_created`       |
| Existing busy   | Skip `--then` (try again next cron fire)            | `target_stale_or_not_consuming` |

`--then` dispatch rules:
- `/`-prefixed → `tmux send-keys` into the live Claude session (skill dispatch for `/idle`)
- anything else → `bash -c` subprocess (for `conductr orchestrate --repo X --once`)

### Migration

Re-running `cadence sync` on any project rewrites old cron lines to the new shape. The
`merge()` function strips any existing `# conductr-cron: <tag>-*` blocks and appends the
new ones, so no manual crontab editing is required.

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
