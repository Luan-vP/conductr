//! Classify each tmux session by what's currently rendered in its pane.
//!
//! The Claude Code TUI has a recognisable footprint: a banner, a `❯` prompt,
//! a status line ("auto mode on", "shift+tab to cycle"), and various spinner
//! frames when busy. We use those markers as evidence the agent is alive.
//! Absence of any Claude markers, with a shell prompt visible, means the
//! agent has exited and the session has fallen back to the shell.

use chrono::Utc;

use conductr_core::ports::TmuxAgent;
use conductr_core::types::{Diagnosis, Health, TmuxError, TmuxSession};

/// Run a diagnose pass over all live tmux sessions.
///
/// `pattern` is an optional substring filter on session name. Pass `None` to
/// inspect every session on the host.
pub async fn diagnose_all(tmux: &impl TmuxAgent, pattern: Option<&str>) -> Result<Vec<Diagnosis>, TmuxError> {
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
pub async fn diagnose_one(tmux: &impl TmuxAgent, name: &str) -> Result<Diagnosis, TmuxError> {
    let sessions = tmux.list_sessions().await?;
    let s = sessions
        .into_iter()
        .find(|s| s.name == name)
        .ok_or_else(|| TmuxError::Parse(format!("no such session: {name}")))?;
    diagnose_session(tmux, s).await
}

async fn diagnose_session(tmux: &impl TmuxAgent, session: TmuxSession) -> Result<Diagnosis, TmuxError> {
    let pane = tmux.capture_pane(&session.name).await?;
    let health = classify(&pane);
    let remote_control = remote_control_active(&pane);
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
    Ok(Diagnosis { session, health, idle_seconds, remote_control, tail })
}

/// True when Claude Code's Remote Control is active in the rendered pane.
///
/// An active session renders a banner (`/remote-control is active · Continue
/// here, on your phone, or at https://claude.ai/code/session_…`) and a compact
/// `/rc active` marker in the status footer. Either marker is sufficient.
pub fn remote_control_active(pane: &str) -> bool {
    pane.contains("/remote-control is active") || pane.contains("/rc active")
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
            .rfind(|l| !l.trim().is_empty())
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
    l.contains("auto mode on")
        || l.contains("tokens")
        || l.contains("Auto-update")
        || l.starts_with("⏵⏵")
        || l.chars().all(|c| matches!(c, '─' | '━' | '-' | ' '))
}

fn last_user_or_assistant_message(pane: &str) -> Option<String> {
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
    s.starts_with("Try \"") && s.ends_with('"')
}

fn token_count(pane: &str) -> Option<String> {
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

    const RC_ACTIVE_PANE: &str = r#"
           Claude Code v2.1.220
 ▐▛███▜▌   Sonnet 5 · Claude Pro
▝▜█████▛▘  /home/dev
  /remote-control is active · Continue here, on your phone, or at
  https://claude.ai/code/session_01EdD2D4JHuLKQZrfJjQPctc
────────────────────────────────────────────────────────────────────────────────
❯ make one more
────────────────────────────────────────────────────────────────────────────────
  ⏵⏵ auto mode on (shift+tab to cycle) · ← for agents         ● high · /effort
                                                                    /rc active
"#;

    #[test]
    fn detects_remote_control_active() {
        assert!(remote_control_active(RC_ACTIVE_PANE));
        // Footer marker alone is sufficient.
        assert!(remote_control_active("some pane\n  /rc active\n"));
    }

    #[test]
    fn detects_remote_control_inactive() {
        // IDLE_PANE has no remote-control markers.
        assert!(!remote_control_active(IDLE_PANE));
        assert!(!remote_control_active(""));
    }

    #[tokio::test]
    async fn mock_diagnose_reports_remote_control() {
        use conductr_adapters::mock::MockTmuxAgent;
        let mock = MockTmuxAgent::new()
            .with_session("rc-on", RC_ACTIVE_PANE)
            .with_session("rc-off", IDLE_PANE);
        let diagnoses = diagnose_all(&mock, None).await.unwrap();
        let on = diagnoses.iter().find(|d| d.session.name == "rc-on").unwrap();
        let off = diagnoses.iter().find(|d| d.session.name == "rc-off").unwrap();
        assert!(on.remote_control, "rc-on should report remote_control = true");
        assert!(!off.remote_control, "rc-off should report remote_control = false");
    }

    #[test]
    fn handles_non_breaking_space_after_prompt_glyph() {
        let pane = "Claude Code v2\n▐▛███▜▌\n\n❯\u{00A0}try out the conductr\n  ⏵⏵ auto mode on (shift+tab to cycle)\n";
        match classify(pane) {
            Health::Idle { last_message, .. } => {
                assert_eq!(last_message.as_deref(), Some("try out the conductr"));
            }
            other => panic!("expected Idle, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn mock_diagnose_all_crashed_pane() {
        use conductr_adapters::mock::MockTmuxAgent;
        let mock = MockTmuxAgent::new().with_session("s1", CRASHED_PANE);
        let diagnoses = diagnose_all(&mock, None).await.unwrap();
        assert_eq!(diagnoses.len(), 1);
        assert!(matches!(diagnoses[0].health, Health::Crashed { .. }));
    }

    #[tokio::test]
    async fn mock_diagnose_all_idle_pane() {
        use conductr_adapters::mock::MockTmuxAgent;
        let mock = MockTmuxAgent::new().with_session("s1", IDLE_PANE);
        let diagnoses = diagnose_all(&mock, None).await.unwrap();
        assert_eq!(diagnoses.len(), 1);
        assert!(matches!(diagnoses[0].health, Health::Idle { .. }));
    }

    #[tokio::test]
    async fn mock_diagnose_all_pattern_filter() {
        use conductr_adapters::mock::MockTmuxAgent;
        let mock = MockTmuxAgent::new()
            .with_session("claude-1", IDLE_PANE)
            .with_session("other", CRASHED_PANE);
        let diagnoses = diagnose_all(&mock, Some("claude")).await.unwrap();
        assert_eq!(diagnoses.len(), 1);
        assert_eq!(diagnoses[0].session.name, "claude-1");
    }

    #[tokio::test]
    async fn mock_diagnose_one_by_name() {
        use conductr_adapters::mock::MockTmuxAgent;
        let mock = MockTmuxAgent::new()
            .with_session("foo", IDLE_PANE)
            .with_session("bar", CRASHED_PANE);
        let d = diagnose_one(&mock, "bar").await.unwrap();
        assert!(matches!(d.health, Health::Crashed { .. }));
    }
}
