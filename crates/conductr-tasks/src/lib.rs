//! Task tracking for conductr.
//!
//! Two backends:
//! - [`beads`] — wraps the `br` CLI from <https://github.com/Dicklesworthstone/beads_rust>
//!   (vendored at `vendor/beads_rust`). Local SQLite + JSONL storage.
//! - [`notion`] — minimal Notion REST client for syncing into Notion databases.
//!
//! Both speak the same [`Task`] type so a sync layer can move records between
//! them.

pub mod beads;
pub mod notion;
pub mod sync;

pub use conductr_core::types::*;
