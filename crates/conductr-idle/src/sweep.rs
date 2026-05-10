use std::path::PathBuf;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use conductr_orchestrate::RepoSlug;

/// Severity of a finding. Tunes the default sink behaviour and downstream
/// triage policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Low,
    Medium,
    High,
    Critical,
}

/// One actionable observation produced by a [`Sweep`].
///
/// `id` is the *stable identity* of this finding across runs: two runs that
/// observe the same condition must produce the same id, so sinks can dedup.
/// Convention: `format!("{source}:{stable_key}")`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub id: String,
    pub source: String,
    pub title: String,
    pub body: String,
    pub severity: Severity,
    pub labels: Vec<String>,
}

impl Finding {
    pub fn new(
        source: &'static str,
        stable_key: impl AsRef<str>,
        title: impl Into<String>,
        body: impl Into<String>,
        severity: Severity,
    ) -> Self {
        Self {
            id: format!("{source}:{}", stable_key.as_ref()),
            source: source.to_string(),
            title: title.into(),
            body: body.into(),
            severity,
            labels: Vec::new(),
        }
    }

    pub fn with_labels(mut self, labels: impl IntoIterator<Item = String>) -> Self {
        self.labels.extend(labels);
        self
    }
}

/// Context passed to every sweep.
#[derive(Debug, Clone)]
pub struct SweepContext {
    /// Path to the working copy.
    pub repo_root: PathBuf,
    /// Owner/repo slug, if a remote is configured.
    pub repo_slug: Option<RepoSlug>,
    /// Long-lived development branch (git-flow `develop`, often).
    pub develop_branch: String,
    /// Production / mainline branch.
    pub main_branch: String,
    /// Lower bound for "recent" lookups.
    pub since: DateTime<Utc>,
}

impl SweepContext {
    pub fn new(repo_root: PathBuf) -> Self {
        Self {
            repo_root,
            repo_slug: None,
            develop_branch: "develop".into(),
            main_branch: "main".into(),
            since: Utc::now() - chrono::Duration::days(7),
        }
    }
}

#[async_trait]
pub trait Sweep: Send + Sync {
    fn name(&self) -> &'static str;
    async fn run(&self, ctx: &SweepContext) -> anyhow::Result<Vec<Finding>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finding_id_is_stable() {
        let a = Finding::new("security", "cve-2025-9999", "x", "y", Severity::High);
        let b = Finding::new("security", "cve-2025-9999", "x", "y", Severity::High);
        assert_eq!(a.id, b.id);
        assert_eq!(a.id, "security:cve-2025-9999");
    }

    #[test]
    fn finding_with_labels() {
        let f = Finding::new("git-flow", "fwd-gap", "t", "b", Severity::Medium)
            .with_labels(["maintenance".to_string(), "git-flow".to_string()]);
        assert_eq!(f.labels.len(), 2);
    }
}
