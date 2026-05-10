//! Background maintenance ("idle work") for a project.
//!
//! `conductr idle` runs a set of [`Sweep`]s — small, idempotent, read-only-by-default
//! checks whose job is to *keep a project ticking along* between active development
//! sessions. Each sweep returns [`Finding`]s; findings flow to a [`Sink`] which can
//! print them, file beads tickets, open GitHub issues, or push to Notion.
//!
//! ## What belongs in `idle`
//!
//! Tasks that are:
//!
//! - **Periodic** — they should run on a cron / git hook, not on demand.
//! - **Read-only by default** — safe to schedule unattended; opt in to filing.
//! - **Idempotent** — re-running yields the same findings (so dedup works).
//! - **Maintenance, not feature work** — they unblock or de-risk human/agent work
//!   rather than implementing it.
//!
//! Initial built-in sweeps:
//!
//! - [`sweeps::security`] — surfaces security work the project owes itself
//!   (vulnerable deps via `cargo audit`, etc.) and turns each into a Finding.
//! - [`sweeps::git_flow`] — checks git-flow hygiene: forward-integration gaps
//!   (commits on `main` not yet on `develop`), PRs targeting the wrong base.
//! - [`sweeps::smoke_tests`] — finds recent commits on the development branch
//!   that have no recorded smoke-test run.
//!
//! ## Single vs multiple instances
//!
//! Conductr is designed to run on a single instance. If RAM or "chord" size
//! (the number of active agents) becomes a constraint, sweeps can be sharded
//! across instances with [`Runner::shard`]: each instance handles findings
//! whose stable id hashes into its slot. Sweeps remain independent and
//! idempotent, so this is a deployment knob, not a code change.

pub mod runner;
pub mod sink;
pub mod sweep;
pub mod sweeps;

pub use runner::{Runner, RunReport, Shard};
pub use sink::{PrintSink, BeadsSink, Sink, SubmitOutcome};
pub use sweep::{Finding, Severity, Sweep, SweepContext};
