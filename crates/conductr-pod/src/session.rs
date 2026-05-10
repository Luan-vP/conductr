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
