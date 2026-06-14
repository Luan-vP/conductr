use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use conductr_core::ports::{CrontabAgent, CrontabError};
use conductr_dashboard_core::{model::CronEntry, SseEvent};
use tokio::sync::broadcast;

use super::Aggregator;
use crate::state::SharedState;

/// Reads the user's crontab and extracts entries tagged with
/// `# conductr-cron:` markers.
pub struct CronAggregator {
    crontab: Arc<dyn CrontabAgent>,
}

impl CronAggregator {
    pub fn new(crontab: Arc<dyn CrontabAgent>) -> Self {
        Self { crontab }
    }
}

#[async_trait]
impl Aggregator for CronAggregator {
    async fn refresh(
        &self,
        state: &SharedState,
        _tx: &broadcast::Sender<SseEvent>,
    ) -> Result<()> {
        let entries = match self.crontab.list().await {
            Ok(text) => parse_crontab(&text),
            Err(CrontabError::NoCrontab) | Err(CrontabError::NotInstalled) => Vec::new(),
            Err(e) => return Err(e.into()),
        };
        state.write().await.cron = entries;
        Ok(())
    }
}

fn parse_crontab(text: &str) -> Vec<CronEntry> {
    let mut entries = Vec::new();

    for line in text.lines() {
        let trimmed = line.trim();
        // Skip pure comment lines and empty lines
        if trimmed.is_empty() || (trimmed.starts_with('#') && !trimmed.contains("conductr-cron:")) {
            continue;
        }
        // Only include lines that have a conductr-cron marker (inline comment style)
        if !trimmed.contains("conductr-cron:") {
            continue;
        }
        // Extract the marker text
        let marker = trimmed
            .split('#')
            .find(|s| s.contains("conductr-cron:"))
            .map(|s| format!("# {}", s.trim()))
            .unwrap_or_default();

        if let Some(entry) = parse_cron_line(trimmed, &marker) {
            entries.push(entry);
        }
    }
    entries
}

fn parse_cron_line(line: &str, marker: &str) -> Option<CronEntry> {
    // Strip inline comment for parsing
    let code_part = line.split('#').next().unwrap_or(line).trim();
    let parts: Vec<&str> = code_part.splitn(6, ' ').collect();
    if parts.len() < 6 {
        return None;
    }
    let expression = parts[..5].join(" ");
    let command = parts[5..].join(" ").trim().to_string();
    if command.is_empty() {
        return None;
    }

    // Rough next-fire: just use now+1min as placeholder (real scheduling is complex)
    let next_fire = Utc::now() + chrono::Duration::minutes(1);

    Some(CronEntry {
        expression,
        command,
        marker: marker.to_string(),
        next_fire,
    })
}
