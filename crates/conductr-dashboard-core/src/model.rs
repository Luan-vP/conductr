use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub use conductr_core::maturity::MaturityLevel;
pub use conductr_core::types::{
    CiRunRow, CiStatus, ClosedPr, Diagnosis, Finding, FindingSeverity, Pr, PrState, RepoSlug,
    SafetyPreset,
};

// ── §4.1 Repo / project registry ─────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RepoStatus {
    Active,
    Pending,
    Archived,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoEntry {
    pub slug: RepoSlug,
    pub tag: String,
    pub local_path: String,
    pub status: RepoStatus,
    pub cadence: HashMap<String, String>,
    pub maturity: Option<MaturityLevel>,
    /// Safety preset override from the repo's `.conductr` `[orchestrate]` section.
    /// `None` means the effective preset is derived from `maturity`.
    pub safety_preset: Option<SafetyPreset>,
    /// Chord size cap from the repo's `.conductr` `[orchestrate].max_parallel_beats`.
    pub max_parallel_beats: Option<u32>,
}

// ── §4.2 Orchestrate cycles ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CycleTrigger {
    Cron,
    Manual,
    Webhook,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CycleState {
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BeatState {
    Queued,
    Running,
    Done,
    Skipped,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Beat {
    pub name: String,
    pub state: BeatState,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub pr_number: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cycle {
    pub repo: RepoSlug,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub trigger: CycleTrigger,
    pub state: CycleState,
    pub beats: Vec<Beat>,
    pub pr_numbers: Vec<u64>,
}

// ── §4.3 Pull requests ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrGrouped {
    pub repo: RepoSlug,
    pub mergeable_green: Vec<Pr>,
    pub mergeable_red: Vec<Pr>,
    pub conflicting: Vec<Pr>,
    pub draft: Vec<Pr>,
}

// ── §4.4 Idle findings ────────────────────────────────────────────────────────

/// Dashboard-wire `Finding` extends the core `Finding` with metadata needed by
/// outlets. `issue_number` and `first_seen` are added here; `repo` anchors the
/// finding to a specific project.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FindingEntry {
    pub title: String,
    pub body: String,
    pub severity: FindingSeverity,
    pub fingerprint: String,
    pub issue_number: Option<u64>,
    pub repo: RepoSlug,
    pub first_seen: DateTime<Utc>,
}

impl FindingEntry {
    pub fn from_core(f: Finding, repo: RepoSlug, first_seen: DateTime<Utc>) -> Self {
        Self {
            title: f.title,
            body: f.body,
            severity: f.severity,
            fingerprint: f.fingerprint,
            issue_number: None,
            repo,
            first_seen,
        }
    }
}

// ── §4.5 Pod / tmux ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PodSnapshot {
    pub sessions: Vec<Diagnosis>,
}

// ── §4.6 Cadence staff ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StaffGlyph {
    Head,
    Rest,
    Hit,
    Tied,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StaffHit {
    pub at: DateTime<Utc>,
    pub duration_seconds: Option<f64>,
    pub glyph: StaffGlyph,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StaffRow {
    pub label: String,
    pub hits: Vec<StaffHit>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StaffWindow {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CadenceStaff {
    pub repo: RepoSlug,
    pub window: StaffWindow,
    pub rows: Vec<StaffRow>,
}

// ── §4.7 Cron schedule ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronEntry {
    pub expression: String,
    pub command: String,
    pub marker: String,
    pub next_fire: DateTime<Utc>,
}

// ── §4.8 Local-agent health ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LocalAgentKind {
    Ollama,
    Llamacpp,
    Pi,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalAgentEntry {
    pub kind: LocalAgentKind,
    pub endpoint: String,
    pub reachable: bool,
    pub latency_ms: Option<u64>,
    pub models: Vec<String>,
    pub last_checked: DateTime<Utc>,
}

// ── §4.9 Build / CI ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CiAggregate {
    Green,
    Red,
    Amber,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CiSnapshot {
    pub repo: RepoSlug,
    pub recent_runs: Vec<CiRunRow>,
    pub current_status: CiAggregate,
}

// ── Full dashboard state (GET /state) ────────────────────────────────────────

/// The full snapshot returned by `GET /state`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardState {
    pub repos: Vec<RepoEntry>,
    pub cycles: Vec<Cycle>,
    pub prs_by_repo: Vec<PrGrouped>,
    pub findings: Vec<FindingEntry>,
    pub pod: PodSnapshot,
    pub cron: Vec<CronEntry>,
    pub local_agents: Vec<LocalAgentEntry>,
}

impl Default for DashboardState {
    fn default() -> Self {
        Self {
            repos: Vec::new(),
            cycles: Vec::new(),
            prs_by_repo: Vec::new(),
            findings: Vec::new(),
            pod: PodSnapshot { sessions: Vec::new() },
            cron: Vec::new(),
            local_agents: Vec::new(),
        }
    }
}
