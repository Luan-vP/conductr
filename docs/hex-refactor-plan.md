# Hexagonal Refactor — Plan

## Why

`conductr` already has two surfaces driving the same flows:

- The CLI binary (`conductr pod diagnose`, `conductr pod save-state`, …).
- The Claude skills (`skills/pod/SKILL.md`, future skill ports of
  `tasks` and `orchestrate`).

And the use cases share connectors that don't yet share an interface:

- `IssueTracker`-shaped: beads, Notion, GitHub Issues, Linear (future).
- `ScmHost`-shaped: `gh` CLI today, GitHub REST tomorrow.
- `TmuxAgent`-shaped: local tmux today, `ssh + tmux` tomorrow.
- `InstanceProvider`-shaped: stub today, AWS / Hetzner / DO tomorrow.

Today each crate mixes its own connector with its own use-case logic
(`conductr-tasks` ships beads + Notion + the sync flow in one crate;
`conductr-pod` ships the tmux wrapper alongside the classifier). That makes
"plug Notion into save-state" or "test the orchestrator against a fake
GitHub" a bigger change than it needs to be.

We're already half-hex in `conductr-orchestrate` (the orchestrator is generic
over a `GitHub` trait). Push that pattern through every crate.

## Target architecture

```
                            ┌───────────────────────────┐
  driving adapters          │  conductr (binary, CLI)   │
  (skills are not in        │  skills/* (markdown)      │
   the workspace, they      └────────────┬──────────────┘
   shell out to the CLI)                 │
                                         ▼
                            ┌───────────────────────────┐
  use-case crates           │  conductr-orchestrate     │
  (pure flows, depend       │  conductr-pod             │
   only on core)            │  conductr-tasks           │
                            │  conductr-instance        │
                            │  conductr-schedule (pure) │
                            └────────────┬──────────────┘
                                         │ trait calls
                                         ▼
                            ┌───────────────────────────┐
  ports (traits) + types    │  conductr-core            │
                            │   ::types  (Task, Issue,  │
                            │            Diagnosis…)    │
                            │   ::ports  (IssueTracker, │
                            │            ScmHost,       │
                            │            TmuxAgent,     │
                            │            InstanceProvider)│
                            └────────────┬──────────────┘
                                         │ implemented by
                                         ▼
                            ┌───────────────────────────┐
  driven adapters           │  conductr-adapters        │
                            │   feature: tmux           │
                            │   feature: beads          │
                            │   feature: notion         │
                            │   feature: gh-cli         │
                            │   feature: mock           │
                            └───────────────────────────┘
```

### Rules

1. **Use-case crates may not depend on `conductr-adapters` or any specific
   connector crate.** They depend on `conductr-core` (types + ports) only.
2. **The binary is the only place where adapters are constructed and wired
   into use cases.** Selection happens via flags / env / config.
3. **Adapters never depend on use-case crates.** They speak the port traits
   defined in core.
4. **`conductr-core` has no I/O.** No `tokio::process`, no `reqwest`, no
   filesystem reads beyond what `serde_json` already does on `&str`.
5. **Mock adapters live next to real ones**, behind a `mock` feature, and
   are used by use-case unit tests. No bespoke mocks per use-case crate.
6. **One trait per port.** Adding a new connector adds an adapter, not a new
   port shape, until the port can no longer represent the new connector.

### What stays the same

- `conductr-schedule` is already pure — no work beyond moving its types
  into `conductr-core` (or leaving it; this is a judgement call captured
  in T1).
- `conductr-orchestrate`'s `GitHub` trait is the model the other ports
  should follow.
- The CLI surface (`conductr <subcommand> …`) does not change in this
  refactor. The skill surface does not change.

## Migration steps

Foundation, then adapters, then use-case rewires, then the binary, then
docs. Eight tickets in the main hex track, plus three independent tickets
(T9–T11) that share the foundation but otherwise run on their own.

```
            T1 (core: types + ports)
                    │
        ┌───────────┴───────────────────────────────┐
        ▼                                           │
        T2 (adapters: tmux, beads, notion,          │
            gh-cli, mock)                           │
        │                                           │
   ┌────┴────┬───────────┬───────────┐              │
   ▼         ▼           ▼           ▼              │
   T3 pod   T4 tasks    T5 orch    T6 instance      │
   │         │           │           │              │
   └─────────┴─────┬─────┴───────────┘              │
                   ▼                                │
              T7 (binary: wire it up,               │
                  save-state --tracker)             │
                   │                                │
                   ▼                                │
              T8 (base.md + README)                 │
                                                    │
  Independent track (each depends only on T1):      │
                                                    │
                          T9   agent mail ──────────┤
                          T10  maturity wizard ─────┤
                          T11  plan → sheet music ──┘
```

## Tickets

Each ticket below is intended to be self-contained for an implementing
agent (Sonnet / Haiku via the GitHub Action bot). They reference each other
by `T<n>`; on GitHub they will be filed with `depends on #<issue>` markers.

Common acceptance gates for every ticket:

- `cargo build --workspace` passes.
- `cargo test --workspace` passes.
- `cargo clippy --workspace --all-targets -- -D warnings` passes (or, if
  clippy isn't currently green on `main`, no new warnings are introduced).
- Public CLI surface and skill surface are unchanged unless the ticket
  explicitly says otherwise.

Out-of-scope for every ticket unless explicitly named:

- Changing the schedule pattern parser or DSL.
- Touching `vendor/`.
- Adding new connectors beyond what's listed (Notion is in scope; Linear
  and Jira are not).
- Changing CI or `.github/` (no CI exists yet).

---

### T1 — Create `conductr-core` with shared types + port traits

**Why:** Use-case crates currently embed their own copies of the domain
types and call concrete adapters directly. Centralising the types and the
trait surface is the precondition for every following ticket.

**Files to create:**
- `crates/conductr-core/Cargo.toml` — minimal deps: `serde`, `serde_json`,
  `thiserror`, `async-trait`, `chrono`. **No** `tokio`, `reqwest`,
  `tokio::process`. (Async traits use `async-trait`; the runtime is the
  caller's concern.)
- `crates/conductr-core/src/lib.rs` — declares `pub mod types;` and
  `pub mod ports;`.
- `crates/conductr-core/src/types.rs` — moves the following definitions
  from their current crates *verbatim*:
  - From `conductr-tasks::lib`: `Task`, `TaskStatus`.
  - From `conductr-pod::diagnose`: `Health`, `Diagnosis`. From
    `conductr-pod::tmux`: `TmuxSession`.
  - From `conductr-orchestrate::types`: `RepoSlug`, `Issue`, `IssueState`,
    `Pr`, `PrState`, `CiStatus`. From `conductr-orchestrate::classifier`:
    `Bucket`, `Classification`. From `conductr-orchestrate::graph`:
    `DepGraph` (and any helpers it needs). From
    `conductr-orchestrate::orchestrator`: `OrchestratorConfig`,
    `CycleReport`.
  - From `conductr-instance::lib`: `InstanceSpec`, `Provider`,
    `InstanceHandle`, `InstanceError`.
- `crates/conductr-core/src/ports.rs` — defines four async traits, each
  matching the *current* method shape of the existing adapter so the
  refactor doesn't change behaviour:
  - `IssueTracker` — modelled on the methods `Beads` exposes today
    (`list`, `list_ready`, `create_full`, `close`) plus a write-side
    `upsert` that Notion needs (`upsert_task`, currently on
    `conductr-tasks::notion::Notion`). Errors return a shared
    `IssueTrackerError` enum.
  - `ScmHost` — modelled on the trait already present in
    `conductr-orchestrate::github` (the trait `GhCli` implements). Move
    the trait body wholesale; rename the trait to `ScmHost`.
  - `TmuxAgent` — modelled on the public methods of `conductr-pod::Tmux`
    (`list_sessions`, `capture_pane`, `send_line`, `send_key`). Errors
    return a shared `TmuxError`.
  - `InstanceProvider` — modelled on
    `conductr-instance::InstanceManager` (`spin_up`, `connect`, `run`,
    `tear_down`). Move the trait wholesale; rename to
    `InstanceProvider`.

**Updates to existing crates:**
- Add `conductr-core = { workspace = true }` and `pub use
  conductr_core::types::*;` in each affected crate so existing import
  paths (`conductr_tasks::Task`, `conductr_pod::Diagnosis`, …) keep
  resolving. The next tickets will narrow these re-exports.
- Workspace `Cargo.toml`: add `crates/conductr-core` to `members`, declare
  the workspace-level dep `conductr-core = { path = "crates/conductr-core",
  version = "0.1.0" }`.

**Acceptance:**
- All existing tests still compile and pass without code changes outside
  the crates touched here.
- `conductr-core` has no `tokio` / `reqwest` / `tokio::process` /
  `std::fs` / `std::process` dependencies (verify via `cargo tree -p
  conductr-core`).
- Each port trait is `Send + Sync` and uses `#[async_trait]` where it has
  async methods.

**Out of scope:**
- Moving any adapter code. The `Beads`, `Notion`, `Tmux`, `GhCli`,
  `StubManager` structs stay where they are in this ticket; they will be
  relocated in T2.
- Changing public API beyond adding the new core crate.

---

### T2 — Create `conductr-adapters` with feature-gated adapter modules

**Depends on:** T1.

**Why:** Pull every concrete connector out of the use-case crates and into
one place. Feature-gated so a downstream consumer that only wants `tmux`
doesn't pay for `reqwest`.

**Files to create:**
- `crates/conductr-adapters/Cargo.toml`:
  ```toml
  [features]
  default = []
  tmux    = ["tokio"]
  beads   = ["tokio"]
  notion  = ["tokio", "reqwest", "url"]
  gh-cli  = ["tokio"]
  mock    = []
  full    = ["tmux", "beads", "notion", "gh-cli", "mock"]
  ```
  Dependencies follow the existing crates' choices (e.g. `reqwest` with
  `rustls-tls`).
- `crates/conductr-adapters/src/lib.rs` declares each module behind its
  feature:
  ```rust
  #[cfg(feature = "tmux")]   pub mod tmux;
  #[cfg(feature = "beads")]  pub mod beads;
  #[cfg(feature = "notion")] pub mod notion;
  #[cfg(feature = "gh-cli")] pub mod gh_cli;
  #[cfg(feature = "mock")]   pub mod mock;
  ```
- `src/tmux.rs` — moves `conductr-pod::tmux::Tmux` here. Implements
  `TmuxAgent` (the existing methods become trait method bodies; keep
  inherent impls if convenient, but the trait must be implemented).
- `src/beads.rs` — moves `conductr-tasks::beads::Beads`. Implements
  `IssueTracker`. Maps `BeadsError` → `IssueTrackerError`.
- `src/notion.rs` — moves `conductr-tasks::notion::Notion`. Implements
  `IssueTracker` (read methods may return
  `IssueTrackerError::Unsupported` for now; only the upsert path is
  required to work). Keep `from_env()` as a constructor.
- `src/gh_cli.rs` — moves `conductr-orchestrate::github::GhCli`.
  Implements `ScmHost`.
- `src/mock.rs` — in-memory test doubles:
  - `MockIssueTracker { tasks: RwLock<HashMap<String, Task>> }` with
    deterministic id generation.
  - `MockScmHost { issues, prs, comments, … }` with builder helpers.
  - `MockTmuxAgent { sessions: HashMap<String, MockSession> }` where
    `MockSession` carries a configurable pane buffer, sent-key log, and
    metadata for `list_sessions`.
  - `MockInstanceProvider`.
  Each mock struct implements its corresponding port trait. Mocks expose
  inspection helpers (e.g. `MockTmuxAgent::sent_lines(&self,
  session: &str) -> Vec<String>`) for assertions.
- `crates/conductr-adapters/tests/` — one integration test per adapter
  using its mock counterpart where applicable, mirroring the existing
  unit tests in the source crates.

**Updates to existing crates:**
- `conductr-pod`, `conductr-tasks`, `conductr-orchestrate`,
  `conductr-instance` keep their `Cargo.toml` deps for now — they still
  re-export the moved types via `conductr-core` (T1) and call the moved
  adapters via *deprecated* re-exports until T3-T6 land. To keep this
  ticket atomic, leave the old type aliases in place: e.g.
  `conductr-pod` adds `pub use conductr_adapters::tmux::Tmux;` so
  `conductr_pod::Tmux` still resolves.

**Acceptance:**
- `cargo build --workspace --features full` builds.
- `cargo build --workspace` (no features) still builds — adapters compile
  to empty modules.
- `cargo test -p conductr-adapters --features full` runs all adapter
  tests.
- The existing tests in `conductr-pod`, `conductr-tasks`,
  `conductr-orchestrate` still pass, exercising the re-exports.

**Out of scope:**
- Removing `Beads`/`Notion`/`Tmux`/`GhCli` from their original crates —
  the re-export shim stays until the corresponding use-case ticket
  removes it.
- Refactoring the use-case logic.

---

### T3 — Refactor `conductr-pod` to use ports only

**Depends on:** T1, T2.

**Why:** Today `diagnose_all` / `heal_all` take a concrete `&Tmux`. Make
them take `&dyn TmuxAgent`. This unblocks unit tests against
`MockTmuxAgent` and lets a future remote adapter slot in.

**Files to change:**
- `crates/conductr-pod/src/diagnose.rs`:
  - Change `pub async fn diagnose_all(tmux: &Tmux, ...)` to
    `pub async fn diagnose_all(tmux: &impl TmuxAgent, ...)` (or `&dyn
    TmuxAgent` if dyn dispatch is preferred for binary size). Same for
    `diagnose_one` and the private `diagnose_session`.
- `crates/conductr-pod/src/heal.rs`:
  - Same change for `heal_all`. `plan_for` is already pure (no I/O), no
    change needed.
- `crates/conductr-pod/src/lib.rs`:
  - Remove `pub mod tmux;` and the `pub use tmux::*;` re-export.
  - Remove the deprecated `pub use conductr_adapters::tmux::Tmux;`
    re-export added in T2 (callers should import from
    `conductr_adapters` directly).
- `crates/conductr-pod/src/tmux.rs` — delete (moved to adapters in T2).
- `crates/conductr-pod/Cargo.toml`:
  - Remove `tokio`, `tracing` if they were only used by the deleted
    `tmux.rs`.
  - Add `conductr-core = { workspace = true }`.
  - Add `[dev-dependencies] conductr-adapters = { workspace = true,
    features = ["mock"] }`.
- `crates/conductr-pod/src/diagnose.rs` (tests):
  - Add `#[tokio::test]` cases that build a `MockTmuxAgent` with a known
    pane buffer and assert the resulting `Diagnosis`.
- `crates/conductr-pod/src/heal.rs` (tests):
  - Add `#[tokio::test]` covering: idle session is skipped, crashed
    session triggers `send_line`, `--dry-run` does not call `send_line`.
    Use `MockTmuxAgent::sent_lines` for assertions.

**Updates to binary:**
- `crates/conductr/src/main.rs` — change `let tmux = Tmux::new();` to
  `let tmux = conductr_adapters::tmux::Tmux::new();` (already a
  `TmuxAgent`). Pass `&tmux` as before; the call sites remain unchanged.

**Acceptance:**
- `crates/conductr-pod` does not depend on `tokio` or any process/network
  crate. (`cargo tree -p conductr-pod` should show only `conductr-core`
  and lightweight deps like `serde`, `chrono`, `async-trait`.)
- Diagnose/heal continue to work end-to-end via the binary against real
  tmux.
- New mock-based tests cover the diagnose flow without needing a real
  tmux server.

**Out of scope:**
- save-state — that's binary-side, covered by T7.

---

### T4 — Refactor `conductr-tasks` to use ports only

**Depends on:** T1, T2.

**Why:** Right now `conductr-tasks` couples beads + Notion + the sync
flow. Split it: the crate becomes "task use-cases over an `IssueTracker`
port", and beads/Notion live in `conductr-adapters`.

**Files to change:**
- `crates/conductr-tasks/src/beads.rs` — delete (moved in T2).
- `crates/conductr-tasks/src/notion.rs` — delete (moved in T2).
- `crates/conductr-tasks/src/lib.rs`:
  - Remove `pub mod beads;` / `pub mod notion;`.
  - Remove the `Task` / `TaskStatus` definitions (they live in
    `conductr-core::types` now); re-export from core.
  - Keep `pub mod sync;` but rewrite it: see below.
- `crates/conductr-tasks/src/sync.rs`:
  - Replace `beads_to_notion(&Beads, &Notion, &str)` with
    `pub async fn sync_tasks(src: &impl IssueTracker, dst: &impl
    IssueTracker, dst_database: Option<&str>) -> Result<SyncReport,
    IssueTrackerError>`. The `dst_database` param is Notion-specific and
    optional; pass-through is fine — the Notion adapter knows what to do
    with it via `IssueTracker::context` or similar.
  - Add a use-case `pub async fn list_tasks(src: &impl IssueTracker,
    ready_only: bool)`.
  - Add `pub async fn create_task(src: &impl IssueTracker, title: &str,
    priority: Option<u8>, body: Option<&str>, labels: &[&str])`.
  - Move `SyncReport` to `conductr-core::types` (it's a pure type).
- `crates/conductr-tasks/Cargo.toml`:
  - Remove `tokio`, `reqwest`, `chrono` dependencies — they leave with
    the adapter code.
  - Add `conductr-core = { workspace = true }`.
  - Add `[dev-dependencies] conductr-adapters = { workspace = true,
    features = ["mock"] }`.
- `crates/conductr-tasks/tests/` — at least one test per use-case
  (`list_tasks`, `create_task`, `sync_tasks`) using
  `MockIssueTracker`.

**Updates to binary:**
- `crates/conductr/src/main.rs` `run_tasks`:
  - Construct concrete adapters: `let beads =
    conductr_adapters::beads::Beads::new();` and (for sync)
    `conductr_adapters::notion::Notion::from_env()?`.
  - Replace direct method calls with use-case calls
    (`conductr_tasks::sync::list_tasks(&beads, ready)`, etc.).

**Acceptance:**
- `conductr-tasks` does not depend on `tokio`, `reqwest`, or any I/O
  crate. (`cargo tree`.)
- `conductr tasks list/--ready/create/sync-to-notion` continues to work
  end-to-end. Note: testing `sync-to-notion` end-to-end requires a real
  Notion key — limit the acceptance check to the dry-path / a mock
  destination, plus a smoke test against a personal Notion if available.
- New mock-based unit tests cover all three use cases.

**Out of scope:**
- Changing the wire format of beads or Notion responses.
- save-state's tracker selection (T7).

---

### T5 — Refactor `conductr-orchestrate` to use ports only

**Depends on:** T1, T2.

**Why:** The orchestrator is already trait-driven (`Orchestrator<C:
GitHub>`). The change is moving the trait and the concrete impl, not
restructuring the loop.

**Files to change:**
- `crates/conductr-orchestrate/src/github.rs` — delete the `GhCli` impl
  (moved to `conductr-adapters::gh_cli` in T2). Keep the `GitHub` trait
  *only* if the rename to `ScmHost` in core didn't cover it; otherwise
  delete and import `conductr_core::ports::ScmHost`.
- `crates/conductr-orchestrate/src/orchestrator.rs`:
  - Change generic bound: `Orchestrator<C: GitHub>` →
    `Orchestrator<C: ScmHost>`. Keep the rest of the loop intact.
- `crates/conductr-orchestrate/src/types.rs` — remove the types moved to
  `conductr-core::types`; re-export them.
- `crates/conductr-orchestrate/src/classifier.rs` and `graph.rs` — same
  treatment for any moved types.
- `crates/conductr-orchestrate/src/lib.rs` — re-exports updated; remove
  the `pub use github::GhCli;` re-export (binary will import from
  adapters directly).
- `crates/conductr-orchestrate/Cargo.toml`:
  - Remove `tokio`, `regex`, `once_cell` if only the moved adapter used
    them.
  - Add `conductr-core = { workspace = true }`.
  - Keep `[dev-dependencies] conductr-adapters = { workspace = true,
    features = ["mock"] }`.

**Updates to binary:**
- `crates/conductr/src/main.rs` — `let orch = Orchestrator::new(GhCli,
  cfg);` becomes `Orchestrator::new(conductr_adapters::gh_cli::GhCli,
  cfg)`.

**Acceptance:**
- `conductr-orchestrate` does not depend on `tokio` (or any subprocess /
  HTTP crate). `async-trait` is fine.
- `conductr orchestrate --repo owner/repo --once --dry-run` continues to
  print the same JSON.
- Existing `conductr-orchestrate` tests pass; if any of them used
  `GhCli`, they now use `MockScmHost` from adapters.

**Out of scope:**
- Adding a GitHub REST adapter (separate ticket later if wanted).
- Changing the issue classification rules.

---

### T6 — Refactor `conductr-instance` to use ports only

**Depends on:** T1, T2.

**Why:** Mostly a relabel — `InstanceManager` → `InstanceProvider` in
core, `StubManager` moves to `conductr-adapters::mock`.

**Files to change:**
- `crates/conductr-instance/src/lib.rs`:
  - Remove the `InstanceManager` trait and `StubManager` impl (moved in
    T1 / T2).
  - Re-export `InstanceProvider` from core.
  - Add a placeholder `pub async fn spin_up(provider: &impl
    InstanceProvider, spec: &InstanceSpec) -> ...` use-case if it makes
    the binary cleaner; otherwise leave the crate empty pending future
    provider implementations.
- `crates/conductr-instance/Cargo.toml`:
  - Keep deps minimal; add `conductr-core = { workspace = true }`.

**Updates to binary:**
- `crates/conductr/src/main.rs` `run_instance`:
  - Use `conductr_adapters::mock::MockInstanceProvider` for the `List`
    subcommand (still returns `[]`). For `SpinUp`, keep the existing
    `bail!` until provider implementations land.

**Acceptance:**
- Workspace builds and tests pass.
- `conductr instance list` still prints `[]`.
- `conductr instance spin-up --name foo` still bails with the existing
  message.

**Out of scope:**
- Wiring real cloud providers.

---

### T7 — Wire adapters into the binary; add `save-state --tracker`

**Depends on:** T3, T4, T5, T6.

**Why:** With every use-case crate on ports, the binary becomes the only
place that mentions concrete adapters. Codify that. Then add the
flexibility the original motivation required: `save-state` should be
able to write recovery issues to either beads or Notion.

**Files to change:**
- `crates/conductr/Cargo.toml`:
  - Drop `conductr-tasks::beads::Beads` direct paths; instead depend on
    `conductr-adapters = { workspace = true, features = ["full"] }`.
- `crates/conductr/src/main.rs`:
  - Audit every adapter construction. They should look like
    `conductr_adapters::beads::Beads::new()`, etc. No re-exports through
    the use-case crates.
  - Add a small `wiring` module (`crates/conductr/src/wiring.rs`) with
    helpers such as `fn issue_tracker(kind: TrackerKind) -> Box<dyn
    IssueTracker>`. Defaults: `beads`. Alternatives: `notion`. The
    helper handles `Notion::from_env()` failure cleanly.
  - Add a `--tracker beads|notion` flag to `conductr pod save-state`.
    Default `beads`. When `notion`, use the wiring helper. The Notion
    adapter must be passed the database id via a new `--notion-database
    <id>` flag (or `CONDUCTR_NOTION_DATABASE` env var).
  - The save-state JSON manifest's `beads_id` field is renamed to
    `tracker_id` and gains a sibling `tracker: "beads" | "notion"` so
    skill-side code can route correctly.

**Updates to skill:**
- `skills/pod/SKILL.md`: mention the new `--tracker` flag and
  the renamed manifest fields. The "Notion update" section is now an
  *alternative* path — when `--tracker notion` is used, the skill
  doesn't need to mirror, the binary already wrote it. When `--tracker
  beads` (default), the skill mirrors as before.

**Acceptance:**
- `conductr pod save-state --tracker beads` behaves exactly as today (modulo
  field renames documented in the skill).
- `conductr pod save-state --tracker notion --notion-database <id>` writes
  recovery issues into the named Notion database. Smoke-test against a
  personal database if available; otherwise document the env-var setup
  and ship.
- The binary contains zero direct mentions of `tokio::process::Command`,
  `reqwest::Client`, or `tmux` strings — those only appear inside
  `conductr-adapters`.

**Out of scope:**
- Adding new tracker kinds beyond beads + notion.

---

### T8 — Architecture base + README update

**Depends on:** T7.

**Why:** Codify the rules in `.claude/base.md` so the architect agent
recognises them on future passes. Update the README so contributors land
in the right mental model.

**Files to create / change:**
- `.claude/base.md` — short canonical description of the hex layout. Use
  the diagram + rules at the top of this plan as the source. Mention the
  six rules explicitly. Reference the use-case crates as "arms" and the
  adapter modules as "folds" if you want to lean into the origami
  vocabulary the orchestrate skill uses; otherwise plain language is
  fine.
- `README.md`:
  - Update the crate table to include `conductr-core`,
    `conductr-adapters`.
  - Update the project layout tree.
  - Add a short "Architecture" section linking to `.claude/base.md`.

**Acceptance:**
- A new contributor reading the README can identify which crate to touch
  for: "I want to add a Linear connector" (→ adapters); "I want to
  change how save-state classifies sessions" (→ use-case crate); "I
  want to rename a domain field" (→ core).
- `.claude/base.md` is < 200 lines.

**Out of scope:**
- Generating per-issue ARNs (that's the architect's job, not this
  ticket).

---

### T9 — Agent mail: scope-dedup and parallel-synthesis substrate

**Depends on:** T1 (uses core types).

**Why:** Today nothing prevents two agents from independently working on
overlapping scope, and once they do there's no mechanism to compare and
synthesise their outputs. `conductr mail` is the shared bulletin board
that fixes both: agents publish what they're working on (so others can
see overlap before they start), and once parallel PRs exist for the same
issue, a synthesiser agent reads them through the mailbox and proposes a
merged solution.

**Two consumers, one substrate:**
- *Scope dedup* — before triggering implementation, the orchestrator
  scans recent mail for in-scope claims that overlap with the candidate
  issue and either skips, merges, or assigns differently.
- *Synthesis* — when ≥2 PRs exist for the same issue (e.g. someone ran
  `/orchestrate` twice, or two agents raced), a synthesis flow reads
  both diffs from mail and produces a third PR that picks the strongest
  parts of each.

**Files to create:**
- `crates/conductr-core/src/types.rs` — add:
  ```rust
  pub struct MailMessage {
      pub id: String,                // ULID-ish, server-assigned
      pub from: AgentId,
      pub kind: MailKind,
      pub subject: String,
      pub body: String,
      pub refs: Vec<MailRef>,        // issue/pr/file pointers
      pub posted_at: DateTime<Utc>,
      pub thread_id: Option<String>, // groups replies
  }
  pub enum MailKind {
      ScopeClaim { issue: u64, files: Vec<String>, summary: String },
      SynthesisRequest { issue: u64, pr_numbers: Vec<u64> },
      SynthesisProposal { issue: u64, pr_numbers: Vec<u64>, diff_url: Option<String> },
      Note,
  }
  pub enum MailRef { Issue(u64), Pr(u64), File(String), Message(String) }
  pub struct AgentId(pub String);    // e.g. "claude-thread4@host" or "github-bot"
  ```
- `crates/conductr-core/src/ports.rs` — add `Mailbox` port:
  ```rust
  #[async_trait]
  pub trait Mailbox: Send + Sync {
      async fn send(&self, msg: &MailMessage) -> Result<String, MailboxError>;
      async fn inbox(&self, since: Option<DateTime<Utc>>, kinds: &[MailKindFilter])
          -> Result<Vec<MailMessage>, MailboxError>;
      async fn thread(&self, thread_id: &str) -> Result<Vec<MailMessage>, MailboxError>;
  }
  ```
- `crates/conductr-mail/` (new use-case crate) with:
  - `src/dedup.rs` — `pub async fn check_scope(mailbox: &impl Mailbox,
    issue: u64, candidate_files: &[String]) -> ScopeReport` returns
    overlapping ScopeClaim messages.
  - `src/synthesise.rs` — `pub async fn request_synthesis(mailbox: &impl
    Mailbox, issue: u64, pr_numbers: Vec<u64>) -> Result<String, ...>`
    posts a SynthesisRequest message and returns its id. The actual diff
    synthesis is done by a Claude agent reading the message; this crate
    just brokers the request.
- `crates/conductr-adapters/src/mail_fs.rs` (behind feature
  `mail-fs`) — first adapter implementation: append-only JSONL files
  under `.conductr/mail/<thread>.jsonl`. Good enough for single-host
  experimentation.
- `crates/conductr-adapters/src/mail_github.rs` (behind feature
  `mail-github`) — second adapter: maps `MailMessage` to a GitHub
  Discussion or to comments on a sentinel issue (one issue per repo,
  e.g. `#0` titled `agent-mail`). Use whichever the repo configures.
- `crates/conductr-adapters/src/mock.rs` — extend with `MockMailbox`.

**CLI surface:**
- `conductr mail send --kind scope-claim --issue <N> --files
  <a,b,c> --summary <s>`
- `conductr mail inbox [--kind <k>] [--since <duration>]`
- `conductr mail dedup --issue <N> [--files <a,b,c>]` — runs
  `dedup::check_scope` and prints overlapping claims.
- `conductr mail synthesise --issue <N> --prs <p1,p2,...>` — posts a
  SynthesisRequest and returns the message id.

**Hook into orchestrator:**
- `conductr-orchestrate` gains an *optional* `&dyn Mailbox` on the
  `Orchestrator`. When present, `run_cycle` calls
  `dedup::check_scope` for each Ready issue before triggering and skips
  with `Bucket::ScopeOverlap { existing_message_id }` if a match is
  found. Without a mailbox the behaviour is unchanged.

**Acceptance:**
- `conductr mail send` and `conductr mail inbox` round-trip a message
  through both `mail-fs` and `mail-github` adapters.
- A unit test on `dedup::check_scope` returns the expected overlaps for
  a `MockMailbox` populated with two ScopeClaim messages.
- Running `conductr orchestrate --once` against a repo with an existing
  ScopeClaim covering issue #N skips that issue with the new
  `ScopeOverlap` bucket.

**Out of scope:**
- The actual LLM-side synthesis logic. We post the request; the
  consuming agent does the merge. That's a skill change for a follow-up
  ticket if needed.
- Auth and ACLs on the mailbox. First-pass is single-tenant.

---

### T10 — Project maturity model + `conductr setup` wizard

**Depends on:** T1.

**Why:** Today the repo's "is it ready for `/orchestrate`?" answer is
folklore (CI? `dev` branch? `.claude/base.md`? Claude GitHub App?
CODEOWNERS? skills installed?). Encode it. The wizard walks the
checklist on a target repo, reports a maturity level, and offers to
install the missing pieces.

**Files to create:**
- `crates/conductr-core/src/types.rs` — add:
  ```rust
  pub struct MaturityCheck {
      pub id: &'static str,                 // e.g. "ci-workflow"
      pub level: MaturityLevel,
      pub label: &'static str,
      pub fixable: bool,
  }
  pub enum MaturityLevel { L0Bootstrap, L1Tested, L2GitFlow, L3Architected, L4Skilled, L5Orchestrated }
  pub struct MaturityReport {
      pub repo: PathBuf,
      pub level_reached: MaturityLevel,
      pub checks: Vec<MaturityCheckResult>,
  }
  pub struct MaturityCheckResult {
      pub check: MaturityCheck,
      pub passed: bool,
      pub detail: Option<String>,
  }
  ```
- `crates/conductr-setup/` (new use-case crate):
  - `src/checks.rs` — one function per check. Each returns
    `MaturityCheckResult`. The catalogue:
    - **L1 Tested**: `cargo test --workspace` (or repo-language equiv)
      runs locally; `.github/workflows/*.yml` runs tests on push;
      `.gitignore` covers `target/`.
    - **L2 GitFlow**: `dev` (or `develop`) branch exists; `main` is
      protected; default base for PRs is `dev`; `git flow init` config
      present (optional but nice).
    - **L3 Architected**: `.claude/base.md` exists; `CONTRIBUTING.md`
      mentions architecture conventions; `CODEOWNERS` present.
    - **L4 Skilled**: at least one `skills/<name>/SKILL.md` shipped;
      `.claude/agents/` populated as appropriate; the `conductr-pod`
      skill is installed at `~/.claude/skills/` or available as a
      plugin.
    - **L5 Orchestrated**: Claude GitHub App installed on the repo;
      `.github/workflows/claude.yml` workflow present (the bot's entry
      point); a green run of `conductr orchestrate --once --dry-run`.
  - `src/fixes.rs` — one function per *fixable* check. Each does the
    minimal thing to flip the check from failing to passing:
    - `add_ci_workflow()` writes `.github/workflows/test.yml`.
    - `init_git_flow()` creates `dev` branch off `main`, sets it as
      default with `gh repo edit --default-branch dev` (asks first).
    - `add_codeowners()` writes a starter `CODEOWNERS`.
    - `install_claude_app()` *opens* the install URL
      (`https://github.com/apps/claude` or the user's configured app)
      in the browser and prints follow-up steps; never tries to
      auto-install.
    - `add_claude_workflow()` writes `.github/workflows/claude.yml` from
      a template.
  - `src/wizard.rs` — `pub async fn run(repo: &Path, mode: WizardMode) ->
    Result<MaturityReport>` with `WizardMode { Interactive,
    NonInteractive, DryRun }`. Interactive prompts before each fix;
    NonInteractive fixes everything fixable; DryRun reports without
    writing.

**CLI surface:**
- `conductr setup status [--repo <path>]` — report only, no writes.
- `conductr setup wizard [--repo <path>] [--non-interactive]
  [--dry-run]` — walk the checklist and offer fixes.
- `conductr setup install-claude-app` — directly invoke that fix.

**Documentation deliverable:**
- `docs/setup.md` — the human-readable form of the same checklist:
  what each level means, why you'd want it, exactly what files / config
  / external installs the wizard touches. The wizard's printed output
  links to this doc per check.

**Acceptance:**
- `conductr setup status` against the current `conductr` repo prints a
  truthful report (it should currently be ~L2 once the plan PR and the
  CI workflow merge).
- `conductr setup wizard --dry-run` against a fresh empty repo prints
  the full set of fixes it would apply, with no side effects.
- `conductr setup wizard --non-interactive` on a fresh empty test repo
  brings it to L4 in one run (L5 needs the Claude App install which the
  wizard cannot fully automate; it stops with a clear "open this URL"
  message).
- `docs/setup.md` is < 300 lines and includes a checklist a user can
  tick off manually.

**Out of scope:**
- Language-agnostic checks (we're Rust-only for now). The CI check
  encodes `cargo test` directly — multi-language support is later work.
- Self-hosted GitHub Enterprise variations of the App install.

---

### T11 — `conductr schedule from-plan` (plan → sheet music)

**Depends on:** T1.

**Why:** The schedule crate already turns musical notation into ASCII
timelines. Inverting the relationship — turning an *implementation plan*
into a pattern — turns the dependency graph into something you can
render, time, and (eventually) reason about as a piece of music.

**The mapping:**
- One *bar* per topological batch. Parallel work fits into the same
  bar.
- One *beat* per ticket inside the batch.
- Note value per beat encodes the ticket's estimated weight:
  - whole `w` ≥ 1 day
  - half `h` ~ half day
  - quarter `q` ~ 2 hours
  - eighth `e` ~ 1 hour
  - 16th `s` ~ 30 min
  - 32nd `t` ~ 15 min
- Beat *tag* is `<ticket-id>:<note-value>`, e.g. `T3:q`.
- Subdivisions express ticket sub-tasks (`T3:q[design:e,impl:e]`).
- Time signature follows the bar with the most beats; the renderer pads
  shorter bars with `rest:<value>` to keep total bar duration constant.

**Files to create:**
- `crates/conductr-schedule/src/from_plan.rs`:
  - `pub fn parse_plan(markdown: &str) -> Result<Plan, PlanError>` —
    extracts ticket sections (`### T<n> — <title>`), their `Depends on:`
    line, and an optional `Estimate:` line (default `q`).
  - `pub fn plan_to_pattern(plan: &Plan) -> Pattern` — builds a
    `Pattern` (the existing type) using the topo-batch / note-value
    mapping above.
  - `Plan` and `PlanItem` types live in this module (not core — they're
    schedule-specific).
- `crates/conductr/src/main.rs` — add the subcommand:
  - `conductr schedule from-plan <path>` — parses the file, builds the
    pattern, prints the pattern DSL.
  - `conductr schedule from-plan <path> --render` — additionally
    renders the ASCII timeline (existing `render_ascii`).

**Acceptance:**
- `conductr schedule from-plan docs/hex-refactor-plan.md --render`
  produces a sensible timeline: one bar per batch, T3-T6 in the same
  bar, T7 alone in the next bar, etc.
- Round-trip property test: `parse(plan_to_pattern(plan).to_dsl())`
  produces the same pattern (or, if that's painful, a structural-equality
  test on the parsed pattern).
- A snapshot test against `docs/hex-refactor-plan.md` (or a smaller
  fixture) so renderer changes don't silently break the timeline.

**Out of scope:**
- Synchronising the plan with real wall-clock time. The pattern is a
  *score*, not a Gantt — `quarter_duration` is the user's choice.
- Bidirectional sync (pattern → plan). One-way is enough for now.
