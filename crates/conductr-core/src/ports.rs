use std::collections::BTreeSet;

use async_trait::async_trait;

use crate::types::{
    InstanceHandle, InstanceSpec, Issue, IssueNumber, Pr, PrNumber, RepoSlug, Task, TmuxSession,
};

// ── TmuxAgent ─────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum TmuxError {
    #[error("`tmux` not found on PATH")]
    NotInstalled,
    #[error("no tmux server running")]
    NoServer,
    #[error("`tmux {args}` exited with {status}: {stderr}")]
    Exit {
        args: String,
        status: std::process::ExitStatus,
        stderr: String,
    },
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse: {0}")]
    Parse(String),
}

#[async_trait]
pub trait TmuxAgent: Send + Sync {
    async fn list_sessions(&self) -> Result<Vec<TmuxSession>, TmuxError>;
    async fn capture_pane(&self, session: &str) -> Result<String, TmuxError>;
    async fn send_line(&self, session: &str, text: &str) -> Result<(), TmuxError>;
    async fn send_key(&self, session: &str, key: &str) -> Result<(), TmuxError>;
}

// ── IssueTracker ──────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum IssueTrackerError {
    #[error("operation not supported by this adapter")]
    Unsupported,
    #[error("backend error: {0}")]
    Backend(String),
    #[error("configuration error: {0}")]
    Configuration(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("parse error: {0}")]
    Parse(String),
}

#[async_trait]
pub trait IssueTracker: Send + Sync {
    async fn list(&self) -> Result<Vec<Task>, IssueTrackerError>;
    async fn list_ready(&self) -> Result<Vec<Task>, IssueTrackerError>;
    async fn create_full(
        &self,
        title: &str,
        priority: Option<u8>,
        body: Option<&str>,
        labels: &[&str],
    ) -> Result<Task, IssueTrackerError>;
    async fn close(&self, id: &str) -> Result<(), IssueTrackerError>;
    /// Create-or-update a task. Adapters that are write-only (e.g. Notion)
    /// implement this; read-only adapters return `IssueTrackerError::Unsupported`.
    async fn upsert_task(&self, task: &Task) -> Result<(), IssueTrackerError>;
}

// ── ScmHost ───────────────────────────────────────────────────────────────────

#[async_trait]
pub trait ScmHost: Send + Sync {
    async fn list_open_issues(&self, repo: &RepoSlug) -> anyhow::Result<Vec<Issue>>;
    async fn list_closed_issue_numbers(
        &self,
        repo: &RepoSlug,
    ) -> anyhow::Result<BTreeSet<IssueNumber>>;
    async fn list_open_prs(&self, repo: &RepoSlug) -> anyhow::Result<Vec<Pr>>;
    async fn list_issue_comments(
        &self,
        repo: &RepoSlug,
        n: IssueNumber,
    ) -> anyhow::Result<Vec<String>>;
    async fn comment_issue(
        &self,
        repo: &RepoSlug,
        n: IssueNumber,
        body: &str,
    ) -> anyhow::Result<()>;
    async fn assign_issue(
        &self,
        repo: &RepoSlug,
        n: IssueNumber,
        login: &str,
    ) -> anyhow::Result<()>;
    async fn merge_pr_squash(&self, repo: &RepoSlug, n: PrNumber) -> anyhow::Result<()>;
}

// ── InstanceProvider ──────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum InstanceError {
    #[error("provider {0:?} not implemented yet")]
    NotImplemented(crate::types::Provider),
    #[error("ssh: {0}")]
    Ssh(String),
    #[error("provider error: {0}")]
    Provider(String),
}

#[async_trait]
pub trait InstanceProvider: Send + Sync {
    async fn spin_up(&self, spec: &InstanceSpec) -> Result<InstanceHandle, InstanceError>;
    async fn connect(&self, handle: &InstanceHandle) -> Result<(), InstanceError>;
    async fn run(&self, handle: &InstanceHandle, cmd: &str) -> Result<String, InstanceError>;
    async fn tear_down(&self, handle: &InstanceHandle) -> Result<(), InstanceError>;
}
