# conductr Calendar

Conductr uses Google Calendar as a scheduling layer. Events carry structured titles that the `sync` skill reads and writes. This document specifies the title grammar, the lifecycle contract, and the rhythm-layer convention.

## Title grammar

All conductr-managed events have titles that begin with a `[conductr:*]` tag. The parser splits on the first `:` after `conductr` to extract the structural keyword or tag.

### Window events (user-created)

Mark blocks of time that are eligible for scheduling.

| Pattern | Meaning |
|---------|---------|
| `[conductr:window]` | Generic window — any item may be scheduled here |
| `[conductr:window:<tag>]` | Tagged window — only items matching this tag are scheduled here |
| `[conductr:window:<tag1>,<tag2>]` | Multi-tag window — items matching any of these tags may be scheduled here |

### Blocker events (user-created)

Override a window, marking time as unavailable even if a window covers it.

| Pattern | Meaning |
|---------|---------|
| `[conductr:blocked]` | This span is not available for scheduling |

### Scheduled items (conductr-managed)

Created and maintained by `conductr sync`. Users should not create these manually.

| Pattern | Meaning |
|---------|---------|
| `[conductr:<tag>] decision: <subject>` | A decision slot for a human-labeled issue |
| `[conductr:<tag>] test: <subject>` | A manually-scheduled test slot |
| `[conductr:<tag>] review: <subject>` | Reserved — not yet emitted by the scheduler |

`<kind>` ∈ `{decision, test, review}`. After the outer `[conductr:<tag>]` prefix, the parser reads the kind from the token before `:`.

## Reserved names

`window` and `blocked` are reserved and may not be used as tag names in `.conductr`. If the token immediately after `conductr:` is `window` or `blocked`, the event is treated as a structural event — not a tagged scheduled item.

## Multi-tag union semantics

`[conductr:window:auth,api]` means "an item tagged `auth` **or** an item tagged `api` may be scheduled into this window." It is union semantics — any matching tag qualifies. It is not intersection.

## Rhythm-layer convention

The user shapes available time by creating window events. Conductr fills those windows with scheduled items.

**Standard pattern**: create `[conductr:window]` or `[conductr:window:<tag>]` events on recurring slots — e.g., every Tuesday and Thursday morning. These become the pool of time the scheduler draws from each `conductr sync` run.

**Client-day pattern**: when a client has an all-day calendar event, convert it to `[conductr:window:<client-tag>]` so that client's working day becomes review time for that project. This conversion is a manual user action — conductr does not auto-import external calendars.

**Blocking out time**: place `[conductr:blocked]` events over any span within a window that should not be scheduled. This is a sub-override: the parent window stays, but that portion is skipped by the scheduler.

## Lifecycle

### Past events are receipts

Any event whose start time is before the current run's clock is never read or modified. It is a historical record of what was scheduled.

### Future events are fully reconciled

On every `conductr sync` run, all future `[conductr:*]` events are compared against current issue state and updated as needed — new events created, stale events deleted, and existing events left in place if still valid.

### Manual time-drags are respected

Users may drag a scheduled event to a different time slot. Conductr detects this by comparing the event's current start time against the `originally-scheduled` timestamp stored in its description. If they differ, the user has manually moved the event — conductr leaves it in place.

Conductr will warn (but not move) if a manually-dragged event has drifted outside all eligible windows.

### Event identity

Every conductr-managed event carries two lines in its description:

```
conductr-id: <value>
originally-scheduled: <ISO-8601>
```

For decision slots, `<value>` is the GitHub issue number. For test slots, it is a UUID assigned at creation. This identity survives title edits and time moves, letting the skill find and update its own events reliably.

## Worked example

A week view with two window events, one blocker, and three scheduled items:

```
Monday    09:00–12:00  [conductr:window]                              ← generic window
          10:00–10:30  [conductr:blocked]                             ← sub-override (standup)
          10:30–11:00  [conductr:auth] decision: Fix login timeout    ← decision (issue #42)

Wednesday 14:00–17:00  [conductr:window:api]                          ← api-tagged window
          14:00–14:30  [conductr:api] decision: Rate limiting design   ← decision (issue #37)
          14:30–15:00  [conductr:*] test: auth integration suite       ← manual test slot

Friday    (no window — nothing can be scheduled)
```

**Reading the example:**

- Monday's window runs 09:00–12:00. The `[conductr:blocked]` event at 10:00–10:30 (standup) is carved out. The scheduler placed issue #42's decision at 10:30 — the next available slot after the blocker.
- Wednesday's window is tagged `api`. Only api-tagged items land here. Two back-to-back 30-minute slots are filled: one decision and one test.
- The test slot was created via `conductr sync schedule-test`, not by the automatic decision scheduler.
- Friday has no window — no items are scheduled there regardless of how many issues are waiting.
- The `[conductr:blocked]` event on Monday is a sub-override: it does not erase the parent window, it only marks that portion unavailable.
