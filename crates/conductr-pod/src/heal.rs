//! Restart crashed Claude Code sessions.
//!
//! For every session we diagnose as [`Health::Crashed`], we type `claude` into
//! its pane and press Enter. We don't try to restore the prior conversation
//! (Claude's `--continue` is the user's call, not ours), and we leave non-pod
//! sessions alone unless the caller explicitly opts in via the name pattern.

use serde::{Deserialize, Serialize};

use conductr_core::ports::TmuxAgent;
use conductr_core::types::{Diagnosis, Health, TmuxError};

use crate::diagnose::diagnose_all;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealPlan {
    pub session: String,
    pub action: HealAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HealAction {
    /// Run `claude` in the pane to bring the agent back up.
    RestartClaude { command: String },
    /// Session is healthy; no action needed.
    Skip { reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealOutcome {
    pub plan: HealPlan,
    pub executed: bool,
    pub error: Option<String>,
}

/// Walk every matching session and restart the crashed ones.
///
/// `command` is the literal text we'll send to the pane (default: `claude`).
/// Set `dry_run` to plan without sending keys.
pub async fn heal_all(
    tmux: &impl TmuxAgent,
    pattern: Option<&str>,
    command: &str,
    dry_run: bool,
) -> Result<Vec<HealOutcome>, TmuxError> {
    let diagnoses = diagnose_all(tmux, pattern).await?;
    let mut out = Vec::with_capacity(diagnoses.len());
    for d in diagnoses {
        let plan = plan_for(&d, command);
        let outcome = match (&plan.action, dry_run) {
            (HealAction::RestartClaude { command }, false) => match tmux.send_line(&plan.session, command).await {
                Ok(()) => HealOutcome { plan, executed: true, error: None },
                Err(e) => HealOutcome {
                    plan: plan.clone(),
                    executed: false,
                    error: Some(e.to_string()),
                },
            },
            _ => HealOutcome { plan, executed: false, error: None },
        };
        out.push(outcome);
    }
    Ok(out)
}

fn plan_for(d: &Diagnosis, command: &str) -> HealPlan {
    let action = match &d.health {
        Health::Crashed { .. } => HealAction::RestartClaude { command: command.to_string() },
        Health::Idle { .. } => HealAction::Skip { reason: "idle, awaiting input".into() },
        Health::Working { activity } => HealAction::Skip { reason: format!("working: {activity}") },
        Health::Unknown { reason } => HealAction::Skip { reason: format!("unclassified: {reason}") },
    };
    HealPlan { session: d.session.name.clone(), action }
}

#[cfg(test)]
mod tests {
    use super::*;
    use conductr_adapters::mock::MockTmuxAgent;

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

    const CRASHED_PANE: &str = r#"
hello world
$ exit
exit
[Process completed]
user@host:~/developer$
"#;

    #[tokio::test]
    async fn idle_session_is_skipped() {
        let mock = MockTmuxAgent::new().with_session("s1", IDLE_PANE);
        let outcomes = heal_all(&mock, None, "claude", false).await.unwrap();
        assert_eq!(outcomes.len(), 1);
        assert!(!outcomes[0].executed);
        assert!(matches!(outcomes[0].plan.action, HealAction::Skip { .. }));
        assert_eq!(mock.sent_lines("s1"), Vec::<String>::new());
    }

    #[tokio::test]
    async fn crashed_session_gets_restart() {
        let mock = MockTmuxAgent::new().with_session("s1", CRASHED_PANE);
        let outcomes = heal_all(&mock, None, "claude", false).await.unwrap();
        assert_eq!(outcomes.len(), 1);
        assert!(outcomes[0].executed);
        assert!(matches!(outcomes[0].plan.action, HealAction::RestartClaude { .. }));
        assert_eq!(mock.sent_lines("s1"), vec!["claude"]);
    }

    #[tokio::test]
    async fn dry_run_does_not_send_line() {
        let mock = MockTmuxAgent::new().with_session("s1", CRASHED_PANE);
        let outcomes = heal_all(&mock, None, "claude", true).await.unwrap();
        assert_eq!(outcomes.len(), 1);
        assert!(!outcomes[0].executed);
        assert_eq!(mock.sent_lines("s1"), Vec::<String>::new());
    }
}
