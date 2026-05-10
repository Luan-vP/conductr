//! Classify each tmux session by what's currently rendered in its pane.
//!
//! The Claude Code TUI has a recognisable footprint: a banner, a `❯` prompt,
//! a status line ("auto mode on", "shift+tab to cycle"), and various spinner
//! frames when busy. We use those markers as evidence the agent is alive.
//! Absence of any Claude markers, with a shell prompt visible, means the
//! agent has exited and the session has fallen back to the shell.

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::tmux::{Tmux, TmuxError, TmuxSession};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Health {
    /// Claude Code is rendered and waiting at the input prompt.
    Idle {
        /// Most recent message visible above the prompt, if any.
        last_message: Option<String>,
        /// Token count if the status bar is showing one (`123.4k tokens`).
        tokens: Option<String>,
    },
    /// Claude Code is rendered and actively processing (spinner / tool use).
    Working {
        /// One-line summary of what the spinner / status line says.
        activity: String,
    },
    /// Pane shows a shell prompt and no Claude markers — the agent has exited.
    Crashed {
        /// Last shell line on screen, useful to see *why* it died.
        last_shell_line: Option<String>,
    },
    /// Pane is blank or we couldn't classify it.
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
    /// Seconds since the pane's last activity (per tmux).
    pub idle_seconds: i64,
    /// Last non-empty lines of the captured pane (most recent last).
    pub tail: Vec<String>,
}

/// Run a diagnose pass over all live tmux sessions.
///
/// `pattern` is an optional substring filter on session name. Pass `None` to
/// inspect every session on the host.
pub async fn diagnose_all(tmux: &Tmux, pattern: Option<&str>) -> Result<Vec<Diagnosis>, TmuxError> {
    let sessions = tmux.list_sessions().await?;
    let mut out = Vec::with_capacity(sessions.len());
    for s in sessions {
        if let Some(p) = pattern {
            if !s.name.contains(p) {
                continue;
            }
        }
        out.push(diagnose_session(tmux, s).await?);
    }
    Ok(out)
}

/// Diagnose a single named session.
pub async fn diagnose_one(tmux: &Tmux, name: &str) -> Result<Diagnosis, TmuxError> {
    let sessions = tmux.list_sessions().await?;
    let s = sessions
        .into_iter()
        .find(|s| s.name == name)
        .ok_or_else(|| TmuxError::Parse(format!("no such session: {name}")))?;
    diagnose_session(tmux, s).await
}

async fn diagnose_session(tmux: &Tmux, session: TmuxSession) -> Result<Diagnosis, TmuxError> {
    let pane = tmux.capture_pane(&session.name).await?;
    let health = classify(&pane);
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

fn classify(pane: &str) -> Health {
    let trimmed = pane.trim_end_matches(['\n', ' ']);
    if trimmed.is_empty() {
        return Health::Unknown { reason: "empty pane".into() };
    }

    let claude_running = looks_like_claude_tui(pane);

    if !claude_running {
        let last_shell_line = pane
            .lines()
            .filter(|l| !l.trim().is_empty())
            .next_back()
            .map(|s| s.to_string());
        return Health::Crashed { last_shell_line };
    }

    if let Some(activity) = working_activity(pane) {
        return Health::Working { activity };
    }

    Health::Idle {
        last_message: last_user_or_assistant_message(pane),
        tokens: token_count(pane),
    }
}

fn looks_like_claude_tui(pane: &str) -> bool {
    // Any of these are strong evidence the Claude Code TUI is rendered. The
    // banner art and bottom status line are the most reliable.
    const MARKERS: &[&str] = &[
        "Claude Code v",
        "▐▛███▜▌",
        "auto mode on",
        "shift+tab to cycle",
        "/effort to tune",
        "tokens",
    ];
    let has_marker = MARKERS.iter().any(|m| pane.contains(m));
    let has_prompt_glyph = pane.contains('❯');
    has_marker && has_prompt_glyph
}

fn working_activity(pane: &str) -> Option<String> {
    // The TUI shows a spinner only while a turn is in flight, and it always
    // renders *below* the most recent `❯` prompt. So: find the last `❯`
    // prompt; if any spinner glyph appears after it (skipping the bottom
    // status footer), the agent is working. Otherwise idle.
    let lines: Vec<&str> = pane.lines().collect();
    let last_prompt_idx = lines.iter().rposition(|l| l.trim_start().starts_with('❯'))?;
    for line in &lines[last_prompt_idx + 1..] {
        let l = line.trim();
        if l.is_empty() || is_status_footer(l) {
            continue;
        }
        if l.starts_with('✻') || l.starts_with('✽') || l.starts_with('✺') {
            return Some(l.to_string());
        }
    }
    None
}

fn is_status_footer(l: &str) -> bool {
    // Lines that always render at the bottom regardless of agent state.
    l.contains("auto mode on")
        || l.contains("tokens")
        || l.contains("Auto-update")
        || l.starts_with("⏵⏵")
        || l.chars().all(|c| matches!(c, '─' | '━' | '-' | ' '))
}

fn last_user_or_assistant_message(pane: &str) -> Option<String> {
    // Visible prompts look like `❯ message text`, but the TUI uses a
    // non-breaking space (U+00A0) after the glyph rather than ASCII 0x20.
    // We want the most recent prompt with non-empty content, ignoring the
    // bare live prompt at the bottom.
    let mut found: Option<String> = None;
    for line in pane.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix('❯') {
            let r = rest.trim();
            if !r.is_empty() && !is_placeholder_hint(r) {
                found = Some(r.to_string());
            }
        }
    }
    found
}

fn is_placeholder_hint(s: &str) -> bool {
    // Claude renders dimmed `Try "..."` suggestions in the empty input box on
    // fresh sessions. They sit on the prompt line but are not user input.
    s.starts_with("Try \"") && s.ends_with('"')
}

fn token_count(pane: &str) -> Option<String> {
    // Look for something like "174.5k tokens" in the status footer.
    for line in pane.lines().rev() {
        if let Some(idx) = line.find("tokens") {
            let before = line[..idx].trim_end();
            let token_part: String = before
                .chars()
                .rev()
                .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == 'k' || *c == 'K' || *c == 'M')
                .collect::<String>()
                .chars()
                .rev()
                .collect();
            if !token_part.is_empty() {
                return Some(format!("{token_part} tokens"));
            }
        }
    }
    None
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

    const CRASHED_PANE: &str = r#"
hello world
$ exit
exit
[Process completed]
user@host:~/developer$
"#;

    const WORKING_PANE: &str = r#"
           Claude Code v2.1.114
 ▐▛███▜▌   Opus 4.7 (1M context)

❯ build the thing
✻ Sautéed for 2m 35s · ↓ ctrl+r to expand
                                        new task? /clear to save 12.3k tokens
"#;

    #[test]
    fn classifies_idle() {
        let h = classify(IDLE_PANE);
        match h {
            Health::Idle { last_message, tokens } => {
                assert_eq!(last_message.as_deref(), Some("make one more"));
                assert_eq!(tokens.as_deref(), Some("174.5k tokens"));
            }
            other => panic!("expected Idle, got {other:?}"),
        }
    }

    #[test]
    fn classifies_crashed() {
        let h = classify(CRASHED_PANE);
        assert!(matches!(h, Health::Crashed { .. }));
    }

    #[test]
    fn classifies_working() {
        let h = classify(WORKING_PANE);
        assert!(matches!(h, Health::Working { .. }));
    }

    #[test]
    fn empty_pane_is_unknown() {
        assert!(matches!(classify(""), Health::Unknown { .. }));
    }

    #[test]
    fn handles_non_breaking_space_after_prompt_glyph() {
        // The real Claude TUI uses U+00A0 (non-breaking space) after `❯`.
        let pane = "Claude Code v2\n▐▛███▜▌\n\n❯\u{00A0}try out the conductr\n  ⏵⏵ auto mode on (shift+tab to cycle)\n";
        match classify(pane) {
            Health::Idle { last_message, .. } => {
                assert_eq!(last_message.as_deref(), Some("try out the conductr"));
            }
            other => panic!("expected Idle, got {other:?}"),
        }
    }
}
