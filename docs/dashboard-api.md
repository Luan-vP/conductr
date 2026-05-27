# Dashboard API contract

The data model and command surface that the conductr daemon exposes
and that every outlet (VSCode, Android tablet, ratatui TUI, future
web) consumes. This document is the **source of truth**: drift
between daemon, core crate, and outlets relative to this doc is a
bug.

Tracked under #146; meta-tracker #181.

## 0. Why this exists

Multiple outlets need the same live snapshot of conductr state.
Letting each one scrape `tmux`, `crontab`, `gh`, the filesystem, and
the ollama/llamacpp HTTP endpoints independently means three (later
four) copies of fragile glue, three sets of refresh logic, and three
chances for the picture to disagree.

The daemon (#147) does the scraping once and exposes a structured
view. The core crate (#149) carries the types in Rust and emits TS
bindings so outlets don't redefine them. This contract is what holds
the two ends honest.

## 1. Transport

JSON over HTTP/1.1 on a local Unix socket in v1. Frames follow the
shape in §4 (envelope) and §8 (error). Push events use Server-Sent
Events on `/events`.

- **Socket path:** `$XDG_RUNTIME_DIR/conductr-daemon.sock` on Linux;
  `~/.local/share/conductr/daemon.sock` on macOS.
- **Content-type:** `application/json` on REST; `text/event-stream`
  on `/events`.
- **No binary protocols in v1.** Protobuf / msgpack are out of
  scope; the data volume doesn't justify the cost.

Multi-host (#152, v2) keeps JSON over HTTP but moves the transport
to TCP+TLS over the federation network (Tailscale / static IP).
Same envelope, same endpoints.

## 2. Versioning

Semver on the **protocol**, separately from the daemon
implementation version. Both appear in every response envelope and
on `GET /version`.

```json
{
  "protocol": "1.0.0",
  "impl":     "0.4.2"
}
```

- Outlets pin a **major** protocol version. Minor/patch updates are
  applied automatically.
- The daemon supports the last `N=1` major in v1 (so v1 outlets work
  against v1 daemons only). `N` may grow as the protocol matures.
- On a protocol mismatch, the daemon returns `426 Upgrade Required`
  with an error body (§8) carrying the supported range. Outlets
  surface that as a "please update" banner; they do not attempt
  fallback parsing.

## 3. Authentication

| Phase | Transport       | Auth                                |
|-------|-----------------|-------------------------------------|
| v1    | Unix socket     | OS user (socket file mode `0600`)   |
| v2    | TCP+TLS (LAN)   | Bearer token + optional mTLS (#152) |

In v1 there is no in-band auth. The daemon refuses to bind a TCP
socket. The Android outlet works in v1 by relying on whatever tunnel
the user runs (Tailscale, SSH forward) and presenting the socket to
the tablet; properly authenticated network access lands with #152.

## 4. Data model

Every response from a REST endpoint is wrapped in an envelope:

```json
{
  "protocol":    "1.0.0",
  "impl":        "0.4.2",
  "host":        "luan-desk",
  "snapshot_at": "2026-05-27T13:42:08Z",
  "data":        { ... typed payload, shape per endpoint ... }
}
```

- `host` is `gethostname()`. v1 has one host per envelope; v2 may
  return a federated envelope (TBD by #152).
- `snapshot_at` is the daemon's clock at the moment the snapshot
  was assembled. If a section is stale (per §8 `STALE_CACHE`), the
  per-section `stale_at` overrides this for that section only.

The remainder of this section lists every payload shape the daemon
serves. Each type maps 1:1 to a Rust struct in
`crates/conductr-dashboard-core` (#149), generated with `tsify` so
TS outlets get the same definition without hand-rolling it.

Where a type already exists in `crates/conductr-core/src/types.rs`,
it is **reused as-is** rather than parallel-defined. Those reuses
are flagged with `(reused from conductr-core)`.

### 4.0 Wire conventions for enums

Rust enums on the wire use the **stable short identifier**, not the
mnemonic Rust variant name. The mnemonic may shift as the codebase
evolves; the wire value will not. `MaturityLevel` is the canonical
example:

| Rust variant      | Wire value |
|-------------------|------------|
| `L0Bootstrap`     | `"L0"`     |
| `L1Tested`        | `"L1"`     |
| `L2GitFlow`       | `"L2"`     |
| `L3Architected`   | `"L3"`     |
| `L4Skilled`       | `"L4"`     |
| `L5Orchestrated`  | `"L5"`     |

This is enforced via `#[serde(rename = "...")]`. New enums added to
the contract follow the same pattern: pick a short stable
identifier on the wire side, keep the human-readable name in Rust.
Tagged enums (e.g. `Health`, `MailKind`) use `#[serde(tag = "kind")]`
with the same stable-identifier rule for the `kind` value.

### 4.1 Repo / project registry

`RepoEntry` — one per active project in `~/.conductr`.

```ts
type RepoEntry = {
  slug: RepoSlug;            // owner/repo, reused from conductr-core
  tag: string;               // project_tag from per-repo .conductr
  local_path: string;        // absolute path to working tree
  status: "active" | "pending" | "archived";
  cadence: Record<string, string>;  // section -> cron expression
  maturity: MaturityLevel | null;   // last computed, reused from conductr-core
}
```

### 4.2 Orchestrate cycles

`Cycle` — one per in-flight or recently-finished orchestrate pass.

```ts
type Cycle = {
  repo: RepoSlug;
  started_at: string;        // RFC3339
  finished_at: string | null;
  trigger: "cron" | "manual" | "webhook";
  state: "running" | "succeeded" | "failed" | "cancelled";
  beats: Beat[];             // sub-steps the cycle progressed through
  pr_numbers: number[];      // PRs the cycle touched
}

type Beat = {
  name: string;              // "architect", "implementer", "reviewer", ...
  state: "queued" | "running" | "done" | "skipped" | "failed";
  started_at: string | null;
  finished_at: string | null;
  pr_number: number | null;
}
```

### 4.3 Pull requests

Reuses `Pr` (and `ClosedPr` for historic) from
`conductr-core/src/types.rs`. Added wire-only:

```ts
type PrGrouped = {
  repo: RepoSlug;
  mergeable_green: Pr[];     // ready to merge
  mergeable_red:   Pr[];     // CI failing but no conflicts
  conflicting:     Pr[];
  draft:           Pr[];
}
```

`Pr.ci` is the existing `CiStatus` enum (`Pending`, `Success`,
`Failure`, `Unknown`).

### 4.4 Idle findings

`Finding` and `FindingSeverity` from `crates/conductr/src/idle.rs`
move to `conductr-dashboard-core::model` (or
`conductr-core::types`) and pick up `Serialize` / `Deserialize`.
That's a small refactor; tracked as a sub-task of #149.

```ts
type Finding = {
  title: string;
  body: string;
  severity: "Architecture" | "Quality" | "Coverage";
  fingerprint: string;       // deterministic, used for dedup
  issue_number: number | null;  // null if not yet filed
  repo: RepoSlug;
  first_seen: string;        // RFC3339
}
```

### 4.5 Pod / tmux

Reuses `TmuxSession`, `Health`, and `Diagnosis` from
`conductr-core/src/types.rs` as-is.

```ts
type PodSnapshot = {
  sessions: Diagnosis[];     // each carries TmuxSession + Health + tail
}
```

### 4.6 Cadence staff

The staff render is the same payload the CLI's `conductr cadence
show` consumes (parity rule, §11). The data model is the time-pattern
shape from `crates/conductr-schedule`; outlets render to ASCII, SVG,
or native canvas as they like.

```ts
type CadenceStaff = {
  repo: RepoSlug;
  window: { from: string; to: string };  // RFC3339, exclusive-end
  rows: StaffRow[];
}

type StaffRow = {
  label: string;             // "orchestrate", "idle", "cadence", ...
  hits: StaffHit[];
}

type StaffHit = {
  at: string;                // RFC3339
  duration_seconds: number | null;  // null = percussion hit
  glyph: "head" | "rest" | "hit" | "tied";
}
```

Glyph semantics are part of the protocol — outlets exhaustively
switch on the four values and any new glyph requires a protocol
major bump. #72 settles the cadence grammar within this set.

| glyph  | shape                    | `duration_seconds` |
|--------|--------------------------|--------------------|
| `head` | sustained note          | required (> 0)     |
| `rest` | explicit silence        | required (> 0)     |
| `hit`  | percussion / instant    | `null`             |
| `tied` | continuation of a `head` across the window edge | required (> 0) |

A producer that needs a glyph not in this set must emit the closest
fit and surface the detail in the row `label`, not invent a value.

### 4.7 Cron schedule (machine-wide)

```ts
type CronEntry = {
  expression: string;        // "*/30 * * * *"
  command: string;           // "conductr orchestrate ..."
  marker: string;            // "# conductr-cron: orchestrate luan-vp/foo"
  next_fire: string;         // RFC3339
}
```

### 4.8 Local-agent health

```ts
type LocalAgent = {
  kind: "ollama" | "llamacpp" | "pi";
  endpoint: string;          // "http://127.0.0.1:11434"
  reachable: boolean;
  latency_ms: number | null;
  models: string[];          // empty if unreachable
  last_checked: string;      // RFC3339
}
```

### 4.9 Build / CI

Reuses `CiRunRow` from `conductr-core/src/types.rs`. The dashboard
surfaces aggregate CI status per repo:

```ts
type CiSnapshot = {
  repo: RepoSlug;
  recent_runs: CiRunRow[];   // last N, newest first
  current_status: "green" | "red" | "amber" | "unknown";
}
```

## 5. Endpoints

| Method | Path                                  | Purpose                                |
|--------|---------------------------------------|----------------------------------------|
| GET    | `/version`                            | Protocol + impl version (no envelope)  |
| GET    | `/state`                              | Full snapshot — every section in one   |
| GET    | `/repos`                              | List `RepoEntry[]`                     |
| GET    | `/repos/{slug}/prs`                   | `PrGrouped` for one repo               |
| GET    | `/repos/{slug}/cycle`                 | Current or most-recent `Cycle`         |
| GET    | `/repos/{slug}/cycles`                | Recent cycles (paginated)              |
| GET    | `/repos/{slug}/findings`              | `Finding[]` for one repo               |
| GET    | `/repos/{slug}/cadence`               | `CadenceStaff` for one repo            |
| GET    | `/repos/{slug}/ci`                    | `CiSnapshot` for one repo              |
| GET    | `/findings`                           | `Finding[]` across all repos           |
| GET    | `/pod`                                | `PodSnapshot`                          |
| GET    | `/cron`                               | `CronEntry[]`                          |
| GET    | `/local-agents`                       | `LocalAgent[]`                         |
| GET    | `/events`                             | SSE push channel (§6)                  |

`{slug}` is URL-encoded `owner/repo`.

`/state` is the cold-start endpoint; outlets call it once on
connect, then subscribe to `/events` and refetch individual sections
when events indicate a change. Outlets that don't care about a
section omit the corresponding event subscription.

## 6. Push channel (SSE)

`/events` emits a stream of typed events:

```
event: pr.changed
data: {"repo": "luan-vp/conductr", "number": 142}

event: pod.session_changed
data: {"session": "conductr-orchestrate-1"}

event: daemon.stale
data: {"source": "gh", "reason": "rate_limit", "retry_at": "..."}
```

### Event catalogue

| Event                    | Payload                                              |
|--------------------------|------------------------------------------------------|
| `pr.opened`              | `{repo, number}`                                     |
| `pr.changed`             | `{repo, number}`  (status, CI, head_ref, etc.)       |
| `pr.closed`              | `{repo, number}`                                     |
| `pr.merged`              | `{repo, number}`                                     |
| `cycle.started`          | `{repo, trigger}`                                    |
| `cycle.finished`         | `{repo, state}`                                      |
| `pod.session_changed`    | `{session}`                                          |
| `pod.session_crashed`    | `{session}`                                          |
| `finding.new`            | `{repo, fingerprint, severity}`                      |
| `finding.resolved`       | `{repo, fingerprint}`                                |
| `cadence.tick`           | `{repo}`  (advisory — current-time marker advanced)  |
| `local_agent.changed`    | `{kind, reachable}`                                  |
| `daemon.stale`           | `{source, reason, retry_at}`                         |
| `daemon.unstale`         | `{source}`                                           |

Events are **advisory** — they tell the outlet *something changed
in section X*. The outlet decides whether to refetch the affected
endpoint or animate from cached state. Daemon does not push diffs.

Reconnect: outlets resubscribe with `Last-Event-ID` if they want
catch-up; otherwise they refetch `/state` and start clean.

## 7. Command surface (deferred to v2)

v1 is read-only. Commands land with daemon v2 (#148) and outlet v2.
Sketching the surface here so v1 outlets can stub gracefully:

| Method | Path                                  | Purpose                                |
|--------|---------------------------------------|----------------------------------------|
| POST   | `/repos/{slug}/orchestrate`           | Trigger an orchestrate cycle           |
| POST   | `/repos/{slug}/idle`                  | Trigger an idle pass                   |
| POST   | `/repos/{slug}/prs/{n}/merge`         | Merge a PR (with policy options)       |
| POST   | `/repos/{slug}/prs/{n}/comment`       | Post a comment                         |
| POST   | `/pod/{session}/heal`                 | Heal a tmux session                    |
| PUT    | `/repos/{slug}/config`                | Patch the per-repo `.conductr`         |
| PUT    | `/config`                             | Patch `~/.conductr`                    |

The SAFETY-fader writeback (#175) maps to `PUT
/repos/{slug}/config` once command surface lands; until then the
fader is read-only.

## 8. Error model

Non-2xx responses carry a structured body:

```json
{
  "error": {
    "code":      "STALE_CACHE",
    "message":   "GitHub API rate-limited; serving cached data",
    "retryable": true,
    "context":   { "source": "gh", "retry_at": "2026-05-27T13:50:00Z" },
    "stale_at":  "2026-05-27T13:40:12Z"
  }
}
```

Standard codes:

| Code                 | HTTP | Meaning                                        |
|----------------------|------|------------------------------------------------|
| `STALE_CACHE`        | 200  | Section served from cache; refresh failed      |
| `SOURCE_UNAVAILABLE` | 200  | A source (tmux/gh/...) is down                 |
| `NOT_FOUND`          | 404  | Resource not found                             |
| `INVALID_QUERY`      | 400  | Malformed request                              |
| `PROTOCOL_MISMATCH`  | 426  | Outlet's `Accept-Protocol` not supported       |
| `UNAUTHORIZED`       | 401  | v2 only                                        |
| `INTERNAL`           | 500  | Daemon bug                                     |

`STALE_CACHE` and `SOURCE_UNAVAILABLE` return 200 because the
daemon still served a usable (if degraded) snapshot — the outlet
renders the data, surfaces the `stale_at` honestly, and trusts the
daemon will recover.

## 9. Fixtures

Canonical example payloads live under `docs/dashboard-api/fixtures/`:

- `state.json` — full envelope from `GET /state`
- `cycle-in-flight.json` — `Cycle` mid-run
- `prs-by-repo.json` — `PrGrouped` for one repo
- `pod-health.json` — `PodSnapshot`
- `idle-findings.json` — `Finding[]`
- `cadence-render.json` — `CadenceStaff`

Outlets use these as test fixtures. The daemon uses them as
round-trip targets. Adding a new section to the model means adding
a fixture; CI enforces fixture coverage of every public type.

## 10. ADR cross-links

- Hexagonal style: `docs/hex-refactor-plan.md` — the daemon is the
  only producer of state; outlets are pure consumers; the contract
  is the seam.
- CLI parity: `docs/cli-skill-parity.md` — every dashboard section
  has a corresponding `conductr` subcommand. The dashboard does not
  expose state the CLI cannot.
- Cadence vocabulary: `docs/cadence.md` — staff render grammar
  lives there; this contract just carries the shape.

## 11. Parity rule

Every section of this contract corresponds to a `conductr` CLI
subcommand:

| Contract section | CLI                                    |
|------------------|----------------------------------------|
| `/repos`         | (registry — read `~/.conductr`)        |
| `/repos/.../prs` | `gh pr list` via adapter               |
| `/repos/.../cycle` | `conductr orchestrate status`        |
| `/repos/.../findings` | `conductr idle status`            |
| `/repos/.../cadence`  | `conductr cadence show`           |
| `/repos/.../ci`  | `gh run list` via adapter              |
| `/pod`           | `conductr pod diagnose`                |
| `/cron`          | `conductr cadence status`              |
| `/local-agents`  | `conductr local detect`                |

The dashboard adds no state the CLI cannot read. This is a
hard rule: if the dashboard renders something, you can also
get it from the terminal.

## 12. Adding a new outlet

Zero changes to the daemon or core crate. Steps:

1. Pull the `@conductr/dashboard-core` npm package (or the Rust
   crate, for ratatui).
2. Open the socket / TCP endpoint.
3. Call `/state` once for the cold snapshot.
4. Subscribe to `/events`.
5. On each event, refetch the affected section.

If you find yourself wanting daemon-side support, file a ticket
against #146 first; the contract changes, then the daemon, then
the outlet — never in reverse.

## 13. Adding a new data section

1. Add the type to `crates/conductr-dashboard-core::model`.
2. Add the fixture under `docs/dashboard-api/fixtures/`.
3. Add the endpoint to §5 (and §6 events if relevant).
4. Implement the daemon-side aggregator.
5. Outlets pick it up incrementally; nothing breaks if an outlet
   ignores a new section.

## 14. Open questions

- **Cycle history retention.** How many cycles does `/cycles` return
  by default? Pagination shape? — Defer to daemon impl (#147).
- **Multi-repo envelopes.** `/state` could either nest by repo
  or return parallel arrays. Strawman: parallel arrays; outlets
  index by `RepoSlug`. Revisit if outlet code ends up duplicating
  the indexing.
- **Federation envelope shape.** `/state` in a multi-host world —
  TBD by #152; v1 contract assumes single host.
- **Compression.** SSE + repeated `/state` calls on connect can be
  chatty. Defer to daemon impl; if it bites, gzip is sufficient.

Edits to this contract land via PR with the change-summary at the
top of the diff. Outlets that consume the wire format track this
file in their CI.
