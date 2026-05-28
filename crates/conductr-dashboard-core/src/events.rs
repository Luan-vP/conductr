use serde::{Deserialize, Serialize};

use conductr_core::types::RepoSlug;

/// All SSE event types emitted on `GET /events` (§6 of dashboard-api.md).
///
/// The `event_name()` method returns the SSE `event:` field; `to_data_json()`
/// serialises only the inner payload (the SSE `data:` field — no event tag).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum SseEvent {
    PrOpened(PrRef),
    PrChanged(PrRef),
    PrClosed(PrRef),
    PrMerged(PrRef),
    CycleStarted(CycleRef),
    CycleFinished(CycleFinishedPayload),
    PodSessionChanged(SessionRef),
    PodSessionCrashed(SessionRef),
    FindingNew(FindingRef),
    FindingResolved(FindingRef),
    CadenceTick(RepoRef),
    LocalAgentChanged(LocalAgentRef),
    DaemonStale(StalePayload),
    DaemonUnstale(UnstalePayload),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrRef {
    pub repo: RepoSlug,
    pub number: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CycleRef {
    pub repo: RepoSlug,
    pub trigger: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CycleFinishedPayload {
    pub repo: RepoSlug,
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRef {
    pub session: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FindingRef {
    pub repo: RepoSlug,
    pub fingerprint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub severity: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoRef {
    pub repo: RepoSlug,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalAgentRef {
    pub kind: String,
    pub reachable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StalePayload {
    pub source: String,
    pub reason: String,
    pub retry_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnstalePayload {
    pub source: String,
}

impl SseEvent {
    /// SSE event name (the `event:` field in the stream).
    pub fn event_name(&self) -> &'static str {
        match self {
            SseEvent::PrOpened(_) => "pr.opened",
            SseEvent::PrChanged(_) => "pr.changed",
            SseEvent::PrClosed(_) => "pr.closed",
            SseEvent::PrMerged(_) => "pr.merged",
            SseEvent::CycleStarted(_) => "cycle.started",
            SseEvent::CycleFinished(_) => "cycle.finished",
            SseEvent::PodSessionChanged(_) => "pod.session_changed",
            SseEvent::PodSessionCrashed(_) => "pod.session_crashed",
            SseEvent::FindingNew(_) => "finding.new",
            SseEvent::FindingResolved(_) => "finding.resolved",
            SseEvent::CadenceTick(_) => "cadence.tick",
            SseEvent::LocalAgentChanged(_) => "local_agent.changed",
            SseEvent::DaemonStale(_) => "daemon.stale",
            SseEvent::DaemonUnstale(_) => "daemon.unstale",
        }
    }

    /// Serialize only the inner payload (used as the SSE `data:` field).
    pub fn to_data_json(&self) -> String {
        match self {
            SseEvent::PrOpened(p) | SseEvent::PrChanged(p) | SseEvent::PrClosed(p)
            | SseEvent::PrMerged(p) => serde_json::to_string(p),
            SseEvent::CycleStarted(p) => serde_json::to_string(p),
            SseEvent::CycleFinished(p) => serde_json::to_string(p),
            SseEvent::PodSessionChanged(p) | SseEvent::PodSessionCrashed(p) => serde_json::to_string(p),
            SseEvent::FindingNew(p) | SseEvent::FindingResolved(p) => serde_json::to_string(p),
            SseEvent::CadenceTick(p) => serde_json::to_string(p),
            SseEvent::LocalAgentChanged(p) => serde_json::to_string(p),
            SseEvent::DaemonStale(p) => serde_json::to_string(p),
            SseEvent::DaemonUnstale(p) => serde_json::to_string(p),
        }
        .unwrap_or_else(|_| "{}".into())
    }
}
