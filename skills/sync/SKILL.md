---
name: sync
description: Schedule and reconcile conductr calendar events in Google Calendar. Places decision slots for open human-labeled issues (priority by downstream blocking count), lets users manually schedule test slots, and fully reconciles future events on every run. Invoked automatically after every orchestrate run.
cli_parity: true
---

# Sync

Reconcile the conductr calendar: place `[conductr:*]` events into Google Calendar and keep them in sync with the current state of the issue tracker.

## When to invoke

- **Automatically** — at the end of every `orchestrate` run (see the orchestrate skill § Calendar Sync).
- **Manually** — `conductr sync` to reconcile; `conductr sync schedule-test` to add a test slot.

## Prerequisites

- Google Calendar MCP tools available in the current session. The tool prefix
  varies by server version (e.g. `mcp__e182bec4-3b31-4422-aed8-27fa867a8de5__`
  or `mcp__claude_ai_Google_Calendar__`). Discover the active prefix from the
  tools listed in your session; the operation names are stable: `list_events`,
  `create_event`, `update_event`, `delete_event`.
- Calendar target: read `calendar_id` from `.conductr [calendar]`; fall back to
  the user's primary calendar if absent.
- Events follow the title grammar in `docs/calendar.md`.

If the Google Calendar MCP tools are not available, report this to the user and skip — do not attempt to simulate or stub the sync.

## Subcommands

### `conductr sync` (default reconcile)

Runs a full reconcile of future calendar events against the current issue state.

#### Step 1 — Read window inventory

Call `mcp__claude_ai_Google_Calendar__list-events` (or equivalent) to fetch all `[conductr:*]` events from now onward. Partition into:

- **Windows** — titles matching `[conductr:window]`, `[conductr:window:<tag>]`, or `[conductr:window:<tag1>,<tag2>]`
- **Blockers** — titles matching `[conductr:blocked]`
- **Scheduled items** — titles matching `[conductr:<tag>] <kind>: <subject>` where `<kind>` ∈ `{decision, test, review}`

Past events (start time < now) are **receipts** — never read, never modified.

#### Step 2 — Read issue state

```
gh issue list --label human --state open --json number,title,labels,body
```

For each open `human`-labeled issue, compute:

- **Priority** — count of other open issues whose dependency chain includes this one (i.e., issues blocked directly or transitively by this one). Higher count = scheduled sooner.
- **Tag** — the primary tag from `.conductr` for this issue, if any. Used to match against tagged windows.
- **Existing slot** — search scheduled items for a `conductr-id` line in the description matching the issue number.

#### Step 3 — Compute eligible windows

Split each window into non-overlapping 30-minute slots (greedy packing from window start). A slot is eligible if:

1. Its start time is in the future.
2. No `[conductr:blocked]` event fully or partially overlaps it.
3. No existing `[conductr:*]` scheduled event occupies it.
4. The window's tag matches the issue's tag (or the window is `[conductr:window]` / `[conductr:window:*]` — untagged or wildcard; multi-tag windows use union semantics — any matching tag qualifies).

#### Step 4 — Reconcile decision slots

Sort open `human`-labeled issues by priority (descending). For each issue in priority order:

1. If a slot already exists (`conductr-id` found) and falls in an eligible window, **leave it alone** — respect any manual time-drag (see `docs/calendar.md` § Lifecycle).
2. If a slot exists but the window it occupied is gone or blocked, **delete the stale event** and re-schedule.
3. If no slot exists, **create** a new event in the earliest available 30-minute slot.

Event creation (Google Calendar MCP `create_event`):
- **Title**: `[conductr:<tag>] decision: <issue-title>` (omit tag prefix if the issue has no tag)
- **Duration**: 30 minutes
- **Description** (preserve these two lines across all future edits):
  ```
  conductr-id: <issue-number>
  originally-scheduled: <ISO-8601 start time>
  ```

#### Step 5 — Remove orphaned slots

Any scheduled `decision` event whose `conductr-id` does not match any currently open `human`-labeled issue is **deleted** — the issue was closed or re-labeled since the last run.

#### Step 6 — Report

```
Calendar sync complete
======================
Added:   [conductr:auth] decision: Fix login timeout   (issue #42, slot 2026-05-15 09:00)
Kept:    [conductr:api] decision: Rate limiting design  (issue #37, manually dragged)
Deleted: [conductr:auth] decision: Add OAuth            (issue #29 — now closed)
Windows: 3 eligible, 2 slots filled
```

### `conductr sync schedule-test`

Manually place a single test slot in the next eligible window.

1. Read `$ARGUMENTS` for an optional tag and subject. If none provided, prompt the user.
2. Fetch windows and compute eligible slots as in Steps 1 and 3 above.
3. Find the earliest available 30-minute slot across all eligible windows.
4. Create the event (Google Calendar MCP `create_event`):
   - **Title**: `[conductr:<tag>] test: <subject>` (use `*` if no tag given)
   - **Duration**: 30 minutes
   - **Description**:
     ```
     conductr-id: test-<uuid>
     originally-scheduled: <ISO-8601 start time>
     ```
5. Report the created slot and its window.

## Lifecycle and time-drag detection

See `docs/calendar.md` § Lifecycle for the full contract. Summary:

- **Past events**: never touched — they are receipts.
- **Future events**: fully reconciled each run.
- **Manual time-drag**: if an event's current start differs from `originally-scheduled` in its description, the user moved it. Respect the position — do not re-schedule. Warn (but do not move) if it has drifted outside an eligible window.

## MCP tools used

| Purpose | Operation name |
|---------|----------------|
| List calendar events | `list_events` |
| Create an event | `create_event` |
| Update an event | `update_event` |
| Delete an event | `delete_event` |

The MCP server prefix varies by session (e.g. `mcp__e182bec4-3b31-4422-aed8-27fa867a8de5__`
or `mcp__claude_ai_Google_Calendar__`). Discover the active prefix from the
Google Calendar tools listed in your session and combine it with the operation
names above. If a specific operation is missing, report to the user and skip
that step rather than erroring out.

## Non-goals (bootstrap phase)

- Native gcal dependency — tracked in the Rust follow-up.
- Cron scheduling of `conductr sync` independent of orchestrate — same Rust follow-up.
- Per-tag duration overrides — waiting for #19's `.conductr` schema.
- `review` slots — grammar reserved, scheduler does not emit them yet.
- Bar-boundary / release-aware test slots — manual only.
