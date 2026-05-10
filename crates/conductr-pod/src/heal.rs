//! Restart crashed Claude Code sessions.
//!
//! For every session we diagnose as [`Health::Crashed`], we type `claude` into
//! its pane and press Enter. We don't try to restore the prior conversation
//! (Claude's `--continue` is the user's call, not ours), and we leave non-pod
//! sessions alone unless the caller explicitly opts in via the name pattern.

use serde::{Deserialize, Serialize};

use conductr_core::types::{Diagnosis, Health, TmuxError};

use crate::diagnose::diagnose_all;
use crate::tmux::Tmux;

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
    tmux: &Tmux,
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
