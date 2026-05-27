---
name: change-overview
description: Manual-invocation change impact report. Assesses topology changes, LOC delta as a percentage of the codebase, user-flow changes, compliance / legal exposure, and newly-introduced tech debt for a given diff. References the human-ticket-draft skill to surface relevant open `human`-labeled issues with draft answers. Used at the end of a review process when the human wants the bigger-picture view of what just landed.
cli: conductr change-overview [<base>] [--repo-path <path>] [--dry-run]
tools: Read, Grep, Glob, Bash
model: opus
---

# Change Overview

Produce a one-page impact report for a diff. Not a code review — code review
catches per-line bugs. Change overview answers the question **"what is this
change actually doing to the codebase?"** across five axes, and surfaces
human-ticket drafts the diff might unblock.

## Invocation

| Form | Command |
|------|---------|
| Claude slash command | `/change-overview [<base>]` |
| CLI (spawns this session) | `conductr change-overview [<base>]` |

The CLI form opens or reuses the `conductr-change-overview` tmux session,
starts Claude if needed, and sends `/change-overview [<base>]`. Both forms
must stay in sync (parity rule).

`<base>` is the comparison base. Defaults to `origin/main`. Accepts:
- a branch name (`origin/main`, `develop`)
- a tag (`v0.3.17`)
- a commit SHA
- a PR number (`#142`) — resolves to that PR's base..head

If the working tree is dirty, the diff is `<base>..HEAD` plus uncommitted
changes.

## Workflow

### Phase 1 — Establish the diff

Compute the comparison range:

```bash
# branch / tag / SHA
git diff <base>..HEAD --stat
git diff <base>..HEAD --name-only

# PR form: resolve base via gh first
gh pr view <num> --json baseRefName -q .baseRefName
```

Capture: changed files, lines added, lines removed, files added/deleted/renamed.

### Phase 2 — LOC delta + % of total

1. **Diff LOC**: lines added + lines removed from `git diff --shortstat`.
2. **Total LOC**: use `tokei` if available, else `cloc`, else `find … | xargs wc -l`
   filtered to tracked code files. Always exclude `target/`, `node_modules/`,
   lockfiles, generated code.
3. **Percentage**: `(diff_loc / total_loc) * 100`, rounded to one decimal.

Report all three. Flag `Large` if percentage > 2%, `Sweeping` if > 5%.

### Phase 3 — Topology changes

Topology = the shape of the system, not the lines inside it. Look for:

- **New / removed crates** (workspace `Cargo.toml` member list changed).
- **New / removed ports** (new trait in a `*-ports` crate; new file under
  `crates/*-ports/src/`).
- **New / removed adapters** (new module under `crates/*-adapters/src/`).
- **New cross-arm edges** — an arm crate gaining a dependency on another
  arm. Hexagonal rule violation candidate; flag for follow-up.
- **Public-API changes** — added or removed `pub fn` / `pub struct` /
  `pub trait` items in lib roots.
- **CLI surface changes** — new top-level subcommand, removed subcommand,
  changed flag set. (Cross-check `crates/conductr/src/main.rs` `Cmd` enum.)
- **Skill surface changes** — new or removed `skills/*/SKILL.md`.

If any topology change is detected, delegate the deep audit to the
architect skill:

```bash
conductr architect review <pr-or-base>
```

Quote the architect's verdict in the report; do not duplicate its checks.

### Phase 4 — User-flow changes

A "user flow" is any path a human takes through the product surface. For
this repo that's:

- CLI subcommand behaviour (changed args, changed output, changed exit
  codes).
- Skill workflows (changed `/<skill>` slash-command surface or workflow
  ordering inside a `SKILL.md`).
- Dashboard UI flows (when present — `docs/dashboard/`).
- Cron-driven cadence changes (`[cadence]` block edits, default cron
  expressions).

Heuristic: any diff that touches `crates/conductr/src/main.rs`,
`skills/*/SKILL.md`, `docs/dashboard/`, or a `.conductr` cadence default
is a user-flow change. List each one with a one-line before/after.

### Phase 5 — Compliance / legal exposure

Scan the diff for any of:

- **Licence headers** — files added without the project's header;
  third-party code vendored in (look for unfamiliar copyright lines).
- **Dependency additions** — new entries in `Cargo.toml`, `package.json`,
  or lockfiles. Check the licence of each new direct dependency against
  the project's allowed-licence list (MIT / Apache-2.0 / BSD-3-Clause /
  ISC by default). GPL / AGPL / SSPL / proprietary licences need explicit
  human review — flag prominently.
- **Personal-data handling** — keywords introduced that suggest new PII
  flow: `email`, `phone`, `address`, `dob`, `password`, `token`, `secret`,
  `pii`, `personal_data`. False-positive rate is high; just surface, don't
  block.
- **Secrets** — anything that looks like a hard-coded credential
  (`AKIA…`, `sk-…`, `ghp_…`, base64 blobs ≥ 40 chars in source).
- **Telemetry / tracking** — new analytics / telemetry endpoints
  introduced.

Each finding gets a severity:
- `Block` — hard licence conflict, leaked secret. Stops the report
  recommending merge.
- `Review` — new PII flow, new direct dependency with non-default
  licence. Needs human eyeballs.
- `Note` — anything else that's worth knowing but not blocking.

### Phase 6 — New tech debt

Look for debt the diff introduces (not pre-existing debt):

- **TODOs / FIXMEs / HACKs / XXXs** added.
- **`unwrap()` / `expect()` / `panic!`** in non-test code.
- **`unimplemented!()` / `todo!()`** in non-test code.
- **`#[allow(…)]`** added (especially `dead_code`, `clippy::*`).
- **Test-skipped paths** — new code without corresponding tests, judged
  by whether a `tests/` or `#[cfg(test)]` block was touched alongside the
  source change.
- **`@ts-ignore` / `eslint-disable`** added (JS/TS files).
- **Schema migrations without rollback** — new migration file without a
  paired down-migration.

For each item: file:line, what was added, suggested follow-up label
(e.g. `tech-debt`, `quality`, `architecture`).

### Phase 7 — Human-ticket drafts (delegated)

Pass the change summary to the `human-ticket-draft` skill:

```bash
/human-ticket-draft --context <inline-summary>
```

That skill scans open `human`-labeled issues, filters for topical
relevance to the change context, and returns drafted answers per ticket.
Embed those drafts as a section in this report — do not duplicate the
scanning logic.

If no relevant tickets, omit the section entirely (do not print "no
tickets found"; quiet is fine).

### Phase 8 — Render the report

Print to stdout in this order:

```markdown
# Change Overview — <base>..HEAD

## Magnitude
- Files changed: N
- Lines: +A −B   (delta of D LOC, X.X% of total Y LOC)
- Verdict: <Small / Medium / Large / Sweeping>

## Topology
<bulleted list of structural changes; architect verdict quoted if invoked>

## User flows
<bulleted list of surface changes; one line per flow>

## Compliance / legal
<grouped by severity: Block, Review, Note. Omit groups with no entries.>

## Tech debt introduced
<file:line list; group by category>

## Human-ticket drafts
<embedded from /human-ticket-draft; omit section if none>

## Recommended next steps
<2–4 bullets: what the human should do before merging>
```

If invoked inside a PR (detected via `gh pr view` succeeding for the
current branch), also offer to post the report as a PR comment. Ask
before posting.

## Notes

- This skill is **manual-only**. It does not run from cron, idle, or
  orchestrate. The human invokes it when they want the view.
- It does not produce `Finding`s for the idle inbox. Its output is
  prose for the human; tech-debt items become follow-up tickets only if
  the human chooses to file them.
- The architect skill is the source of truth for hexagonal-rule
  enforcement. Change-overview delegates to it; do not re-implement
  those rules here.
- Keep the report under a screen-and-a-half. If the report gets long,
  trim narrative; the bulleted axes are load-bearing.
