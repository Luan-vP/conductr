# Band — Roster, Slots, Voicings, Escalation

> ADR. Decision adopted in #6. This document records the vocabulary and
> operational rules for the crew of voices that runs on a machine.

## 1. Vocabulary

| Term | Definition |
|------|------------|
| **Band** | The crew of named voices running on a machine. |
| **Voice** | Name + role + model. A voice is what occupies one tmux slot at a time. |
| **Role** | Which skill a voice is running right now. Roles are skill labels; voices switch between roles depending on the task. |
| **Slot** | A tmux pane name from a fixed set. Slots are the runtime pod's concurrency capacity. |

### Roles and slot families

7 roles, 3 slot families:

| Role | Slot family |
|------|-------------|
| `architect` | `agent<n>` |
| `implementer` | `agent<n>` |
| `reviewer` | `qa<n>` |
| `tester` | `qa<n>` |
| `security` | `qa<n>` |
| `doc-writer` | `agent<n>` |
| `idle-sweeper` | `agent<n>` |

Special: the `conductr` slot is reserved for orchestrate (the dispatcher) — never a worker role.

## 2. Slot caps

| Slot | Cap | Override |
|------|-----|----------|
| `conductr` | 1 | — |
| `agent<n>` | 3 | `[orchestrate] max_parallel_beats` |
| `qa<n>` | 2 | `[orchestrate] max_parallel_qa` |

## 3. Voicing table

Default model assignment per role (tmux-pane execution):

| Role | Default voicing |
|------|----------------|
| `architect` | Opus |
| `implementer` | Sonnet |
| `reviewer` | Sonnet |
| `tester` | Haiku |
| `security` | Haiku |
| `doc-writer` | Sonnet |
| `idle-sweeper` | Opus |

`idle-sweeper` is Opus, not Haiku, because `idle` (post-#88) is Claude-required and
delegates to `architect review` — needs the architect-grade model.

**Local voicings (Ollama/llamacpp)**: no band roles are eligible. `LocalAgent` stays
available for `conductr run-task` only.

## 4. Web vs. tmux placement

Most beats run via the **web runner** (GH Actions Claude Code, triggered by an `@claude`
mention from orchestrate). Escalation to a **tmux pane** happens when either:

- The beat needs backend verification (db, migration, integration test, infra).
- The beat's complexity demands a bigger model than the web runner offers.

Architect annotates each issue with `runner = "web" | "tmux"` in the ARN at plan time.
Orchestrate reads ARN at dispatch.

### Escalation precedence

1. Manual label `runner/web` or `runner/tmux` if present.
2. ARN's `runner` field.
3. Auto-heuristic: complexity ≥ `[orchestrate] tmux_complexity_min` (default `L`) OR
   architect-detected backend signal from issue body (keywords: `db`, `migration`,
   `integration`, `backend`, `infrastructure`).

## 5. Distribution rule

**Shard the chord, not the band.** When work outpaces one machine, fan out tickets
across instances — each instance has its own full pod. Voices stay co-located per
machine.

v1 doesn't implement this; it's documented as the rule so future scaling work follows
it. The existing `conductr idle --shard-index k --shard-of n` is the precedent.

## Cross-links

- Decision: #6.
- Depends on: nothing — pure docs.
- Affects: schema ticket (`[band]` rewrite), architect runner annotation ticket,
  orchestrate web/tmux dispatch ticket.
