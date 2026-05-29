use serde::{Deserialize, Serialize};

use conductr_core::types::RepoSlug;

/// All SSE event types emitted on `GET /events` (§6 of dashboard-api.md).
///
/// Wire format follows §6: each variant serialises as
/// `{"event":"<dotted.name>","data":{...payload...}}`, and the SSE-framing
/// helpers (`event_name()`, `to_data_json()`) project the same shape onto
/// the `event:` / `data:` lines of the stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", content = "data")]
pub enum SseEvent {
    #[serde(rename = "pr.opened")]
    PrOpened(PrRef),
    #[serde(rename = "pr.changed")]
    PrChanged(PrRef),
    #[serde(rename = "pr.closed")]
    PrClosed(PrRef),
    #[serde(rename = "pr.merged")]
    PrMerged(PrRef),
    #[serde(rename = "cycle.started")]
    CycleStarted(CycleRef),
    #[serde(rename = "cycle.finished")]
    CycleFinished(CycleFinishedPayload),
    #[serde(rename = "pod.session_changed")]
    PodSessionChanged(SessionRef),
    #[serde(rename = "pod.session_crashed")]
    PodSessionCrashed(SessionRef),
    #[serde(rename = "finding.new")]
    FindingNew(FindingRef),
    #[serde(rename = "finding.resolved")]
    FindingResolved(FindingRef),
    #[serde(rename = "cadence.tick")]
    CadenceTick(RepoRef),
    #[serde(rename = "local_agent.changed")]
    LocalAgentChanged(LocalAgentRef),
    #[serde(rename = "daemon.stale")]
    DaemonStale(StalePayload),
    #[serde(rename = "daemon.unstale")]
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn repo() -> RepoSlug {
        RepoSlug::new("luan-vp", "conductr")
    }

    #[test]
    fn pr_opened_serializes_with_dotted_event_and_data_envelope() {
        let ev = SseEvent::PrOpened(PrRef { repo: repo(), number: 142 });
        let v = serde_json::to_value(&ev).unwrap();
        assert_eq!(
            v,
            json!({
                "event": "pr.opened",
                "data": { "repo": { "owner": "luan-vp", "repo": "conductr" }, "number": 142 }
            })
        );
    }

    #[test]
    fn local_agent_changed_keeps_underscore_after_dot() {
        // §6 spells this one "local_agent.changed" — the underscore is part of
        // the family name; only the family→action separator is a dot.
        let ev = SseEvent::LocalAgentChanged(LocalAgentRef {
            kind: "ollama".into(),
            reachable: true,
        });
        let v = serde_json::to_value(&ev).unwrap();
        assert_eq!(v["event"], "local_agent.changed");
    }

    #[test]
    fn every_variant_round_trips_through_serde() {
        let cases: Vec<(SseEvent, &str)> = vec![
            (SseEvent::PrOpened(PrRef { repo: repo(), number: 1 }), "pr.opened"),
            (SseEvent::PrChanged(PrRef { repo: repo(), number: 1 }), "pr.changed"),
            (SseEvent::PrClosed(PrRef { repo: repo(), number: 1 }), "pr.closed"),
            (SseEvent::PrMerged(PrRef { repo: repo(), number: 1 }), "pr.merged"),
            (SseEvent::CycleStarted(CycleRef { repo: repo(), trigger: "cron".into() }), "cycle.started"),
            (SseEvent::CycleFinished(CycleFinishedPayload { repo: repo(), state: "succeeded".into() }), "cycle.finished"),
            (SseEvent::PodSessionChanged(SessionRef { session: "agent1".into() }), "pod.session_changed"),
            (SseEvent::PodSessionCrashed(SessionRef { session: "agent1".into() }), "pod.session_crashed"),
            (SseEvent::FindingNew(FindingRef { repo: repo(), fingerprint: "fp".into(), severity: Some("quality".into()) }), "finding.new"),
            (SseEvent::FindingResolved(FindingRef { repo: repo(), fingerprint: "fp".into(), severity: None }), "finding.resolved"),
            (SseEvent::CadenceTick(RepoRef { repo: repo() }), "cadence.tick"),
            (SseEvent::LocalAgentChanged(LocalAgentRef { kind: "ollama".into(), reachable: true }), "local_agent.changed"),
            (SseEvent::DaemonStale(StalePayload { source: "gh".into(), reason: "rate_limit".into(), retry_at: None }), "daemon.stale"),
            (SseEvent::DaemonUnstale(UnstalePayload { source: "gh".into() }), "daemon.unstale"),
        ];
        for (ev, expected_event) in &cases {
            let s = serde_json::to_string(ev).unwrap();
            let v: serde_json::Value = serde_json::from_str(&s).unwrap();
            assert_eq!(v["event"], *expected_event, "wire event mismatch for {ev:?}");
            let parsed: SseEvent = serde_json::from_str(&s).unwrap();
            // Re-serialize and compare as Value to assert deserialize is the inverse.
            assert_eq!(serde_json::to_value(&parsed).unwrap(), v, "round-trip mismatch for {ev:?}");
        }
    }

    #[test]
    fn serde_event_field_matches_event_name_helper_for_every_variant() {
        let evs = [
            SseEvent::PrOpened(PrRef { repo: repo(), number: 1 }),
            SseEvent::PrChanged(PrRef { repo: repo(), number: 1 }),
            SseEvent::PrClosed(PrRef { repo: repo(), number: 1 }),
            SseEvent::PrMerged(PrRef { repo: repo(), number: 1 }),
            SseEvent::CycleStarted(CycleRef { repo: repo(), trigger: "cron".into() }),
            SseEvent::CycleFinished(CycleFinishedPayload { repo: repo(), state: "succeeded".into() }),
            SseEvent::PodSessionChanged(SessionRef { session: "agent1".into() }),
            SseEvent::PodSessionCrashed(SessionRef { session: "agent1".into() }),
            SseEvent::FindingNew(FindingRef { repo: repo(), fingerprint: "fp".into(), severity: None }),
            SseEvent::FindingResolved(FindingRef { repo: repo(), fingerprint: "fp".into(), severity: None }),
            SseEvent::CadenceTick(RepoRef { repo: repo() }),
            SseEvent::LocalAgentChanged(LocalAgentRef { kind: "ollama".into(), reachable: true }),
            SseEvent::DaemonStale(StalePayload { source: "gh".into(), reason: "rate_limit".into(), retry_at: None }),
            SseEvent::DaemonUnstale(UnstalePayload { source: "gh".into() }),
        ];
        for ev in &evs {
            let v = serde_json::to_value(ev).unwrap();
            assert_eq!(v["event"], ev.event_name(), "drift between derive and event_name() for {ev:?}");
        }
    }
}
