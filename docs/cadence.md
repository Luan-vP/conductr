# ADR: Cadence — Vocabulary, Tempo Schema, Algorithms, Coordination

**Status:** Adopted (decision made in #19; this document records it)
**Closes:** ADR portion of #19

---

## 1. Vocabulary

Time and work are **orthogonal axes**. A phrase is not bar-bounded; a chord
can span bars freely; a bar can hold N beats or zero.

| Term       | Meaning |
|------------|---------|
| **bar**    | Unit of time = 4 h. Six bars in a day. |
| **beat**   | A single task (≈ one PR). |
| **phrase** | A sequence of beats toward a meta-goal. |
| **chord**  | N parallel beats. |
| **day**    | 6 bars (24 h). |

---

## 2. `.conductr` schema additions

Three new sections extend the project config file:

```toml
# One row per closed PR. Written by `orchestrate` on PR close.
[[tempo.prs]]
number     = 21
phrase     = "begin"          # optional; absent = ad-hoc
chord      = "begin-impl-1"   # optional
complexity = "M"              # XS / S / M / L; defaults to M
opened     = "2026-05-01T09:12:00Z"
closed     = "2026-05-01T14:38:00Z"
merged     = true

# One row per CI run. Written by `orchestrate` on CI completion.
[[ci.runs]]
pr      = 21
minutes = 4.2
ts      = "2026-05-01T14:35:00Z"

# Orchestration tuning.
[orchestrate]
max_parallel_beats = 3        # chord size cap; per-repo override
```

Records are editable in place — there is no append-only ceremony.

---

## 3. Algorithms

### Complexity

Precedence chain (highest wins):

1. GitHub label — `complexity/xs`, `complexity/s`, `complexity/m`, `complexity/l`
2. Architect ARN estimate
3. Default: `M`

Four buckets: `XS` / `S` / `M` / `L`.

### Rolling average

30-day time-decayed window over per-bucket beat duration (`closed - opened`).
More recent beats are weighted more heavily than older ones.

### Maturity signal

The bucketed duration profile is the source of truth. A single derived number
(median across buckets, weighted by recent PR count) surfaces in
`cadence show` output and the README banner. This number is a convenience
display — the underlying per-bucket data is what tooling reads.

---

## 4. Write-back ownership

| Component        | Responsibility |
|------------------|----------------|
| `architect plan` | Writes `complexity` into ARNs at plan time. |
| `orchestrate`    | Reads complexity (precedence chain); appends `[[tempo.prs]]` rows on PR close; appends `[[ci.runs]]` rows on CI completion. Commits are **batched** — one `.conductr` write per orchestrate pass, not one per observation. |
| `idle`           | Backfills missing rows; surfaces anomalies as findings. |

---

## 5. Coordination

**In-flight label:** `conductr:in-flight` is set on an issue before
`@claude` dispatch and cleared on PR open or close.

**Secondary check:** before any dispatch, scan recent issue comments for
existing `@claude` mentions. This catches state the label missed (crashed
previous runs, manual triggers, pre-migration state).

**No lockfile, no `.conductr` live state** — the label + comment scan is the
entire coordination mechanism.

---

## 6. Architect / orchestrate behaviour

### Phrase scoping

`architect plan` infers phrases from dependency clusters; one cluster = one
phrase. Phrases are auto-named from the highest-level issue title in the
cluster.

### Pacing rule

None. Phrases are descriptive metadata for the tempo log; they do not gate
dispatch. `orchestrate` picks unblocked work regardless of phrase boundaries.

### Chord size cap

Default: `3`. Overridable per-repo via `[orchestrate] max_parallel_beats`.

---

## 7. Smells

*Reserved. Populated as real-world cases are observed.*

Examples that will go here:

- CI run time >> small-bucket beat time → orchestrator is racing the build.
- Beat durations diverge sharply across complexity buckets → estimates are
  miscalibrated; revisit the complexity precedence chain.

---

## Cross-links

- Implements the ADR portion of #19.
- DSL alignment (renames in `conductr-schedule` + examples): separate ticket.
- Schema migration, architect, orchestrate, maturity surfacing: separate
  implementation tickets.

## See also

- [`operations/cadence/`](../operations/cadence/) — orchestration timing rules
  (polling intervals, timeouts, re-classify triggers).
- [`operations/operations/`](../operations/operations/) — PR flow, dependency
  resolution, safety invariants.
- [`.conductr`](../.conductr) — live project config file (schema defined here).
