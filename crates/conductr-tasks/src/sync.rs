//! Sync between beads and Notion.
//!
//! Direction: beads → Notion (push). Reverse direction (pull) is intentionally
//! deferred until the Notion property mapping is locked in.

use crate::beads::Beads;
use crate::notion::Notion;

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error("beads: {0}")]
    Beads(#[from] crate::beads::BeadsError),
    #[error("notion: {0}")]
    Notion(#[from] crate::notion::NotionError),
}

#[derive(Debug, Clone, Default)]
pub struct SyncReport {
    pub pushed: Vec<String>,
    pub failed: Vec<(String, String)>,
}

/// Push every task currently in beads up to a Notion database.
pub async fn beads_to_notion(
    beads: &Beads,
    notion: &Notion,
    database_id: &str,
) -> Result<SyncReport, SyncError> {
    let tasks = beads.list().await?;
    let mut report = SyncReport::default();
    for t in tasks {
        match notion.upsert_task(database_id, &t).await {
            Ok(_) => report.pushed.push(t.id),
            Err(e) => report.failed.push((t.id, e.to_string())),
        }
    }
    Ok(report)
}
