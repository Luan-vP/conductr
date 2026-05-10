// ── conductr-tasks types ──────────────────────────────────────────────────────

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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncReport {
    pub pushed: Vec<String>,
    pub failed: Vec<(String, String)>,
}

// ── conductr-pod types ────────────────────────────────────────────────────────

use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TmuxSession {
    pub name: String,
    pub created: DateTime<Utc>,
    pub last_activity: DateTime<Utc>,
    pub windows: u32,
    pub attached: bool,
    pub cwd: Option<String>,
}

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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Health {
    Idle {
        last_message: Option<String>,
        tokens: Option<String>,
    },
    Working {
        activity: String,
    },
    Crashed {
        last_shell_line: Option<String>,
    },
    Unknown {
        reason: String,
    },
}

impl Health {
    pub fn is_alive(&self) -> bool {
        matches!(self, Health::Idle { .. } | Health::Working { .. })
    }

    pub fn needs_heal(&self) -> bool {
        matches!(self, Health::Crashed { .. })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnosis {
    pub session: TmuxSession,
    pub health: Health,
    pub idle_seconds: i64,
    pub tail: Vec<String>,
}

// ── conductr-orchestrate types ────────────────────────────────────────────────

use std::collections::{BTreeMap, BTreeSet};

pub type IssueNumber = u64;
pub type PrNumber = u64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoSlug {
    pub owner: String,
    pub repo: String,
}

impl RepoSlug {
    pub fn new(owner: impl Into<String>, repo: impl Into<String>) -> Self {
        Self { owner: owner.into(), repo: repo.into() }
    }
}

impl std::fmt::Display for RepoSlug {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.owner, self.repo)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Issue {
    pub number: IssueNumber,
    pub title: String,
    pub body: String,
    pub labels: Vec<String>,
    pub state: IssueState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IssueState {
    Open,
    Closed,
}

impl Issue {
    pub fn has_label(&self, label: &str) -> bool {
        self.labels.iter().any(|l| l.eq_ignore_ascii_case(label))
    }

    pub fn is_human(&self) -> bool {
        self.has_label("human")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pr {
    pub number: PrNumber,
    pub title: String,
    pub body: String,
    pub head_ref: String,
    pub state: PrState,
    pub ci: CiStatus,
    pub linked_issue: Option<IssueNumber>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PrState {
    Open,
    Merged,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CiStatus {
    Passing,
    Failing,
    Pending,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Bucket {
    Ready,
    PrOpen,
    PrFailing,
    Blocked,
    TriggeredWaiting,
    Human,
    AlreadyClosed,
    ScopeOverlap { existing_message_id: String },
}

#[derive(Debug, Clone)]
pub struct Classification {
    pub issue: IssueNumber,
    pub bucket: Bucket,
    pub blocking: Vec<IssueNumber>,
    pub pr: Option<u64>,
}

#[derive(Debug, Clone, Default)]
pub struct DepGraph {
    pub edges: BTreeMap<IssueNumber, BTreeSet<IssueNumber>>,
}

#[derive(Debug, thiserror::Error)]
pub enum GraphError {
    #[error("cycle detected involving issues: {0:?}")]
    Cycle(Vec<IssueNumber>),
    #[error("issue #{from} depends on #{to} which is not in the batch")]
    DependencyOutsideBatch { from: IssueNumber, to: IssueNumber },
}

impl DepGraph {
    pub fn new() -> Self { Self::default() }

    pub fn add_issue(&mut self, issue: IssueNumber) {
        self.edges.entry(issue).or_default();
    }

    pub fn add_dep(&mut self, from: IssueNumber, to: IssueNumber) {
        self.edges.entry(from).or_default().insert(to);
        self.edges.entry(to).or_default();
    }

    pub fn issues(&self) -> impl Iterator<Item = IssueNumber> + '_ {
        self.edges.keys().copied()
    }

    pub fn deps_of(&self, issue: IssueNumber) -> Option<&BTreeSet<IssueNumber>> {
        self.edges.get(&issue)
    }

    pub fn check_closed(&self) -> Result<(), GraphError> {
        for (from, deps) in &self.edges {
            for to in deps {
                if !self.edges.contains_key(to) {
                    return Err(GraphError::DependencyOutsideBatch { from: *from, to: *to });
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct OrchestratorConfig {
    pub repo: RepoSlug,
    pub trigger_comment: String,
    pub poll_interval: std::time::Duration,
    pub max_cycles: Option<u32>,
    pub default_human_assignee: Option<String>,
    pub dry_run: bool,
    /// How to resolve local vs GitHub CI status when a `LocalCi` adapter is
    /// attached. Ignored when no adapter is wired in.
    pub ci_mode: CiMode,
}

impl OrchestratorConfig {
    pub fn new(repo: RepoSlug) -> Self {
        Self {
            repo,
            trigger_comment: "@claude please implement".into(),
            poll_interval: std::time::Duration::from_secs(60),
            max_cycles: None,
            default_human_assignee: None,
            dry_run: false,
            ci_mode: CiMode::PreferLocal,
        }
    }
}

// ── local-ci types ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum CiMode {
    Local,
    #[default]
    PreferLocal,
    PreferGithub,
    Github,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalCiConfig {
    pub commands: Vec<String>,
    pub timeout_secs: u64,
    pub mode: CiMode,
}

impl Default for LocalCiConfig {
    fn default() -> Self {
        Self { commands: Vec::new(), timeout_secs: 900, mode: CiMode::PreferLocal }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandRun {
    pub cmd: String,
    pub exit: i32,
    pub stdout_tail: String,
    pub stderr_tail: String,
    pub duration_secs: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalCiReport {
    pub status: CiStatus,
    pub commands: Vec<CommandRun>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrLocalCiResult {
    pub pr: PrNumber,
    pub status: CiStatus,
    pub commands: Vec<CommandRun>,
}

#[derive(Debug, Default, Clone)]
pub struct CycleReport {
    pub merged: Vec<u64>,
    pub triggered: Vec<IssueNumber>,
    pub waiting: Vec<IssueNumber>,
    pub blocked: Vec<IssueNumber>,
    pub human: Vec<IssueNumber>,
    pub pr_failing: Vec<u64>,
    pub scope_overlap: Vec<IssueNumber>,
    pub progress_made: bool,
    pub local_ci: Vec<PrLocalCiResult>,
}

// ── conductr-instance types ───────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstanceSpec {
    pub name: String,
    pub provider: Provider,
    pub size: String,
    pub region: Option<String>,
    pub image: Option<String>,
    pub ssh_key: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Aws,
    Hetzner,
    DigitalOcean,
    Local,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstanceHandle {
    pub id: String,
    pub provider: Provider,
    pub host: String,
    pub user: String,
}

#[derive(Debug, thiserror::Error)]
pub enum InstanceError {
    #[error("provider {0:?} not implemented yet")]
    NotImplemented(Provider),
    #[error("ssh: {0}")]
    Ssh(String),
    #[error("provider error: {0}")]
    Provider(String),
}

// ── local-agent types ────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum LocalAgentError {
    #[error("agent not installed")]
    NotInstalled,
    #[error("agent unreachable: {0}")]
    Unreachable(String),
    #[error("backend error: {0}")]
    Backend(String),
    #[error("model missing: {0}")]
    ModelMissing(String),
}

// ── conductr-mail types ───────────────────────────────────────────────────────

/// Opaque identifier for an agent (e.g. "claude/issue-16-…" branch name or session name).
pub type AgentId = String;

/// Opaque reference to a `MailMessage` (its unique id string).
pub type MailRef = String;

/// The kind of payload carried by a `MailMessage`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MailKind {
    /// An agent claiming ownership of a set of files for a given issue.
    ScopeClaim {
        issue: IssueNumber,
        files: Vec<String>,
        summary: String,
    },
    /// A request for a synthesis agent to merge two or more PRs for one issue.
    SynthesisRequest {
        issue: IssueNumber,
        pr_numbers: Vec<u64>,
    },
    /// A proposed merged solution produced by a synthesis agent.
    SynthesisProposal {
        issue: IssueNumber,
        request_id: MailRef,
        body: String,
    },
    /// An informational note from an agent (free-form).
    Note { text: String },
}

/// A single message on the shared bulletin board.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MailMessage {
    pub id: MailRef,
    pub agent: AgentId,
    pub sent_at: DateTime<Utc>,
    pub payload: MailKind,
}
