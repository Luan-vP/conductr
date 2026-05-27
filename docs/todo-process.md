# Todo process: beads → Notion (per-project) → Notion (personal)

Two-tier task model for personal projects. **Beads** (`br`) is the source of truth, sitting in each repo's `.beads/` directory. Notion is the human-facing surface — split into a per-project task list (full mirror) and a single personal inbox (filtered to actionable human work).

This is a parallel track to the GitHub-issue / `orchestrate` flow. Use it for personal repos that don't run on GitHub-issue throughput, or where dependency-graph-heavy planning matters more than PR-by-PR delivery.

## Tools

- `br` — [beads-rust](https://github.com/Dicklesworthstone/beads_rust). Local SQLite + JSONL issue tracker, no daemons, no auto-commits. Install: `bash <cloned-repo>/install.sh --verify`. Binary lands at `~/.local/bin/br`.
- Notion MCP — used to create databases, mirror issues, and propagate status. The MCP is what bridges beads JSONL to Notion pages.

## Why two tiers

- **Per-project DB** — a real backlog. Includes blocked issues. Engineers / agents can see the dependency landscape, claim ready work, and look ahead.
- **Personal list** — an inbox. Filtered to *only* the issues a human actually has to do, across all projects. Blocked work, agent-doable work, and tracking artifacts (epics) stay out.

The split prevents either surface from becoming useless: a backlog that's also an inbox grows unreadable; an inbox that mirrors the whole backlog drowns the human.

## Notion structure

```
Projects (page, lives at workspace root)
├── 📋 Project Tasks Template (page) — canonical schema reference
│   └── Tasks (template) (database)
└── 🧠 <project> (page) — one per repo
    └── Tasks (database) — full mirror of the repo's beads issues

Control Panel
└── Task Dashboard
    └── Task List (database) — personal inbox, multi-project
```

To stand up a new project: create a project page under Projects, then create a Tasks database with the canonical schema (DDL below).

## Canonical schema — per-project Tasks DB

```sql
CREATE TABLE (
  "Name" TITLE,
  "Status" SELECT('Not started':gray, 'In progress':blue, 'Blocked':orange, 'Done':green),
  "Priority" SELECT('P0':red, 'P1':orange, 'P2':yellow, 'P3':blue, 'P4':gray),
  "Type" SELECT('epic':purple, 'task':blue, 'bug':red, 'feature':green, 'chore':gray),
  "Owner" SELECT('Person':orange, 'Agent':blue, 'Auto':gray),
  "Mirror to personal" CHECKBOX,
  "Personal Mirror URL" URL,
  "Beads ID" RICH_TEXT,
  "External Ref" URL,
  "Due Date" DATE,
  "Estimate (min)" NUMBER,
  "Tags" MULTI_SELECT(),
  "Created" CREATED_TIME
)
```

Use `notion-create-database` (Notion MCP) with this DDL. Field rationale:

- **Priority is P0–P4** to match beads natively — no lossy mapping at sync time.
- **Status has `Blocked`** — beads tracks dependencies; many issues are blocked at any moment. This is the mid-project view.
- **Owner** drives who's expected to act:
  - `Person` — needs human (gcloud commands, console UIs, design decisions, account-level changes)
  - `Agent` — Claude / Codex / orchestrate can do it end-to-end
  - `Auto` — tracking artifact (epics, parents); no real work in this row
- **Mirror to personal** is the explicit opt-in for the personal-list filter. Required even when `Owner = Person` — an explicit checkbox lets you keep some Person work out of the inbox if it's already on a calendar, in someone else's plate, etc.
- **Personal Mirror URL** is filled by sync after the personal page is created. Lets re-runs detect "already mirrored".
- **Beads ID** is the round-trip identifier. Beads owns the canonical record; this column is the join key.
- **External Ref** is for upstream pointers (GitHub issue, Linear ticket, etc.) when relevant.

## Personal Task List — additions

The personal list pre-existed; mirror-aware columns were added on top:

| Property | Type | Purpose |
|---|---|---|
| `Project` | select | Which project this task came from. Add a select option per active project. |
| `Source URL` | URL | Direct link to the project Tasks DB row. The "go look at the dependency graph" link. |

Plus the existing fields: `Name`, `Priority Level` (Low/Medium/High — coarser than P0–P4), `Status` (Not started / In progress / Done — no Blocked), `Due Date`, `Category` (Personal / Art / Client).

The personal list is intentionally simpler. It's an inbox — Done means done, not "merged but not deployed".

## Sync rules

### beads → project Tasks DB (full mirror)

For every beads issue:

1. Match by `Beads ID`. Upsert.
2. Status: `closed` → Done; `in_progress` → In progress; `open` AND ready → Not started; `open` AND blocked → Blocked.
3. Priority: pass through P0–P4.
4. Type: pass through.
5. After creating/updating the Notion page, write the Notion page URL back to the beads issue: `br update <id> --external-ref <url>`. The `external_ref` is the round-trip link from beads.

### project Tasks DB → personal Task List (filtered)

For each row where:

- `Owner = Person`, AND
- `Mirror to personal` is checked, AND
- `Status != Done`, AND
- `Personal Mirror URL` is empty

Create a personal Task List page with:

- `Name = "[<project>] " + <project task name>`
- `Project = <project>` (select; add the option to the schema if new)
- `Source URL = <project task page URL>`
- `Priority Level`: P0/P1 → High, P2 → Medium, P3/P4 → Low
- `Category = ["Personal"]`
- `Status = Not started` (or In progress if that's where the beads issue is)

Write the new personal page URL back into `Personal Mirror URL` on the project row.

When status changes in beads, propagate to both rows. `Done` in the project DB triggers `Done` in the personal list.

## Workflow

A typical session:

1. `br ready` in the repo — see what's unblocked.
2. Pick a beads issue, work on it. (Or delegate to an agent — beads has `coordination` subcommands for swarm-claim diagnosis.)
3. `br update <id> -s closed` when done.
4. Run sync: re-mirror beads → project DB; promote any newly-ready Person+Mirror tasks to the personal list.

Today the sync runs **manually inside a Claude session** using the Notion MCP — Claude reads `.beads/issues.jsonl`, makes the upserts. The script-ification of this loop is itself a beads issue per project (it's the "Coordination" task).

## Beads CLI cheat sheet

```bash
br init                           # create .beads/ in a repo
br create "title" --slug X --type task -p 1 -d "..." [--parent <id>] --silent
br dep add <child> <parent> --type blocks
br ready                          # unblocked issues
br stats                          # progress
br list --json                    # all issues, machine-readable
br update <id> -s in_progress
br update <id> --external-ref <url>
br show <id>
```

## Property-update gotchas (Notion MCP)

- `update-page` properties named literally `id` or `url` need a `userDefined:` prefix. Other property names — including `Source URL` — use the plain name. Don't reflexively prefix.
- Checkbox values: `"__YES__"` / `"__NO__"`.
- Multi-select values: JSON-array string, e.g. `"[\"infra\", \"proxy\"]"`.
- Date properties use expanded keys: `date:<column>:start`, `date:<column>:end`, `date:<column>:is_datetime`.
- The `Tags` multi-select rejects values that aren't pre-defined options. If the type's tag pool is set explicitly in DDL, only those are accepted; if defined as `MULTI_SELECT()` (empty), Notion auto-creates options as you write them.

## When to use this vs `orchestrate`

| | beads + Notion (this doc) | `orchestrate` (GitHub issues) |
|---|---|---|
| Source of truth | local SQLite + JSONL | GitHub Issues |
| Granularity | dependency graph, ready-set | issue list, dependency comments |
| Surface | Notion (project DB + personal inbox) | GitHub PRs + checks |
| Best for | personal repos, planning-heavy work, agent-swarm with claim coordination | repos with CI / GitHub Action bot, PR-throughput-driven delivery |
| Human inbox | Notion personal Task List, filtered by Owner=Person | issues assigned to you / `human` label |

They can coexist on the same repo. Beads can carry the granular dependency graph; GitHub Issues can carry the human-visible roadmap. Cross-link via `External Ref`.
