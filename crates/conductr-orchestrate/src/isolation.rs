//! Per-detent branch-isolation logic for the routine orchestrator.
//!
//! Translates a `SafetyPreset` + current sibling-branch state into a concrete
//! `IsolationDecision` that the orchestrator acts on when deciding whether (and
//! how) to dispatch a Ready issue.

use std::time::Instant;

use conductr_core::signals::MailKind;
use conductr_core::types::{CiStatus, IssueNumber, Pr, SafetyPreset, SiblingStatus};

/// Decision returned by [`decide`] for each Ready issue.
#[derive(Debug)]
pub enum IsolationDecision {
    /// Dispatch immediately — no coordination check needed or already satisfied.
    Dispatch,
    /// Defer this cycle; re-evaluate on the next pass.
    Defer,
    /// Dispatch and emit the listed `MailKind::Yell` events (FERAL).
    DispatchWithYell(Vec<MailKind>),
    /// Dispatch and post an advisory comment on the issue (FAST).
    DispatchWithAdvisory(String),
}

/// Classify a single PR's CI status into a [`SiblingStatus`] bucket.
pub fn classify_sibling_ci(ci: CiStatus) -> SiblingStatus {
    match ci {
        CiStatus::Passing => SiblingStatus::Green,
        CiStatus::Pending | CiStatus::Unknown => SiblingStatus::Amber,
        CiStatus::Failing => SiblingStatus::Red,
    }
}

/// Aggregate multiple sibling statuses into one overall [`SiblingStatus`].
/// Red beats amber beats green.
pub fn aggregate_sibling_status(siblings: &[&Pr]) -> SiblingStatus {
    let mut worst = SiblingStatus::Green;
    for p in siblings {
        match classify_sibling_ci(p.ci) {
            SiblingStatus::Red => return SiblingStatus::Red,
            SiblingStatus::Amber => worst = SiblingStatus::Amber,
            SiblingStatus::Green => {}
        }
    }
    worst
}

/// Return the subset of `open_prs` that are *not* linked to `issue`.
pub fn sibling_prs<'a>(issue: IssueNumber, open_prs: &'a [Pr]) -> Vec<&'a Pr> {
    open_prs
        .iter()
        .filter(|p| p.linked_issue != Some(issue))
        .collect()
}

/// Decide how to dispatch `issue` given the current `preset` and sibling state.
///
/// * `soft_chord_first_seen` — when the issue was first deferred by the
///   soft-chord (used to track timeout for `STRICT`).
/// * `soft_chord_timeout` — how long to wait before dispatching anyway.
pub fn decide(
    preset: SafetyPreset,
    issue: IssueNumber,
    siblings: &[&Pr],
    soft_chord_first_seen: Option<Instant>,
    soft_chord_timeout: std::time::Duration,
) -> IsolationDecision {
    match preset {
        // ── UNHINGED ───────────────────────────────────────────────────────────
        // Solo branch; siblings are invisible.
        SafetyPreset::Unhinged => IsolationDecision::Dispatch,

        // ── FERAL ──────────────────────────────────────────────────────────────
        // Solo branch; emit a structured Yell for every sibling but don't block.
        SafetyPreset::Feral => {
            if siblings.is_empty() {
                return IsolationDecision::Dispatch;
            }
            let yells = siblings
                .iter()
                .map(|p| MailKind::Yell {
                    issue,
                    sibling_branch: p.head_ref.clone(),
                    message: format!(
                        "Potential post-merge conflict: sibling branch '{}' is open (ci: {:?})",
                        p.head_ref, p.ci
                    ),
                })
                .collect();
            IsolationDecision::DispatchWithYell(yells)
        }

        // ── FAST ───────────────────────────────────────────────────────────────
        // Read-only sibling awareness; post an advisory and proceed.
        SafetyPreset::Fast => {
            if siblings.is_empty() {
                return IsolationDecision::Dispatch;
            }
            let names: Vec<&str> = siblings.iter().map(|p| p.head_ref.as_str()).collect();
            let advisory = format!(
                "Advisory: {} sibling branch(es) are open and may overlap with your work: {}. \
                 This is informational only — your routine will proceed normally.",
                names.len(),
                names.join(", ")
            );
            IsolationDecision::DispatchWithAdvisory(advisory)
        }

        // ── STRICT ─────────────────────────────────────────────────────────────
        // Soft-chord: await green siblings; dispatch if amber after timeout or if red.
        SafetyPreset::Strict => {
            if siblings.is_empty() {
                return IsolationDecision::Dispatch;
            }
            match aggregate_sibling_status(siblings) {
                SiblingStatus::Green => IsolationDecision::Dispatch,
                SiblingStatus::Red => {
                    // Red siblings are unhealthy; skip coordination and proceed.
                    IsolationDecision::Dispatch
                }
                SiblingStatus::Amber => {
                    // Siblings are pending. Check timeout.
                    if let Some(first_seen) = soft_chord_first_seen {
                        if first_seen.elapsed() >= soft_chord_timeout {
                            return IsolationDecision::Dispatch;
                        }
                    }
                    IsolationDecision::Defer
                }
            }
        }

        // ── BUREAUCRATIC ───────────────────────────────────────────────────────
        // Hard-chord: the orchestrator serialises dispatch via its coordinator
        // semaphore. The decision itself is always Dispatch; coordination is
        // enforced at the call site by acquiring the semaphore before acting.
        SafetyPreset::Bureaucratic => IsolationDecision::Dispatch,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use conductr_core::types::{PrState, PrNumber};

    fn make_pr(number: PrNumber, head_ref: &str, ci: CiStatus) -> Pr {
        conductr_core::types::Pr {
            number,
            title: format!("PR {number}"),
            body: String::new(),
            head_ref: head_ref.to_string(),
            state: PrState::Open,
            ci,
            linked_issue: None,
            is_fork: false,
        }
    }

    #[test]
    fn unhinged_always_dispatches() {
        let sibling = make_pr(1, "claude/issue-2-foo", CiStatus::Pending);
        let siblings = vec![&sibling];
        assert!(matches!(
            decide(SafetyPreset::Unhinged, 99, &siblings, None, std::time::Duration::from_secs(10)),
            IsolationDecision::Dispatch
        ));
    }

    #[test]
    fn feral_no_siblings_dispatches() {
        assert!(matches!(
            decide(SafetyPreset::Feral, 1, &[], None, std::time::Duration::from_secs(10)),
            IsolationDecision::Dispatch
        ));
    }

    #[test]
    fn feral_with_siblings_dispatches_with_yell() {
        let sibling = make_pr(2, "claude/issue-2-bar", CiStatus::Passing);
        let siblings = vec![&sibling];
        let decision = decide(SafetyPreset::Feral, 1, &siblings, None, std::time::Duration::from_secs(10));
        assert!(matches!(decision, IsolationDecision::DispatchWithYell(_)));
        if let IsolationDecision::DispatchWithYell(yells) = decision {
            assert_eq!(yells.len(), 1);
            assert!(matches!(
                &yells[0],
                MailKind::Yell { issue: 1, sibling_branch, .. } if sibling_branch == "claude/issue-2-bar"
            ));
        }
    }

    #[test]
    fn fast_no_siblings_dispatches() {
        assert!(matches!(
            decide(SafetyPreset::Fast, 1, &[], None, std::time::Duration::from_secs(10)),
            IsolationDecision::Dispatch
        ));
    }

    #[test]
    fn fast_with_siblings_dispatches_with_advisory() {
        let sibling = make_pr(2, "claude/issue-2-baz", CiStatus::Failing);
        let siblings = vec![&sibling];
        assert!(matches!(
            decide(SafetyPreset::Fast, 1, &siblings, None, std::time::Duration::from_secs(10)),
            IsolationDecision::DispatchWithAdvisory(_)
        ));
    }

    #[test]
    fn strict_green_siblings_dispatches() {
        let sibling = make_pr(2, "claude/issue-2-foo", CiStatus::Passing);
        let siblings = vec![&sibling];
        assert!(matches!(
            decide(SafetyPreset::Strict, 1, &siblings, None, std::time::Duration::from_secs(10)),
            IsolationDecision::Dispatch
        ));
    }

    #[test]
    fn strict_amber_siblings_defers() {
        let sibling = make_pr(2, "claude/issue-2-foo", CiStatus::Pending);
        let siblings = vec![&sibling];
        assert!(matches!(
            decide(SafetyPreset::Strict, 1, &siblings, None, std::time::Duration::from_secs(10)),
            IsolationDecision::Defer
        ));
    }

    #[test]
    fn strict_amber_siblings_dispatches_after_timeout() {
        let sibling = make_pr(2, "claude/issue-2-foo", CiStatus::Pending);
        let siblings = vec![&sibling];
        // first_seen is in the past, beyond the timeout
        let first_seen = Instant::now() - std::time::Duration::from_secs(20);
        assert!(matches!(
            decide(SafetyPreset::Strict, 1, &siblings, Some(first_seen), std::time::Duration::from_secs(10)),
            IsolationDecision::Dispatch
        ));
    }

    #[test]
    fn strict_red_siblings_skips_coordination() {
        let sibling = make_pr(2, "claude/issue-2-foo", CiStatus::Failing);
        let siblings = vec![&sibling];
        assert!(matches!(
            decide(SafetyPreset::Strict, 1, &siblings, None, std::time::Duration::from_secs(10)),
            IsolationDecision::Dispatch
        ));
    }

    #[test]
    fn bureaucratic_always_dispatches() {
        let sibling = make_pr(2, "claude/issue-2-foo", CiStatus::Pending);
        let siblings = vec![&sibling];
        assert!(matches!(
            decide(SafetyPreset::Bureaucratic, 1, &siblings, None, std::time::Duration::from_secs(10)),
            IsolationDecision::Dispatch
        ));
    }

    #[test]
    fn sibling_prs_excludes_own_pr() {
        let own = make_pr(100, "claude/issue-1-fix", CiStatus::Passing);
        let own_linked = conductr_core::types::Pr {
            linked_issue: Some(1),
            ..own
        };
        let other = make_pr(200, "claude/issue-2-feat", CiStatus::Pending);
        let prs = vec![own_linked, other];
        let siblings = sibling_prs(1, &prs);
        assert_eq!(siblings.len(), 1);
        assert_eq!(siblings[0].number, 200);
    }

    #[test]
    fn aggregate_sibling_status_red_beats_amber() {
        let pr1 = make_pr(1, "a", CiStatus::Pending);
        let pr2 = make_pr(2, "b", CiStatus::Failing);
        assert_eq!(aggregate_sibling_status(&[&pr1, &pr2]), SiblingStatus::Red);
    }

    #[test]
    fn aggregate_sibling_status_amber_beats_green() {
        let pr1 = make_pr(1, "a", CiStatus::Passing);
        let pr2 = make_pr(2, "b", CiStatus::Unknown);
        assert_eq!(aggregate_sibling_status(&[&pr1, &pr2]), SiblingStatus::Amber);
    }

    #[test]
    fn aggregate_sibling_status_all_green() {
        let pr1 = make_pr(1, "a", CiStatus::Passing);
        let pr2 = make_pr(2, "b", CiStatus::Passing);
        assert_eq!(aggregate_sibling_status(&[&pr1, &pr2]), SiblingStatus::Green);
    }
}
