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

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub status: TaskStatus,
    pub priority: Option<u8>,
    pub depends_on: Vec<String>,
    pub tags: Vec<String>,
    pub body: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TaskStatus {
    NotStarted,
    InProgress,
    Blocked,
    Done,
}
