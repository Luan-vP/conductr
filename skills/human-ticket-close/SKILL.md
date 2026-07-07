---
name: human-ticket-close
description: Close out a `human`-labeled interview ticket once its open questions have been resolved through conversation with the human. Records the decisions as a closing comment, spins one implementation ticket per coherent unit of decided work (no `human` label, so orchestrate can pick them up), cross-links sibling tickets whose scope the decisions touch, and closes the interview ticket. Never posts or closes without explicit human confirmation of the drafted content.
cli: conductr human-ticket-close <issue-number...> [--repo <owner/repo>] [--dry-run]
tools: Read, Bash, WebFetch
model: opus
---

# Human-Ticket Close

`human`-labeled issues exist because a design/policy question needed a
person, not the orchestrator. Once that person has actually answered the
question — usually through an interview conducted live in conversation —
the decisions need to land somewhere durable and turn into work the
orchestrator *can* pick up. This skill is the other end of
`human-ticket-draft`: that skill proposes answers before the conversation,
this one records answers after it.

## When to use this

Invoke once an interview-style `human` ticket's clusters/questions have
been walked through and resolved — typically at the end of a session where
the human answered a series of scoped questions (e.g. via `AskUserQuestion`
or free-form back-and-forth), not when questions are still open.

## Invocation

| Form | Command |
|------|---------|
| Claude slash command | `/human-ticket-close <issue-number...>` |
| CLI (spawns this session) | `conductr human-ticket-close <issue-number...>` |

Takes one or more issue numbers — interview tickets that share a policy
boundary (e.g. two tickets interviewed in the same sitting) can be closed
together so cross-cutting decisions get recorded once, not duplicated per
ticket.

Flags:
- `--repo <owner/repo>` — defaults to the current repo.
- `--dry-run` — render all drafts (comment text, new issue bodies, close
  message) without posting anything.

## Workflow

### Phase 1 — Confirm the interview is actually resolved

Read each ticket's full body (`gh issue view <n> --json title,body,labels`).
Interview tickets structure their open questions as numbered clusters or a
checklist. For each cluster, there must be a concrete decision available
from the conversation — not "TBD" or a hedge. If a cluster is still open,
say so and stop for that ticket; don't half-close it. It's fine to close
one ticket in a batch while leaving another for a follow-up session.

### Phase 2 — Draft the closing record

For each resolved ticket, write a comment body that maps 1:1 onto the
ticket's own cluster structure:

```markdown
## Interview resolved

1. **<Cluster 1 name>** — <decision>. <one-line why, if given>.
2. **<Cluster 2 name>** — <decision>.
   ...

## Follow-up

- #<new-issue-1> — <title>
- #<new-issue-2> — <title>
```

Keep it a record, not a re-argument — the reasoning already happened in
conversation; this comment is what a future reader (or `git blame` on the
eventual code) checks against.

### Phase 3 — Draft follow-up implementation ticket(s)

Look at the ticket's own "Deliverables" / "Once captured" section for the
natural split into units of work; if absent, split by what can be
implemented and reviewed independently (e.g. a config/schema change vs.
the behavior that reads it).

Each follow-up ticket:

- **Does not carry the `human` label.** The policy question is closed;
  what's left is implementation, which orchestrate should be able to pick
  up on its own.
- **Links back** to the interview ticket in its body (`Resolves the design
  questions from #<interview-ticket>`) so the "why" is one click away.
- **States acceptance criteria** drawn directly from the decisions, not
  restated from scratch — copy the concrete parameters (cadence values,
  gate conditions, config keys) verbatim so nothing drifts between the
  decision and the ticket.
- **Declares dependencies** on sibling follow-up tickets with `Depends on
  #<n>` when one needs another's output (e.g. a deploy step that depends
  on a release step existing first).

### Phase 4 — Flag cross-cutting scope on sibling tickets

If a decision presupposes or narrows the scope of another *open* ticket
(e.g. resolving a cadence question by assuming a 3-tier version of a
preset scale that a separate ticket is still deciding), post a short FYI
comment on that sibling ticket linking back — don't close or resolve it,
just leave a pointer so whoever picks it up next isn't surprised.

### Phase 5 — Confirm, then execute

**Never post comments, create issues, or close tickets without showing the
human the drafted content first and getting explicit go-ahead in this
pass.** Once confirmed:

1. Create each follow-up issue (`gh issue create`), capture the returned
   numbers.
2. Post the closing comment on the interview ticket (Phase 2 draft),
   filled in with the real follow-up issue numbers.
3. Post any Phase 4 FYI comments on sibling tickets.
4. Close the interview ticket (`gh issue close <n> --comment "..."` or a
   short separate close message referencing the follow-up issues already
   linked above).

## Notes

- This skill assumes the resolving conversation already happened — it is
  not a substitute for `human-ticket-draft`'s pre-interview drafting, and
  it does not itself ask the interview questions.
- Batch related tickets (see Invocation) when their decisions overlap, so
  the record isn't split across near-duplicate comments.
- If the human's answers conflict with each other (e.g. picked mutually
  exclusive options in a multi-select), surface the conflict and get it
  resolved before drafting — don't silently pick one.
