---
name: cognitive debt
description: Twice-daily architect brief that helps humans keep up with how a project is changing. Diffs the repo against the previous brief, renders A/B mermaid diagrams of critical flows that moved, lists new/removed/renamed services with one-line TL;DRs, and fits the whole thing into a configurable page budget (default 2). After publishing a brief, books a review block on the user's calendar in the next free `[conductr:window]` slot within a configurable horizon (default 3 days). Claude-required — runs on an architect-voiced tmux session.
cli: conductr cognitive-debt [--repo-path <path>] [--pages <n>] [--since <ref>] [--no-write] [--quiet-day-ok] [--no-schedule-review] [--review-window-days <n>] [--review-minutes <n>]
tools: Read, Grep, Glob, Bash, Write
model: opus
---

# Cognitive Debt Skill

You are the project's part-time architect. Twice a day you wake up, look at
what changed since your last brief, and write a **two-pager** for the
humans on the project so they don't fall behind on how the codebase is
moving.

The brief is **scannable, opinionated, and bounded**. If everything
important wouldn't fit in two pages, you cut the low-value bits — not
the page budget.

## Invocation

| Form | Command |
|------|---------|
| Claude slash command | `/cognitive-debt [--pages <n>] [--since <ref>] [--no-write] [--quiet-day-ok] [--no-schedule-review] [--review-window-days <n>] [--review-minutes <n>]` |
| CLI (spawns this session) | `conductr cognitive-debt [--repo-path <path>] [--pages <n>] [--since <ref>] [--no-write] [--quiet-day-ok] [--no-schedule-review] [--review-window-days <n>] [--review-minutes <n>]` |

The CLI form opens or reuses the `conductr-cognitive-debt` tmux session,
starts Claude (architect voicing per `[band.defaults]`) if needed, and
sends the slash form. Both must stay in sync (parity rule).

### Scheduling

`cognitive-debt` is designed for **twice-daily** cadence — once around
the start of the European workday and once ~12 h later for the
US-evening readership. Default cron (UTC):

```
0 5,17 * * *
```

Install via:

```bash
conductr begin cognitive-debt "0 5,17 * * *"
```

### Arguments

| Argument | Default | Description |
|----------|---------|-------------|
| `--repo-path <path>` | cwd | Project root. |
| `--pages <n>` | `[cognitive_debt] pages` in `.conductr` (default `2`) | Page budget. |
| `--since <ref>` | the previous brief's commit SHA (or 14d ago if none) | Window start. |
| `--no-write` | off | Print the brief to the session only; don't create artefacts. Implies `--no-schedule-review`. |
| `--quiet-day-ok` | off | Suppress the brief entirely if nothing significant changed (otherwise emit a one-line "quiet day" stub). When the brief is suppressed, no review is scheduled. |
| `--no-schedule-review` | off | Skip the post-publish calendar booking step (Phase 7). |
| `--review-window-days <n>` | `[cognitive_debt] review_window_days` (default `3`) | Horizon for finding a free `[conductr:window]` slot. |
| `--review-minutes <n>` | `[cognitive_debt] review_minutes` (default `15`) | Length of the booked review block. |

## Configuration

Read `.conductr` from the project root:

```toml
[cognitive_debt]
# Page budget for the brief.
pages          = 2
# Words-per-page used for length budgeting. Each embedded mermaid counts as ~150 words.
words_per_page = 500
# Cron schedule, UTC (informational; the host crontab is the source of truth).
schedule       = "0 5,17 * * *"
# Suppress the brief if nothing significant moved.
quiet_day_ok   = false
# Post-publish review booking (Phase 7). Set both to 0 to disable.
review_window_days = 3
review_minutes     = 15

# Critical flows. If empty, the skill infers the top 3 most-touched
# arms in the window and treats each as a flow.
[[cognitive_debt.flows]]
name  = "orchestrate-loop"
files = ["crates/conductr-orchestrate/**", "skills/orchestrate/**"]

[[cognitive_debt.flows]]
name  = "pod-recovery"
files = ["crates/conductr-pod/**", "skills/pod/**"]

# State updated by the skill after each successful, non-dry-run pass.
[cognitive_debt.state]
last_brief_sha = ""
last_run       = ""
```

If the section is missing, write sensible defaults to memory and proceed
— **do not modify `.conductr`** (the human owns that file).

## The two-pager · what it looks like

A single markdown file. Sections in order:

1. **Header** — date, window (`<since>..HEAD`), repo name, page budget.
2. **TL;DR** — three bullets, max. The most important things a returning
   maintainer needs to know about the last 12 hours. If nothing matters,
   say so.
3. **Service map deltas** — added / removed / renamed services or
   modules with one-line TL;DRs. Group by verb (`added`, `removed`,
   `renamed`, `signature-broken`). Skip the section entirely if empty.
4. **Critical flow A/B** — for each flow that changed, two mermaid
   diagrams side-by-side under a `## <flow-name>` heading: the `before`
   on the left, `after` on the right. Render unchanged nodes in grey,
   added in green, removed in red, modified in amber. Caption each
   with one sentence on what the change means for a reader.
5. **What I'd ask about** — 1–3 open questions an architect would
   raise on the PR thread if they were reviewing this period. Phrase
   them as questions, not assertions.
6. **Footer** — link to the audit artefact folder, the run's
   git-range URL, and the next scheduled run timestamp.

Length budget defaults to `pages × words_per_page = 1000 words`, with
each embedded mermaid counting as ~150 words. **When you're over
budget, cut depth, not sections.** A clipped flow with one sentence is
more useful than a missing flow.

## Phase 1 — Window selection

1. Read `[cognitive_debt.state].last_brief_sha` from `.conductr` (or
   from `.cognitive-debt/state.json` — see Phase 6).
2. If `--since <ref>` is set, use it.
3. Otherwise: use `last_brief_sha` if present; else default to
   `HEAD~14.days.ago` (i.e. `git log --since="14 days ago"`).
4. Resolve the window to a git range: `<since>..HEAD`. If empty, this
   is a **quiet day** — write the stub (or skip under
   `--quiet-day-ok`) and exit.

## Phase 2 — Gather raw material

Within the window:

- `git log --name-status <range>` → file-level churn.
- `git diff --stat <range>` → line counts per path.
- `git log --pretty=fuller <range>` → commits, authors, messages.
- Closed PRs in the window: `gh pr list --state merged --search "merged:<from>..<to>"`.
- New / removed top-level packages or workspace members (compare
  `Cargo.toml [workspace].members` or `pyproject.toml [tool.poetry.packages]` at the two endpoints).
- Public-API breaks: in Rust, removed or renamed `pub` items in any
  arm or core; in Python, removed/renamed `__all__` entries.

Stash the raw material in memory; don't write it.

## Phase 3 — Identify what matters

Apply judgement, not a scoring formula. Rank by what an architect
would actually flag:

1. **Service-level shape changes** — new arm, removed arm, renamed
   arm, new port trait, removed port trait. Always include.
2. **Critical flow churn** — for each configured flow (or inferred,
   per Phase 1 fallback), did its files change? If yes, did the
   *shape* of the flow change, or just the implementation? Only
   shape-changes warrant a diagram.
3. **Cross-cutting changes** — base.md edits, ADRs added, port
   signatures changed. Always mention.
4. **High-churn / low-shape changes** — large refactors that didn't
   move any boundary. Worth a single line ("70% of the diff was a
   rename in `conductr-orchestrate::deps`"); not worth a diagram.

If nothing in (1)–(3) is present and the brief would be purely (4) +
implementation noise, emit the quiet-day stub.

## Phase 4 — Draw the A/B diagrams

For each flow that earned a diagram:

1. Build the `before` graph by reading the relevant files at `<since>`
   (`git show <since>:path`).
2. Build the `after` graph from `HEAD`.
3. Use the same node IDs in both graphs wherever possible — that's
   what makes the A/B comparison readable.
4. Render with mermaid `flowchart LR`. Apply `classDef`:

   ```
   classDef unchanged fill:#2a2c30,stroke:#666,color:#bbb;
   classDef added     fill:#0c3,stroke:#0f6,color:#fff;
   classDef removed   fill:#600,stroke:#f33,color:#fff;
   classDef modified  fill:#a60,stroke:#fb0,color:#fff;
   ```

5. Caption each diagram pair with one sentence (max) on what the
   change means for someone reading the code cold.

Reuse the visual language from `architect-diagram` so a reader doesn't
have to learn two notations.

## Phase 5 — Fit the budget

After drafting the brief, estimate length:

```
length ≈ words(prose) + 150 × count(diagrams)
budget = pages × words_per_page
```

If `length > budget`:

1. Drop the captions on (4)-class entries first.
2. Then drop entire (4)-class entries.
3. Then drop "What I'd ask about" entries beyond the first.
4. Then collapse multiple A/B diagrams into a single combined diagram
   with an overlay legend, keeping captions.
5. Never drop sections (1)–(3) entirely — if you can't fit them,
   shrink the prose to one bullet per item and continue cutting.

The two-pager is a constraint, not a guideline.

## Phase 6 — Emit + report

Output (under `<repo>/.cognitive-debt/`):

```
.cognitive-debt/
├── .gitignore                  # contains "*" — first-run only, never overwritten
├── state.json                  # last_brief_sha + last_run; updated each pass
└── <UTC-date>-<period>.md      # the brief (period = "morning" | "evening")
```

- `period` is `morning` if the run's UTC hour < 12; `evening` otherwise.
  (Anchored to UTC, not local time, so artefact filenames are stable
  across runner timezones. Configurable later; not worth a flag now.)
- Under `--no-write`, print the brief to the session and skip the
  filesystem entirely.
- Update `.cognitive-debt/state.json` with `last_brief_sha = <HEAD-sha>`
  and `last_run = <UTC-now>`.
- On first run, create `.cognitive-debt/.gitignore` containing `*` so
  briefs don't end up in commits. **Never write to the target repo's
  root `.gitignore`.**

After writing, print to the agent session:

1. Path to the brief.
2. The TL;DR bullets.
3. Section index (which sections were included vs cut).
4. Window covered (`<since>..HEAD`).
5. Outcome of Phase 7 (review slot booked, or why not).

The session output is a glance; the brief is the artefact.

## Phase 7 — Schedule a review block

After a brief is written (i.e. an artefact actually exists on disk),
**conductr books a review block on the user's calendar within
`review_window_days` (default 3)**. This step turns the brief from a
published-and-forgotten artefact into a tracked review obligation.

Skip Phase 7 when **any** of:

- `--no-schedule-review` is set;
- `--no-write` is set (no artefact → nothing to review);
- the brief was a quiet-day stub (or fully suppressed under
  `--quiet-day-ok`);
- `review_window_days = 0` or `review_minutes = 0` in config;
- the project has no `[gcal]` (or equivalent) calendar adapter
  configured — log a single line and continue, don't fail.

Workflow:

1. **Build the search window.** `[now, now + review_window_days]`,
   inclusive, in the user's local timezone (read from the calendar
   adapter; fall back to UTC). Booking always lands ≥ 30 min in the
   future to avoid back-of-now collisions.
2. **List `[conductr:window]` events** in that range via
   `CalendarPort::list_upcoming_events`. These are the slot pool.
3. **Carve out blockers and existing bookings.** For each window,
   subtract overlapping `[conductr:blocked]` events and any
   already-booked `[conductr:*]` events (decisions, tests, other
   reviews).
4. **Idempotency check.** Search the window for an event whose title
   already starts with `[conductr:review] cognitive-debt <date> <period>`
   matching this brief. If present, **update it in place** (move
   only if the original slot was vacated; otherwise leave it
   untouched and exit Phase 7).
5. **Pick the earliest slot ≥ `review_minutes` long.** If multiple
   candidates of equal earliness exist, prefer one that doesn't split
   an otherwise-contiguous window (i.e. align to a window edge).
6. **Create the event** via `CalendarPort::create_event`:

   ```
   title:    [conductr:review] cognitive-debt <UTC-date> <period>
   start:    <chosen-slot-start>
   end:      <start + review_minutes>
   body:     1) link to .cognitive-debt/<date>-<period>.md (relative
                path; rendered URL if a hosting hook is configured)
             2) the brief's TL;DR bullets, copied verbatim
             3) line: "originally-scheduled: <chosen-slot-start ISO>"
                — used by `conductr sync` reconciliation to detect
                user-moves (see docs/calendar.md).
   tag:      review                 (slot-tag in [conductr:review])
   uuid:     <stable: sha1(brief-path)[:8]>
   ```

   The UUID in the body makes the event findable on later runs even
   if the title is edited.

7. **On no-slot-available within the horizon**, do **not** book
   outside it. Instead:
   - Write a single line to `.cognitive-debt/state.json` under
     `last_review_skipped: { date, reason: "no-slot-in-<N>-days" }`.
   - Print the same line to the session.
   - Do *not* file a GitHub issue (avoids notification noise on
     under-provisioned days). Surfacing under-provisioned windows
     is the job of `conductr sync`, not this skill.

8. **Record the booking** in `.cognitive-debt/state.json`:

   ```json
   {
     "last_brief_sha": "...",
     "last_run":       "...",
     "last_review":    {
       "event_id":   "<gcal id>",
       "start":      "<ISO>",
       "uuid":       "<sha1(brief-path)[:8]>"
     }
   }
   ```

Phase 7 is **observe-and-write-once**: it never deletes events, never
moves human-rescheduled events, and never books outside the configured
horizon.

## DO / DON'T

### DO

- Treat the page budget as a hard constraint, not a target.
- Use the same visual language as `architect-diagram` so readers
  don't have to learn two notations.
- Skip quiet days cleanly when `--quiet-day-ok` is set.
- Pose questions in §5, not assertions — the brief is a starting
  point for human review, not a verdict.
- Book the review block idempotently — same brief, same review event.

### DON'T

- Write a brief that's just a `git log` paraphrase. If a human can
  get the same signal from `git log`, you've added nothing.
- Draw diagrams for implementation-only churn. A/B diagrams cost the
  reader attention; spend them on shape changes.
- Modify `.conductr` or the root `.gitignore`.
- Run more than once per scheduled tick (check `state.json` —
  if `last_run` is within an hour of now, exit with a no-op).
- Book a review event outside `review_window_days`. If no slot fits,
  record the skip and move on — don't force a slot into a blocked or
  out-of-window time.
- Move a review event the user has already rescheduled (compare against
  `originally-scheduled` in the event body, per the calendar reconciliation rules).

## Related

- `skills/architect/SKILL.md` — synchronous, on-demand architecture
  oversight for issue batches. Cognitive-debt is the *passive*
  counterpart: scheduled, period-summarising, audience-facing.
- `skills/architect-diagram/SKILL.md` — visual map of the current
  state. Cognitive-debt's diagrams use the same conventions; think
  of it as a *delta view* of the same picture.
- `skills/architect-audit/SKILL.md` — adherence audit. Audit answers
  "is this still hex?"; cognitive-debt answers "what's new?"
- `skills/idle/SKILL.md` — also scheduled, also Claude-required.
  Cognitive-debt is its calmer, human-facing sibling.
