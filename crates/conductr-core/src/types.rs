// ── safety types ─────────────────────────────────────────────────────────────

/// Safety preset controlling branch-isolation and chord behaviour.
///
/// Ordered from least to most restrictive. Can be set per-orchestrator
/// (via `OrchestratorConfig`), per-repo maturity, or per-issue (`safety:<preset>` label).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SafetyPreset {
    /// Solo branch. Sibling branches are not fetched or surfaced.
    Unhinged,
    /// Solo branch. Post-merge conflict detection emits a `MailKind::Yell` event but does not block.
    #[default]
    Feral,
    /// Read-only sibling awareness. Advisory comment posted on likely-conflict overlap; routine still runs.
    Fast,
    /// Soft-chord. Routine awaits green siblings; dispatches if siblings amber/red after timeout.
    Strict,
    /// Hard-chord. Orchestrator serialises all routines through a single coordinator lock.
    Bureaucratic,
}

/// Aggregated CI health of sibling branches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SiblingStatus {
    /// All siblings have passing CI.
    Green,
    /// At least one sibling has pending/unknown CI (no failures).
    Amber,
    /// At least one sibling has failing CI.
    Red,
}

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
    /// Maximum number of Ready issues to dispatch (trigger `@claude`) in a
    /// single orchestrate pass. Corresponds to `[orchestrate] max_parallel_beats`
    /// in `.conductr`. Default: 3.
    pub max_parallel_beats: usize,
    /// Maximum number of parallel `qa<n>` tmux slots. When the pool is full,
    /// new QA work is deferred. Corresponds to `[orchestrate] max_parallel_qa`.
    /// Default: 2.
    pub max_parallel_qa: usize,
    /// Working directory for newly-spawned tmux agent/qa panes. When `None`
    /// the orchestrator uses `"."` (the process cwd).
    pub tmux_cwd: Option<String>,
    /// Path to the `.conductr` project config file. When set, the orchestrator
    /// appends `[[tempo.prs]]` and `[[ci.runs]]` rows on PR close/merge.
    pub conductr_config_path: Option<std::path::PathBuf>,
    /// Orchestrator-level safety preset override. When `None`, the effective
    /// preset is derived from repo maturity (via `resolve_preset`). Individual
    /// issues can further override via `safety:<preset>` label.
    pub safety_preset: Option<SafetyPreset>,
    /// Soft-chord timeout for the `STRICT` preset. If sibling branches remain
    /// amber after this duration (from when the issue was first deferred), the
    /// orchestrator dispatches anyway. Default: 10 minutes.
    pub soft_chord_timeout: std::time::Duration,
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
            max_parallel_beats: 3,
            max_parallel_qa: 2,
            tmux_cwd: None,
            conductr_config_path: None,
            safety_preset: None,
            soft_chord_timeout: std::time::Duration::from_secs(600),
        }
    }
}

// ── Closed PR (for tempo write-back) ─────────────────────────────────────────

/// A PR that has been closed or merged. Returned by `ScmHost::list_closed_prs`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClosedPr {
    pub number: PrNumber,
    pub title: String,
    pub body: String,
    pub head_ref: String,
    pub state: PrState,
    pub linked_issue: Option<IssueNumber>,
    pub opened_at: DateTime<Utc>,
    pub closed_at: DateTime<Utc>,
    pub merged: bool,
}

/// One row in `[[tempo.prs]]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TempoPrRow {
    pub number: PrNumber,
    pub title: String,
    pub phrase: Option<String>,
    pub chord: Option<String>,
    pub complexity: Complexity,
    pub opened: DateTime<Utc>,
    pub closed: DateTime<Utc>,
    pub merged: bool,
}

/// One row in `[[ci.runs]]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CiRunRow {
    pub pr: PrNumber,
    pub minutes: f64,
    pub ts: DateTime<Utc>,
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
    /// Issues deferred by the soft-chord (STRICT) or hard-chord (BUREAUCRATIC) this cycle.
    pub soft_chord_deferred: Vec<IssueNumber>,
    pub progress_made: bool,
    pub local_ci: Vec<PrLocalCiResult>,
}

// ── complexity ────────────────────────────────────────────────────────────────

/// Issue complexity bucket used by the architect ARN and the orchestrate
/// tempo write-back.  Precedence chain when reading:
/// 1. GitHub label `complexity/{xs,s,m,l}`
/// 2. ARN complexity field (set by `architect plan`)
/// 3. Default `M`
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Complexity {
    XS,
    S,
    M,
    L,
}

impl Complexity {
    pub fn as_str(self) -> &'static str {
        match self {
            Complexity::XS => "XS",
            Complexity::S => "S",
            Complexity::M => "M",
            Complexity::L => "L",
        }
    }
}

impl std::fmt::Display for Complexity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
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

// ── Calendar types ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarEvent {
    pub id: String,
    pub title: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewCalendarEvent {
    pub title: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub description: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateCalendarEvent {
    pub title: Option<String>,
    pub start: Option<DateTime<Utc>>,
    pub end: Option<DateTime<Utc>>,
    pub description: Option<String>,
}

// ── conductr-idle types ───────────────────────────────────────────────────────

/// Category of an idle-scan finding. Influences the label applied to the
/// resulting GitHub issue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FindingSeverity {
    Architecture,
    Quality,
    Coverage,
    Security,
}

/// One drift detected by an idle-scan pass (architecture rule violation,
/// clippy warning, coverage gap, CLI/skill parity drift, …). Each finding has
/// a stable `fingerprint` used to dedupe against existing issues.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub title: String,
    pub body: String,
    pub severity: FindingSeverity,
    pub fingerprint: String,
}


// ── CI gate types ─────────────────────────────────────────────────────────────

/// Pass/fail state of a single CI check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    /// Check passed.
    Green,
    /// Advisory warning; not a hard failure.
    Amber,
    /// Check failed with a test-assertion or logic failure.
    Red,
    /// Check is still running.
    Pending,
    /// Transient infrastructure failure (timeout, runner error) — retriable under STRICT.
    Transient,
}

/// A single CI check in a branch's check suite.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Check {
    pub name: String,
    pub status: CheckStatus,
    /// `true` = required check; `false` = advisory/optional.
    pub required: bool,
}

/// A branch pending merge, together with all context the merge gate needs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Branch {
    pub head_ref: String,
    pub checks: Vec<Check>,
    /// A human reviewer has approved the PR (BUREAUCRATIC).
    pub review_approved: bool,
    /// The linked issue is closed (BUREAUCRATIC).
    pub linked_issue_closed: bool,
    /// An ADR file is present in the branch's commits (BUREAUCRATIC).
    pub adr_present: bool,
}

/// The outcome of the merge predicate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeDecision {
    /// Branch is safe to merge.
    Allowed,
    /// Branch cannot be merged; describes the first blocking reason.
    BlockedBy(BlockReason),
    /// One transient infrastructure failure was detected; retry once before blocking.
    RetryOnce(RetryReason),
}

/// Why a merge was blocked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockReason {
    /// FERAL: no required check is green.
    NoRequiredCheckGreen,
    /// A required check is not green (name of the failing check).
    RequiredCheckFailed(String),
    /// An advisory check is not green (name of the check).
    AdvisoryCheckFailed(String),
    /// BUREAUCRATIC: no human review approval.
    ReviewNotApproved,
    /// BUREAUCRATIC: linked issue is still open.
    LinkedIssueNotClosed,
    /// BUREAUCRATIC: no ADR file found in the branch's commits.
    AdrNotPresent,
}

/// Why a retry was requested.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetryReason {
    /// Name of the check that produced a transient signal.
    pub check_name: String,
}
