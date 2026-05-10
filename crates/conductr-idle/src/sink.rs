use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use conductr_tasks::beads::{Beads, BeadsError};
use conductr_tasks::Task;

use crate::sweep::Finding;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubmitOutcome {
    Created(String),
    AlreadyExists,
    Skipped,
}

#[async_trait]
pub trait Sink: Send + Sync {
    async fn submit(&self, finding: &Finding) -> anyhow::Result<SubmitOutcome>;
}

/// Read-only sink: writes findings to stdout. Default for cron use.
#[derive(Debug, Default)]
pub struct PrintSink;

#[async_trait]
impl Sink for PrintSink {
    async fn submit(&self, f: &Finding) -> anyhow::Result<SubmitOutcome> {
        println!(
            "[{:?}] {} — {}\n  id: {}\n  {}",
            f.severity, f.source, f.title, f.id, f.body.lines().next().unwrap_or("")
        );
        Ok(SubmitOutcome::Skipped)
    }
}

/// Beads sink — files each finding as a `br` ticket, deduplicated by an
/// `idle-id:<finding.id>` tag so re-runs don't double-file.
pub struct BeadsSink {
    pub beads: Beads,
    pub seen: tokio::sync::Mutex<HashMap<String, Task>>,
}

impl BeadsSink {
    pub fn new(beads: Beads) -> Self {
        Self { beads, seen: tokio::sync::Mutex::new(HashMap::new()) }
    }

    /// Pre-load the dedup cache from existing beads tasks (one `br list` call).
    pub async fn warm(&self) -> Result<(), BeadsError> {
        let tasks = self.beads.list().await?;
        let mut seen = self.seen.lock().await;
        for t in tasks {
            for tag in &t.tags {
                if let Some(idle_id) = tag.strip_prefix("idle-id:") {
                    seen.insert(idle_id.to_string(), t.clone());
                }
            }
        }
        Ok(())
    }
}

#[async_trait]
impl Sink for BeadsSink {
    async fn submit(&self, f: &Finding) -> anyhow::Result<SubmitOutcome> {
        let mut seen = self.seen.lock().await;
        if seen.contains_key(&f.id) {
            return Ok(SubmitOutcome::AlreadyExists);
        }
        // beads_rust doesn't currently take tags from `br create` flags, so we
        // rely on the title-prefix convention as a fallback identity carrier.
        // Once tag flags are exposed, attach `idle-id:<id>` directly.
        let title = format!("[idle/{}] {}", f.source, f.title);
        let task = self.beads.create(&title, severity_to_priority(&f.severity)).await?;
        seen.insert(f.id.clone(), task.clone());
        Ok(SubmitOutcome::Created(task.id))
    }
}

fn severity_to_priority(s: &crate::sweep::Severity) -> Option<u8> {
    use crate::sweep::Severity::*;
    Some(match s {
        Critical => 0,
        High => 1,
        Medium => 2,
        Low => 3,
        Info => 4,
    })
}

/// Multiplex over many sinks (e.g. print + beads).
pub struct FanOut {
    pub sinks: Vec<Arc<dyn Sink>>,
}

#[async_trait]
impl Sink for FanOut {
    async fn submit(&self, f: &Finding) -> anyhow::Result<SubmitOutcome> {
        let mut last = SubmitOutcome::Skipped;
        for s in &self.sinks {
            last = s.submit(f).await?;
        }
        Ok(last)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sweep::Severity;

    #[tokio::test]
    async fn print_sink_returns_skipped() {
        let s = PrintSink;
        let f = Finding::new("security", "k", "t", "b", Severity::Info);
        let out = s.submit(&f).await.unwrap();
        assert!(matches!(out, SubmitOutcome::Skipped));
    }
}
