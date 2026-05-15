//! Runner determination and slot management for the tmux dispatch path.

use conductr_core::types::{Issue, TmuxSession};

/// Where a beat should be dispatched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Runner {
    /// Post `@claude` on the GitHub issue (GH Actions picks it up).
    Web,
    /// Spawn or reuse a local tmux `agent<n>` slot.
    Tmux,
}

/// Slot pool kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotKind {
    /// Implementation work: sessions named `agent1`, `agent2`, …
    Agent,
    /// QA / review work: sessions named `qa1`, `qa2`, …
    Qa,
}

impl SlotKind {
    pub fn prefix(self) -> &'static str {
        match self {
            SlotKind::Agent => "agent",
            SlotKind::Qa => "qa",
        }
    }
}

/// Canonical name for slot `n` of the given kind.
pub fn slot_name(kind: SlotKind, n: usize) -> String {
    format!("{}{n}", kind.prefix())
}

/// Determine which runner to use for `issue`.
///
/// Precedence: `runner:tmux` or `runner:web` label → default `web`.
pub fn runner_for(issue: &Issue) -> Runner {
    for label in &issue.labels {
        let l = label.trim().to_ascii_lowercase();
        if l == "runner:tmux" {
            return Runner::Tmux;
        }
        if l == "runner:web" {
            return Runner::Web;
        }
    }
    Runner::Web
}

/// 1-based indices of active slots matching `kind` prefix in `sessions`.
pub fn active_slot_indices(sessions: &[TmuxSession], kind: SlotKind) -> Vec<usize> {
    let prefix = kind.prefix();
    let mut indices: Vec<usize> = sessions
        .iter()
        .filter_map(|s| s.name.strip_prefix(prefix)?.parse::<usize>().ok())
        .collect();
    indices.sort_unstable();
    indices
}

/// Return the lowest free 1-based slot index within `1..=max`, or `None` if
/// all slots are occupied.
pub fn next_free_slot(sessions: &[TmuxSession], kind: SlotKind, max: usize) -> Option<usize> {
    let active = active_slot_indices(sessions, kind);
    (1..=max).find(|n| !active.contains(n))
}

/// Names of `agent<n>` sessions that are candidates for release during the
/// idle reconciliation sweep.
///
/// A slot is stale when the active agent-session count exceeds
/// `in_flight_count` (the number of open issues still carrying the
/// `conductr:in-flight` label). The highest-indexed slots are freed first.
pub fn stale_agent_pane_names(sessions: &[TmuxSession], in_flight_count: usize) -> Vec<String> {
    let prefix = SlotKind::Agent.prefix();
    let mut agent_sessions: Vec<&TmuxSession> = sessions
        .iter()
        .filter(|s| {
            s.name
                .strip_prefix(prefix)
                .and_then(|n| n.parse::<usize>().ok())
                .is_some()
        })
        .collect();

    // Sort descending so we free the highest-indexed slots first.
    agent_sessions.sort_by(|a, b| b.name.cmp(&a.name));

    let active_count = agent_sessions.len();
    if active_count <= in_flight_count {
        return Vec::new();
    }
    agent_sessions[..active_count - in_flight_count]
        .iter()
        .map(|s| s.name.clone())
        .collect()
}

/// Names of `qa<n>` sessions that are candidates for release.
///
/// A qa slot is stale when the active qa-session count exceeds
/// `open_pr_count` (the number of open PRs that need QA coverage).
pub fn stale_qa_pane_names(sessions: &[TmuxSession], open_pr_count: usize) -> Vec<String> {
    let prefix = SlotKind::Qa.prefix();
    let mut qa_sessions: Vec<&TmuxSession> = sessions
        .iter()
        .filter(|s| {
            s.name
                .strip_prefix(prefix)
                .and_then(|n| n.parse::<usize>().ok())
                .is_some()
        })
        .collect();

    qa_sessions.sort_by(|a, b| b.name.cmp(&a.name));

    let active_count = qa_sessions.len();
    if active_count <= open_pr_count {
        return Vec::new();
    }
    qa_sessions[..active_count - open_pr_count]
        .iter()
        .map(|s| s.name.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use conductr_core::types::{Issue, IssueState, TmuxSession};

    fn make_session(name: &str) -> TmuxSession {
        TmuxSession {
            name: name.to_string(),
            created: Utc::now(),
            last_activity: Utc::now(),
            windows: 1,
            attached: false,
            cwd: None,
        }
    }

    fn make_issue(labels: &[&str]) -> Issue {
        Issue {
            number: 1,
            title: "test".into(),
            body: String::new(),
            labels: labels.iter().map(|s| s.to_string()).collect(),
            state: IssueState::Open,
        }
    }

    #[test]
    fn runner_default_is_web() {
        assert_eq!(runner_for(&make_issue(&[])), Runner::Web);
    }

    #[test]
    fn runner_tmux_label_detected() {
        assert_eq!(runner_for(&make_issue(&["runner:tmux"])), Runner::Tmux);
    }

    #[test]
    fn runner_web_label_detected() {
        assert_eq!(runner_for(&make_issue(&["runner:web"])), Runner::Web);
    }

    #[test]
    fn runner_label_case_insensitive() {
        assert_eq!(runner_for(&make_issue(&["Runner:Tmux"])), Runner::Tmux);
    }

    #[test]
    fn runner_tmux_takes_precedence_in_order() {
        // First matching label wins.
        assert_eq!(runner_for(&make_issue(&["runner:tmux", "runner:web"])), Runner::Tmux);
    }

    #[test]
    fn slot_name_agent() {
        assert_eq!(slot_name(SlotKind::Agent, 1), "agent1");
        assert_eq!(slot_name(SlotKind::Agent, 3), "agent3");
    }

    #[test]
    fn slot_name_qa() {
        assert_eq!(slot_name(SlotKind::Qa, 2), "qa2");
    }

    #[test]
    fn active_slot_indices_empty_on_unrelated_sessions() {
        let sessions = vec![make_session("conductr"), make_session("other")];
        assert!(active_slot_indices(&sessions, SlotKind::Agent).is_empty());
    }

    #[test]
    fn active_slot_indices_found() {
        let sessions = vec![
            make_session("agent1"),
            make_session("agent3"),
            make_session("qa1"),
        ];
        assert_eq!(active_slot_indices(&sessions, SlotKind::Agent), vec![1, 3]);
        assert_eq!(active_slot_indices(&sessions, SlotKind::Qa), vec![1]);
    }

    #[test]
    fn next_free_slot_first_when_empty() {
        let sessions: Vec<TmuxSession> = vec![];
        assert_eq!(next_free_slot(&sessions, SlotKind::Agent, 3), Some(1));
    }

    #[test]
    fn next_free_slot_fills_gap() {
        let sessions = vec![make_session("agent1"), make_session("agent3")];
        assert_eq!(next_free_slot(&sessions, SlotKind::Agent, 3), Some(2));
    }

    #[test]
    fn next_free_slot_none_when_full() {
        let sessions = vec![
            make_session("agent1"),
            make_session("agent2"),
            make_session("agent3"),
        ];
        assert_eq!(next_free_slot(&sessions, SlotKind::Agent, 3), None);
    }

    #[test]
    fn stale_agent_panes_none_when_at_or_below_in_flight() {
        let sessions = vec![make_session("agent1"), make_session("agent2")];
        assert!(stale_agent_pane_names(&sessions, 2).is_empty());
        assert!(stale_agent_pane_names(&sessions, 3).is_empty());
    }

    #[test]
    fn stale_agent_panes_excess_returned() {
        let sessions = vec![
            make_session("agent1"),
            make_session("agent2"),
            make_session("agent3"),
        ];
        // 2 in-flight → 1 stale (the highest-indexed)
        let stale = stale_agent_pane_names(&sessions, 2);
        assert_eq!(stale.len(), 1);
    }

    #[test]
    fn stale_agent_panes_ignores_non_agent_sessions() {
        let sessions = vec![
            make_session("conductr"),
            make_session("qa1"),
            make_session("agent1"),
        ];
        // 0 in-flight → only agent1 is stale, not qa1 or conductr
        let stale = stale_agent_pane_names(&sessions, 0);
        assert_eq!(stale, vec!["agent1"]);
    }

    #[test]
    fn stale_qa_panes_excess_returned() {
        let sessions = vec![make_session("qa1"), make_session("qa2")];
        let stale = stale_qa_pane_names(&sessions, 1);
        assert_eq!(stale.len(), 1);
    }
}
