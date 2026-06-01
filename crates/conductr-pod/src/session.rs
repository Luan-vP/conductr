//! Session bootstrap: ensure a named tmux session exists and report its health.
//!
//! The key operation is [`ensure_session`], which either creates a new session
//! (returning [`SessionState::Created`]) or diagnoses an existing one (returning
//! [`SessionState::Existing`] with the current [`Health`]). The caller is
//! responsible for deciding what to do next (start Claude, send a prompt, skip).

use conductr_core::ports::TmuxAgent;
use conductr_core::types::{Health, TmuxError};

use crate::diagnose::diagnose_one;

/// What `ensure_session` found or did.
#[derive(Debug, Clone)]
pub enum SessionState {
    /// The session did not exist and was just created (no process running yet).
    Created,
    /// The session already existed; carries its current [`Health`].
    Existing(Health),
}

/// Ensure a tmux session named `name` exists, creating it if absent.
///
/// If the session already exists, its pane is captured and classified via
/// [`diagnose_one`], and [`SessionState::Existing`] is returned. If it was
/// absent, `tmux new-session -d -s <name> -c <cwd>` creates it and
/// [`SessionState::Created`] is returned. No Claude Code process is started —
/// that is the caller's responsibility.
pub async fn ensure_session(
    tmux: &impl TmuxAgent,
    name: &str,
    cwd: &str,
) -> Result<SessionState, TmuxError> {
    let sessions = tmux.list_sessions().await?;
    if sessions.iter().any(|s| s.name == name) {
        let d = diagnose_one(tmux, name).await?;
        Ok(SessionState::Existing(d.health))
    } else {
        tmux.new_session(name, cwd).await?;
        Ok(SessionState::Created)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDLE_PANE: &str = r#"
           Claude Code v2.1.114
 ▐▛███▜▌   Opus 4.7 (1M context) · Claude Max
▝▜█████▛▘  ~/developer/foo

❯ /clear
  ⎿  (no content)

────────────────────────────────────────────────────────────────────────────────
❯ make one more
────────────────────────────────────────────────────────────────────────────────
  ⏵⏵ auto mode on (shift+tab to cycle)
  ✗ Auto-update failed
                                        new task? /clear to save 174.5k tokens
"#;

    #[tokio::test]
    async fn ensure_creates_missing_session() {
        use conductr_adapters::mock::MockTmuxAgent;
        let mock = MockTmuxAgent::new(); // no sessions
        let state = ensure_session(&mock, "conductr-demo", "/tmp").await.unwrap();
        assert!(matches!(state, SessionState::Created));
    }

    #[tokio::test]
    async fn ensure_reuses_existing_idle_session() {
        use conductr_adapters::mock::MockTmuxAgent;
        let mock = MockTmuxAgent::new().with_session("conductr-demo", IDLE_PANE);
        let state = ensure_session(&mock, "conductr-demo", "/tmp").await.unwrap();
        assert!(
            matches!(state, SessionState::Existing(Health::Idle { .. })),
            "expected Existing(Idle), got {state:?}"
        );
    }

    #[tokio::test]
    async fn ensure_is_idempotent_second_call_returns_existing() {
        use conductr_adapters::mock::MockTmuxAgent;
        let mock = MockTmuxAgent::new();

        // First call: session missing → Created
        let first = ensure_session(&mock, "conductr-demo", "/tmp").await.unwrap();
        assert!(matches!(first, SessionState::Created));

        // Second call: session now exists (created by first call with empty pane)
        let second = ensure_session(&mock, "conductr-demo", "/tmp").await.unwrap();
        // Session exists but pane is empty → Unknown (not an error, just unclassified)
        assert!(
            matches!(second, SessionState::Existing(_)),
            "second call should return Existing, got {second:?}"
        );
    }

    #[tokio::test]
    async fn ensure_does_not_create_session_that_already_exists() {
        use conductr_adapters::mock::MockTmuxAgent;
        let mock = MockTmuxAgent::new().with_session("conductr-demo", IDLE_PANE);

        // Call twice — both should return Existing, not Created.
        for _ in 0..2 {
            let state = ensure_session(&mock, "conductr-demo", "/tmp").await.unwrap();
            assert!(
                matches!(state, SessionState::Existing(_)),
                "expected Existing on repeated calls, got {state:?}"
            );
        }
    }
}
