use anyhow::Result;
use async_trait::async_trait;
use conductr_dashboard_core::model::{RepoEntry, RepoStatus};
use conductr_dashboard_core::SseEvent;
use tokio::sync::broadcast;

use crate::state::SharedState;
use super::Aggregator;

/// Reads the per-repo `.conductr` config files from the repos registered in
/// `~/.conductr` and populates `DashboardState::repos`.
///
/// In v1 this scans known project directories configured via the CONDUCTR_REPOS
/// env var (colon-separated list of `owner/repo:path` pairs). When unset,
/// an empty list is used and the outlet shows no repos.
pub struct ReposAggregator;

impl ReposAggregator {
    pub fn new() -> Self {
        Self
    }

    fn read_from_env() -> Vec<RepoEntry> {
        let raw = match std::env::var("CONDUCTR_REPOS") {
            Ok(v) if !v.is_empty() => v,
            _ => return Vec::new(),
        };

        let mut entries = Vec::new();
        for segment in raw.split(':') {
            let parts: Vec<&str> = segment.splitn(2, '=').collect();
            if parts.len() != 2 {
                continue;
            }
            let slug_str = parts[0];
            let path = parts[1];
            let slug_parts: Vec<&str> = slug_str.splitn(2, '/').collect();
            if slug_parts.len() != 2 {
                continue;
            }
            use conductr_core::types::RepoSlug;
            let slug = RepoSlug::new(slug_parts[0], slug_parts[1]);
            entries.push(RepoEntry {
                slug,
                tag: slug_str.replace('/', "-"),
                local_path: path.to_string(),
                status: RepoStatus::Active,
                cadence: Default::default(),
                maturity: None,
            });
        }
        entries
    }
}

#[async_trait]
impl Aggregator for ReposAggregator {
    async fn refresh(
        &self,
        state: &SharedState,
        _tx: &broadcast::Sender<SseEvent>,
    ) -> Result<()> {
        let repos = tokio::task::spawn_blocking(Self::read_from_env).await?;
        state.write().await.repos = repos;
        Ok(())
    }
}
