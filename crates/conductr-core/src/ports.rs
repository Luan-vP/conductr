use std::collections::BTreeSet;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

pub use crate::types::{InstanceError, LocalAgentError, TmuxError};

use crate::types::{
    CalendarEvent, ClosedPr, InstanceHandle, InstanceSpec, Issue, IssueNumber, LocalCiReport,
    MailKind, MailMessage, MailRef, NewCalendarEvent, Pr, PrNumber, RepoSlug, Task, TmuxSession,
    UpdateCalendarEvent,
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
    async fn create_issue(
        &self,
        repo: &RepoSlug,
        title: &str,
        body: &str,
        labels: &[&str],
    ) -> anyhow::Result<IssueNumber>;

    /// Add a label to an issue. No-op by default (opt-in for adapters that
    /// support GH label management).
    async fn add_issue_label(
        &self,
        _repo: &RepoSlug,
        _n: IssueNumber,
        _label: &str,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    /// Remove a label from an issue. No-op by default.
    async fn remove_issue_label(
        &self,
        _repo: &RepoSlug,
        _n: IssueNumber,
        _label: &str,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    /// Return the most recently closed or merged PRs (up to 100). Used by the
    /// orchestrator to drive tempo write-back and stale-label cleanup.
    async fn list_closed_prs(&self, _repo: &RepoSlug) -> anyhow::Result<Vec<ClosedPr>> {
        Ok(vec![])
    }

    /// Return the wall-clock duration in minutes of the latest CI run on
    /// `head_ref`, or `None` if no run exists.
    async fn latest_ci_run_minutes(
        &self,
        _repo: &RepoSlug,
        _head_ref: &str,
    ) -> anyhow::Result<Option<f64>> {
        Ok(None)
    }
}

// ── TmuxAgent ─────────────────────────────────────────────────────────────────

#[async_trait]
pub trait TmuxAgent: Send + Sync {
    async fn list_sessions(&self) -> Result<Vec<TmuxSession>, TmuxError>;
    async fn capture_pane(&self, session: &str) -> Result<String, TmuxError>;
    async fn send_line(&self, session: &str, text: &str) -> Result<(), TmuxError>;
    async fn send_key(&self, session: &str, key: &str) -> Result<(), TmuxError>;
    async fn new_session(&self, name: &str, cwd: &str) -> Result<(), TmuxError>;
}

// ── InstanceProvider ──────────────────────────────────────────────────────────

#[async_trait]
pub trait InstanceProvider: Send + Sync {
    async fn spin_up(&self, spec: &InstanceSpec) -> Result<InstanceHandle, InstanceError>;
    async fn connect(&self, handle: &InstanceHandle) -> Result<(), InstanceError>;
    async fn run(&self, handle: &InstanceHandle, cmd: &str) -> Result<String, InstanceError>;
    async fn tear_down(&self, handle: &InstanceHandle) -> Result<(), InstanceError>;
}

// ── LocalAgent ────────────────────────────────────────────────────────────────

#[async_trait]
pub trait LocalAgent: Send + Sync {
    async fn name(&self) -> &'static str;
    async fn complete(&self, prompt: &str) -> Result<String, LocalAgentError>;
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

// ── LocalCi ───────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum LocalCiError {
    #[error("io: {0}")]
    Io(String),
    #[error("git: {0}")]
    Git(String),
}

/// Runs a project-defined command list against a PR branch HEAD and returns a
/// `LocalCiReport`. The repo root and command configuration are held by the
/// adapter; `head_ref` is the remote branch name (e.g. `claude/issue-1-foo`).
#[async_trait]
pub trait LocalCi: Send + Sync {
    async fn run(&self, head_ref: &str) -> anyhow::Result<LocalCiReport>;
}

// ── CalendarPort ─────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum CalendarError {
    #[error("http: {0}")]
    Http(String),
    #[error("authentication failed — check GCAL_OAUTH_TOKEN")]
    Auth,
    #[error("api error: {0}")]
    Api(String),
    #[error("parse error: {0}")]
    Parse(String),
    #[error("not found: {0}")]
    NotFound(String),
}

#[async_trait]
pub trait CalendarPort: Send + Sync {
    async fn list_upcoming_events(
        &self,
        from: DateTime<Utc>,
    ) -> Result<Vec<CalendarEvent>, CalendarError>;

    async fn create_event(
        &self,
        event: NewCalendarEvent,
    ) -> Result<CalendarEvent, CalendarError>;

    async fn update_event(
        &self,
        id: &str,
        event: UpdateCalendarEvent,
    ) -> Result<CalendarEvent, CalendarError>;

    async fn delete_event(&self, id: &str) -> Result<(), CalendarError>;
}

// ── Mailbox ───────────────────────────────────────────────────────────────────

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
