---
name: human-ticket-draft
description: Scan open `human`-labeled GitHub issues and draft answers for the ones topically relevant to a provided change context. Used as a companion by /change-overview; can also be invoked standalone. Drafts are proposed defaults the human reacts to — never auto-posted as final answers.
cli: conductr human-ticket-draft [--context <text-or-path>] [--repo <owner/repo>] [--limit <n>] [--dry-run]
tools: Read, Bash, WebFetch
model: opus
---

# Human-Ticket Draft

Open `human`-labeled issues exist because *some* design call needs a person.
This skill lowers the cost of the person engaging by drafting defensible
defaults to react to, scoped to a specific change context.

The output is **drafts**, not answers. The human posts whichever ones they
agree with, edits the rest, or ignores the irrelevant ones.

## Invocation

| Form | Command |
|------|---------|
| Claude slash command | `/human-ticket-draft [--context <text-or-path>]` |
| CLI (spawns this session) | `conductr human-ticket-draft [--context <text-or-path>]` |

The CLI form opens or reuses the `conductr-human-ticket-draft` tmux
session, starts Claude if needed, and forwards the slash command. Both
forms must stay in sync (parity rule).

Flags:
- `--context <text-or-path>` — change summary to compare tickets against.
  Can be inline text or a path to a markdown file. If omitted, uses the
  current diff vs `origin/main`.
- `--repo <owner/repo>` — repo to scan. Defaults to the current repo
  (from `gh repo view` or the `repo` field in `.conductr`).
- `--limit <n>` — cap the number of drafts produced (default 5). Tickets
  beyond the cap are listed by number only.

## Workflow

### Phase 1 — List candidate tickets

```bash
gh issue list --repo <repo> --label human --state open --limit 50 \
  --json number,title,body,labels,createdAt,updatedAt
```

Skip closed tickets. Read full bodies for each — the open questions are
usually buried, not in the title.

### Phase 2 — Score topical relevance

For each ticket, judge relevance to the change context. Score on a
0–3 scale:

- **3 — Direct**: the change touches code or surface the ticket
  explicitly discusses. e.g. a ticket about "the cadence visual" while
  the diff modifies `crates/conductr-schedule/src/render.rs`.
- **2 — Adjacent**: the change touches the same subsystem or skill
  family. e.g. a ticket about dashboard safety presets while the diff
  modifies anything under `docs/dashboard/`.
- **1 — Tangential**: shares a concept or vocabulary but the diff
  doesn't move the ticket forward.
- **0 — Unrelated**: drop.

Keep tickets scoring ≥ 2. Sort descending by score, then by most-recent
`updatedAt`. Take the top `--limit` (default 5).

If no tickets score ≥ 2, **output nothing**. Quiet is fine — the report
section that embeds this output will simply omit the human-tickets block.

### Phase 3 — Draft an answer per ticket

For each surviving ticket, identify the open questions / deliverables /
checklist items. For each, propose a **defensible default** — not a
hedge, not "depends", an actual recommendation with one-sentence
rationale.

Drafting rules:

- **One recommendation per question**, with a brief why.
- **Mark explicit trade-offs** the human is choosing between — don't
  hide them. "Default to X over Y because X is simpler to revert if it
  bites; Y wins if we want Z."
- **Cite the diff** where the change makes a question easier to answer
  ("the topology change in this PR removes one of the options anyway").
- **Flag genuine uncertainty** instead of bluffing. If a question
  needs information the diff doesn't supply, say so and don't make up a
  default.
- **Don't propose a follow-up impl ticket** in the draft — that belongs
  to the human's reaction.

### Phase 4 — Render output

Print one section per ticket, in score-then-recency order:

```markdown
## Human-ticket drafts

### #<num> — <title>
<one-paragraph framing: why this ticket is relevant to the change, score>

**Drafted answers**

1. **<Question 1 paraphrased>** — <recommendation>. <one-sentence why>.
2. **<Question 2 paraphrased>** — <recommendation>. <one-sentence why>.
…

**Open uncertainties**

- <Any question where a defensible default can't be drafted>
- <…>

---
```

If invoked standalone (not via `/change-overview`), prepend a one-line
summary of the change context that drove the relevance scoring, so the
human knows what the drafts were filtered against.

## Notes

- **Never post drafts as comments without explicit human approval.** This
  skill renders to stdout (or the calling skill). Posting is a separate
  step the human consents to.
- **Be sparing.** Five drafts is plenty. Twenty drafts get skim-read
  and the whole skill becomes noise.
- **Quality over coverage.** A confident draft on three relevant tickets
  beats wishy-washy paragraphs on ten.
- If `--context` resolves to an empty diff (no changes), this skill
  exits without output — there's nothing to score relevance against.
