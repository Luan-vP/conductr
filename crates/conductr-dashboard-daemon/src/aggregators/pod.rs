use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use conductr_core::{
    ports::{TmuxAgent, TmuxError},
    types::{Diagnosis, Health, TmuxSession},
};
use conductr_dashboard_core::{
    events::SessionRef,
    model::PodSnapshot,
    SseEvent,
};
use tokio::sync::broadcast;

use super::Aggregator;
use crate::state::SharedState;

pub struct PodAggregator {
    tmux: Arc<dyn TmuxAgent>,
}

impl PodAggregator {
    pub fn new(tmux: Arc<dyn TmuxAgent>) -> Self {
        Self { tmux }
    }
}

#[async_trait]
impl Aggregator for PodAggregator {
    async fn refresh(
        &self,
        state: &SharedState,
        tx: &broadcast::Sender<SseEvent>,
    ) -> Result<()> {
        let sessions = match self.tmux.list_sessions().await {
            Ok(s) => s,
            Err(TmuxError::NotInstalled) | Err(TmuxError::NoServer) => Vec::new(),
            Err(e) => return Err(e.into()),
        };

        let mut diagnoses = Vec::new();

        for s in sessions {
            let session_name = s.name.clone();
            match self.diagnose(s).await {
                Ok(d) => {
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

        state.write().await.pod = PodSnapshot { sessions: diagnoses };
        Ok(())
    }
}

impl PodAggregator {
    async fn diagnose(&self, session: TmuxSession) -> Result<Diagnosis> {
        let pane = self.tmux.capture_pane(&session.name).await.map_err(anyhow::Error::from)?;
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
        let remote_control = remote_control_active(&pane);
        Ok(Diagnosis { session, health, idle_seconds, remote_control, tail })
    }
}

/// Mirror of `conductr_pod::diagnose::remote_control_active` — kept inline to
/// avoid a crate dependency, consistent with the local `classify_pane` copy.
fn remote_control_active(pane: &str) -> bool {
    pane.contains("/remote-control is active") || pane.contains("/rc active")
}

fn classify_pane(pane: &str) -> Health {
    let claude_markers = ["❯", "auto mode", "shift+tab", "claude", "Claude"];
    let is_claude = claude_markers.iter().any(|m| pane.contains(m));

    if !is_claude {
        let last = pane.lines().filter(|l| !l.trim().is_empty()).next_back().map(|s| s.to_string());
        return Health::Crashed { last_shell_line: last };
    }

    let working_markers = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏", "Running", "Executing"];
    if let Some(marker) = working_markers.iter().find(|m| pane.contains(*m)) {
        return Health::Working { activity: marker.to_string() };
    }

    Health::Idle { last_message: None, tokens: None }
}
