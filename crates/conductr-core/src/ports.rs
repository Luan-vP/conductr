use std::collections::BTreeSet;

use async_trait::async_trait;

pub use crate::types::{InstanceError, TmuxError};

use crate::types::{
    InstanceHandle, InstanceSpec, Issue, IssueNumber, MailKind, MailMessage, MailRef, Pr, PrNumber,
    RepoSlug, Task, TmuxSession,
};

// ── IssueTracker ──────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum IssueTrackerError {
    #[error("tracker not installed or not found")]
    NotInstalled,
    #[error("io: {0}")]
    Io(String),
    #[error("parse: {0}")]
    Parse(String),
    #[error("api: {0}")]
    Api(String),
    #[error("backend: {0}")]
    Backend(String),
    #[error("configuration: {0}")]
    Configuration(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("operation not supported by this tracker")]
    Unsupported,
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

// ── TmuxAgent ─────────────────────────────────────────────────────────────────

#[async_trait]
pub trait TmuxAgent: Send + Sync {
    async fn list_sessions(&self) -> Result<Vec<TmuxSession>, TmuxError>;
    async fn capture_pane(&self, session: &str) -> Result<String, TmuxError>;
    async fn send_line(&self, session: &str, text: &str) -> Result<(), TmuxError>;
    async fn send_key(&self, session: &str, key: &str) -> Result<(), TmuxError>;
}

// ── InstanceProvider ──────────────────────────────────────────────────────────

#[async_trait]
pub trait InstanceProvider: Send + Sync {
    async fn spin_up(&self, spec: &InstanceSpec) -> Result<InstanceHandle, InstanceError>;
    async fn connect(&self, handle: &InstanceHandle) -> Result<(), InstanceError>;
    async fn run(&self, handle: &InstanceHandle, cmd: &str) -> Result<String, InstanceError>;
    async fn tear_down(&self, handle: &InstanceHandle) -> Result<(), InstanceError>;
}

// ── Mailbox ───────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum MailboxError {
    #[error("io: {0}")]
    Io(String),
    #[error("parse: {0}")]
    Parse(String),
    #[error("backend: {0}")]
    Backend(String),
}

/// Shared bulletin board used by agents to claim scope and request synthesis.
///
/// A single trait covers both reads and writes (Rule 6: one trait per port).
#[async_trait]
pub trait Mailbox: Send + Sync {
    /// Append a new message. Returns its assigned id.
    async fn send(&self, agent: &str, payload: MailKind) -> Result<MailRef, MailboxError>;

    /// Return all messages in the inbox, optionally filtered by kind tag.
    ///
    /// `kind_filter` matches the `kind` field of `MailKind` (e.g. `"scope_claim"`).
    async fn inbox(
        &self,
        kind_filter: Option<&str>,
        since: Option<std::time::Duration>,
    ) -> Result<Vec<MailMessage>, MailboxError>;

    /// Return all messages in a named thread (the thread id is the `MailRef`
    /// of the first message in that thread).
    async fn thread(&self, thread_id: &MailRef) -> Result<Vec<MailMessage>, MailboxError>;
}
