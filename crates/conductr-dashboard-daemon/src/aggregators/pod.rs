use anyhow::Result;
use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use conductr_core::types::{Diagnosis, Health, TmuxSession};
use conductr_dashboard_core::{
    events::SessionRef,
    model::PodSnapshot,
    SseEvent,
};
use tokio::sync::broadcast;

use crate::state::SharedState;
use super::Aggregator;

pub struct PodAggregator;

impl PodAggregator {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Aggregator for PodAggregator {
    async fn refresh(
        &self,
        state: &SharedState,
        tx: &broadcast::Sender<SseEvent>,
    ) -> Result<()> {
        let sessions = list_sessions().await?;
        let mut diagnoses = Vec::new();
        let prev_names: Vec<String> = state
            .read()
            .await
            .pod
            .sessions
            .iter()
            .map(|d| d.session.name.clone())
            .collect();

        for s in sessions {
            let session_name = s.name.clone();
            match diagnose(s).await {
                Ok(d) => {
                    // Emit change event if session state changed
                    let prev_health = state
                        .read()
                        .await
                        .pod
                        .sessions
                        .iter()
                        .find(|prev| prev.session.name == session_name)
                        .map(|prev| format!("{:?}", prev.health));
                    let cur_health = format!("{:?}", d.health);
                    if prev_health.as_deref() != Some(&cur_health) {
                        let payload = SessionRef { session: session_name.clone() };
                        if matches!(d.health, Health::Crashed { .. }) {
                            let _ = tx.send(SseEvent::PodSessionCrashed(payload));
                        } else {
                            let _ = tx.send(SseEvent::PodSessionChanged(payload));
                        }
                    }
                    diagnoses.push(d);
                }
                Err(e) => {
                    tracing::warn!("failed to diagnose session {session_name}: {e:#}");
                }
            }
        }

        let _ = prev_names; // suppress unused warning
        state.write().await.pod = PodSnapshot { sessions: diagnoses };
        Ok(())
    }
}

/// Run `tmux list-sessions -F` and parse basic session metadata.
async fn list_sessions() -> Result<Vec<TmuxSession>> {
    let format = "#{session_name}\t#{session_created}\t#{session_activity}\t#{session_windows}\t#{session_attached}";
    let out = tokio::process::Command::new("tmux")
        .args(["list-sessions", "-F", format])
        .output()
        .await;

    let out = match out {
        Ok(o) if o.status.success() => o,
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            if stderr.contains("no server running") || stderr.contains("no sessions") {
                return Ok(Vec::new());
            }
            anyhow::bail!("tmux list-sessions failed: {}", stderr);
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };

    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut sessions = Vec::new();
    for line in stdout.lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() < 5 {
            continue;
        }
        let name = parts[0].to_string();
        let created_ts: i64 = parts[1].parse().unwrap_or(0);
        let activity_ts: i64 = parts[2].parse().unwrap_or(0);
        let windows: u32 = parts[3].parse().unwrap_or(0);
        let attached = parts[4] == "1";

        sessions.push(TmuxSession {
            name,
            created: Utc.timestamp_opt(created_ts, 0).single().unwrap_or(Utc::now()),
            last_activity: Utc.timestamp_opt(activity_ts, 0).single().unwrap_or(Utc::now()),
            windows,
            attached,
            cwd: None,
        });
    }
    Ok(sessions)
}

/// Capture a session's pane and build a full `Diagnosis`.
async fn diagnose(session: TmuxSession) -> Result<Diagnosis> {
    let out = tokio::process::Command::new("tmux")
        .args(["capture-pane", "-p", "-t", &session.name])
        .output()
        .await?;

    let pane = String::from_utf8_lossy(&out.stdout).into_owned();
    let health = classify_pane(&pane);
    let tail: Vec<String> = pane
        .lines()
        .filter(|l| !l.trim().is_empty())
        .rev()
        .take(8)
        .map(|s| s.to_string())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();

    let idle_seconds = (Utc::now() - session.last_activity).num_seconds().max(0);
    Ok(Diagnosis { session, health, idle_seconds, tail })
}

fn classify_pane(pane: &str) -> Health {
    let claude_markers = ["❯", "auto mode", "shift+tab", "claude", "Claude"];
    let is_claude = claude_markers.iter().any(|m| pane.contains(m));

    if !is_claude {
        let last = pane.lines().filter(|l| !l.trim().is_empty()).next_back().map(|s| s.to_string());
        return Health::Crashed { last_shell_line: last };
    }

    // Detect active work by spinner frames or "Running" indicators
    let working_markers = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏", "Running", "Executing"];
    if let Some(marker) = working_markers.iter().find(|m| pane.contains(*m)) {
        return Health::Working { activity: marker.to_string() };
    }

    Health::Idle { last_message: None, tokens: None }
}
