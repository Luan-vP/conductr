//! Merge gate: per-detent predicate deciding whether a branch is shippable.
//!
//! The five detents, from most permissive to most restrictive:
//!
//! | Detent       | Merge predicate                                                         |
//! |--------------|-------------------------------------------------------------------------|
//! | UNHINGED     | Skip CI entirely; merge on push                                         |
//! | FERAL        | Require ≥1 required check green; optionals ignored                      |
//! | FAST         | Require all required checks green; advisory amber tolerated             |
//! | STRICT       | Require full green including advisory; one flake-retry on transient failures |
//! | BUREAUCRATIC | Full green + human review approved + linked issue closed + ADR present  |

use conductr_core::types::{
    BlockReason, Branch, CheckStatus, MergeDecision, RetryReason, SafetyPreset,
};

/// Decide whether `branch` is ready to merge under the given `preset`.
///
/// Returns a structured [`MergeDecision`]: allowed, blocked by a specific
/// reason, or retry-once for a transient infrastructure failure.
pub fn can_merge(branch: &Branch, preset: SafetyPreset) -> MergeDecision {
    match preset {
        // ── UNHINGED: skip CI entirely ─────────────────────────────────────────
        SafetyPreset::Unhinged => MergeDecision::Allowed,

        // ── FERAL: ≥1 required check green; optionals ignored ─────────────────
        SafetyPreset::Feral => {
            let required: Vec<_> = branch.checks.iter().filter(|c| c.required).collect();
            if required.is_empty() || required.iter().any(|c| c.status == CheckStatus::Green) {
                MergeDecision::Allowed
            } else {
                MergeDecision::BlockedBy(BlockReason::NoRequiredCheckGreen)
            }
        }

        // ── FAST: all required green; advisory amber tolerated ─────────────────
        SafetyPreset::Fast => {
            for check in branch.checks.iter().filter(|c| c.required) {
                if check.status != CheckStatus::Green {
                    return MergeDecision::BlockedBy(BlockReason::RequiredCheckFailed(
                        check.name.clone(),
                    ));
                }
            }
            MergeDecision::Allowed
        }

        // ── STRICT: full green including advisory; one transient retry ─────────
        SafetyPreset::Strict => strict_predicate(branch),

        // ── BUREAUCRATIC: full green + review + linked issue closed + ADR ──────
        SafetyPreset::Bureaucratic => {
            // CI checks evaluated before procedural gates so CI failures surface
            // before asking humans for sign-off that will be invalidated anyway.
            for check in &branch.checks {
                if check.status != CheckStatus::Green {
                    let reason = if check.required {
                        BlockReason::RequiredCheckFailed(check.name.clone())
                    } else {
                        BlockReason::AdvisoryCheckFailed(check.name.clone())
                    };
                    return MergeDecision::BlockedBy(reason);
                }
            }
            if !branch.review_approved {
                return MergeDecision::BlockedBy(BlockReason::ReviewNotApproved);
            }
            if !branch.linked_issue_closed {
                return MergeDecision::BlockedBy(BlockReason::LinkedIssueNotClosed);
            }
            if !branch.adr_present {
                return MergeDecision::BlockedBy(BlockReason::AdrNotPresent);
            }
            MergeDecision::Allowed
        }
    }
}

/// STRICT predicate: all checks (required + advisory) must be green.
///
/// Hard failures (Red, Amber, Pending) block immediately and take priority over
/// transient signals. A single transient infrastructure failure (timeout, runner
/// error) triggers one retry; test-assertion failures (Red) never retry.
fn strict_predicate(branch: &Branch) -> MergeDecision {
    let mut transient_check: Option<String> = None;

    for check in &branch.checks {
        match check.status {
            CheckStatus::Green => {}
            CheckStatus::Transient => {
                // Record first transient; keep scanning for hard failures.
                if transient_check.is_none() {
                    transient_check = Some(check.name.clone());
                }
            }
            // Red, Amber, or Pending: hard block; not retriable.
            _ => {
                let reason = if check.required {
                    BlockReason::RequiredCheckFailed(check.name.clone())
                } else {
                    BlockReason::AdvisoryCheckFailed(check.name.clone())
                };
                return MergeDecision::BlockedBy(reason);
            }
        }
    }

    if let Some(name) = transient_check {
        return MergeDecision::RetryOnce(RetryReason { check_name: name });
    }

    MergeDecision::Allowed
}

#[cfg(test)]
mod tests {
    use super::can_merge;
    use conductr_core::types::{
        BlockReason, Branch, Check, CheckStatus, MergeDecision, RetryReason, SafetyPreset,
    };

    fn check(name: &str, status: CheckStatus, required: bool) -> Check {
        Check { name: name.into(), status, required }
    }

    fn branch(checks: Vec<Check>) -> Branch {
        Branch {
            head_ref: "claude/issue-174-test".into(),
            checks,
            review_approved: false,
            linked_issue_closed: false,
            adr_present: false,
        }
    }

    // ── UNHINGED ─────────────────────────────────────────────────────────────

    #[test]
    fn unhinged_allows_despite_failing_checks() {
        let b = branch(vec![check("ci", CheckStatus::Red, true)]);
        assert_eq!(can_merge(&b, SafetyPreset::Unhinged), MergeDecision::Allowed);
    }

    #[test]
    fn unhinged_allows_empty_suite() {
        let b = branch(vec![]);
        assert_eq!(can_merge(&b, SafetyPreset::Unhinged), MergeDecision::Allowed);
    }

    // ── FERAL ─────────────────────────────────────────────────────────────────

    #[test]
    fn feral_allows_when_one_required_green() {
        let b = branch(vec![
            check("ci-a", CheckStatus::Green, true),
            check("ci-b", CheckStatus::Red, true),
        ]);
        assert_eq!(can_merge(&b, SafetyPreset::Feral), MergeDecision::Allowed);
    }

    #[test]
    fn feral_blocks_when_no_required_green() {
        let b = branch(vec![
            check("ci-a", CheckStatus::Red, true),
            check("ci-b", CheckStatus::Pending, true),
        ]);
        assert_eq!(
            can_merge(&b, SafetyPreset::Feral),
            MergeDecision::BlockedBy(BlockReason::NoRequiredCheckGreen),
        );
    }

    #[test]
    fn feral_ignores_failing_optional_checks() {
        let b = branch(vec![
            check("required", CheckStatus::Green, true),
            check("optional", CheckStatus::Red, false),
        ]);
        assert_eq!(can_merge(&b, SafetyPreset::Feral), MergeDecision::Allowed);
    }

    #[test]
    fn feral_allows_when_no_required_checks_exist() {
        let b = branch(vec![check("optional", CheckStatus::Red, false)]);
        assert_eq!(can_merge(&b, SafetyPreset::Feral), MergeDecision::Allowed);
    }

    // ── FAST ──────────────────────────────────────────────────────────────────

    #[test]
    fn fast_allows_when_all_required_green() {
        let b = branch(vec![
            check("ci", CheckStatus::Green, true),
            check("lint", CheckStatus::Amber, false),
        ]);
        assert_eq!(can_merge(&b, SafetyPreset::Fast), MergeDecision::Allowed);
    }

    #[test]
    fn fast_blocks_on_required_red() {
        let b = branch(vec![check("ci", CheckStatus::Red, true)]);
        assert_eq!(
            can_merge(&b, SafetyPreset::Fast),
            MergeDecision::BlockedBy(BlockReason::RequiredCheckFailed("ci".into())),
        );
    }

    #[test]
    fn fast_blocks_on_required_pending() {
        let b = branch(vec![check("ci", CheckStatus::Pending, true)]);
        assert_eq!(
            can_merge(&b, SafetyPreset::Fast),
            MergeDecision::BlockedBy(BlockReason::RequiredCheckFailed("ci".into())),
        );
    }

    #[test]
    fn fast_blocks_on_required_transient() {
        let b = branch(vec![check("ci", CheckStatus::Transient, true)]);
        assert_eq!(
            can_merge(&b, SafetyPreset::Fast),
            MergeDecision::BlockedBy(BlockReason::RequiredCheckFailed("ci".into())),
        );
    }

    #[test]
    fn fast_tolerates_advisory_amber() {
        let b = branch(vec![
            check("required", CheckStatus::Green, true),
            check("lint", CheckStatus::Amber, false),
        ]);
        assert_eq!(can_merge(&b, SafetyPreset::Fast), MergeDecision::Allowed);
    }

    // ── STRICT ────────────────────────────────────────────────────────────────

    #[test]
    fn strict_allows_when_all_green() {
        let b = branch(vec![
            check("ci", CheckStatus::Green, true),
            check("lint", CheckStatus::Green, false),
        ]);
        assert_eq!(can_merge(&b, SafetyPreset::Strict), MergeDecision::Allowed);
    }

    #[test]
    fn strict_blocks_on_advisory_amber() {
        let b = branch(vec![
            check("ci", CheckStatus::Green, true),
            check("lint", CheckStatus::Amber, false),
        ]);
        assert_eq!(
            can_merge(&b, SafetyPreset::Strict),
            MergeDecision::BlockedBy(BlockReason::AdvisoryCheckFailed("lint".into())),
        );
    }

    #[test]
    fn strict_retries_on_transient_required() {
        let b = branch(vec![
            check("ci", CheckStatus::Green, true),
            check("flaky-runner", CheckStatus::Transient, true),
        ]);
        assert_eq!(
            can_merge(&b, SafetyPreset::Strict),
            MergeDecision::RetryOnce(RetryReason { check_name: "flaky-runner".into() }),
        );
    }

    #[test]
    fn strict_retries_on_transient_advisory() {
        let b = branch(vec![
            check("ci", CheckStatus::Green, true),
            check("infra-check", CheckStatus::Transient, false),
        ]);
        assert_eq!(
            can_merge(&b, SafetyPreset::Strict),
            MergeDecision::RetryOnce(RetryReason { check_name: "infra-check".into() }),
        );
    }

    #[test]
    fn strict_hard_failure_takes_priority_over_transient() {
        // Red check appears after transient in the list — hard failure still wins.
        let b = branch(vec![
            check("flaky", CheckStatus::Transient, true),
            check("tests", CheckStatus::Red, true),
        ]);
        assert_eq!(
            can_merge(&b, SafetyPreset::Strict),
            MergeDecision::BlockedBy(BlockReason::RequiredCheckFailed("tests".into())),
        );
    }

    #[test]
    fn strict_does_not_retry_test_assertion_failure() {
        // Red = logic/test-assertion failure; must never trigger retry.
        let b = branch(vec![check("unit-tests", CheckStatus::Red, true)]);
        assert!(matches!(
            can_merge(&b, SafetyPreset::Strict),
            MergeDecision::BlockedBy(BlockReason::RequiredCheckFailed(_)),
        ));
    }

    #[test]
    fn strict_does_not_retry_amber() {
        // Amber is a hard failure for STRICT; must not retry.
        let b = branch(vec![check("coverage", CheckStatus::Amber, false)]);
        assert!(matches!(
            can_merge(&b, SafetyPreset::Strict),
            MergeDecision::BlockedBy(BlockReason::AdvisoryCheckFailed(_)),
        ));
    }

    // ── BUREAUCRATIC ──────────────────────────────────────────────────────────

    #[test]
    fn bureaucratic_allows_when_fully_compliant() {
        let b = Branch {
            head_ref: "feat/174".into(),
            checks: vec![
                check("ci", CheckStatus::Green, true),
                check("lint", CheckStatus::Green, false),
            ],
            review_approved: true,
            linked_issue_closed: true,
            adr_present: true,
        };
        assert_eq!(can_merge(&b, SafetyPreset::Bureaucratic), MergeDecision::Allowed);
    }

    #[test]
    fn bureaucratic_blocks_failing_ci_before_procedural_gates() {
        let b = Branch {
            head_ref: "feat/174".into(),
            checks: vec![check("ci", CheckStatus::Red, true)],
            review_approved: true,
            linked_issue_closed: true,
            adr_present: true,
        };
        assert_eq!(
            can_merge(&b, SafetyPreset::Bureaucratic),
            MergeDecision::BlockedBy(BlockReason::RequiredCheckFailed("ci".into())),
        );
    }

    #[test]
    fn bureaucratic_blocks_missing_review() {
        let b = Branch {
            head_ref: "feat/174".into(),
            checks: vec![check("ci", CheckStatus::Green, true)],
            review_approved: false,
            linked_issue_closed: true,
            adr_present: true,
        };
        assert_eq!(
            can_merge(&b, SafetyPreset::Bureaucratic),
            MergeDecision::BlockedBy(BlockReason::ReviewNotApproved),
        );
    }

    #[test]
    fn bureaucratic_blocks_open_linked_issue() {
        let b = Branch {
            head_ref: "feat/174".into(),
            checks: vec![check("ci", CheckStatus::Green, true)],
            review_approved: true,
            linked_issue_closed: false,
            adr_present: true,
        };
        assert_eq!(
            can_merge(&b, SafetyPreset::Bureaucratic),
            MergeDecision::BlockedBy(BlockReason::LinkedIssueNotClosed),
        );
    }

    #[test]
    fn bureaucratic_blocks_missing_adr() {
        let b = Branch {
            head_ref: "feat/174".into(),
            checks: vec![check("ci", CheckStatus::Green, true)],
            review_approved: true,
            linked_issue_closed: true,
            adr_present: false,
        };
        assert_eq!(
            can_merge(&b, SafetyPreset::Bureaucratic),
            MergeDecision::BlockedBy(BlockReason::AdrNotPresent),
        );
    }

    #[test]
    fn bureaucratic_blocks_advisory_amber() {
        let b = Branch {
            head_ref: "feat/174".into(),
            checks: vec![
                check("ci", CheckStatus::Green, true),
                check("coverage", CheckStatus::Amber, false),
            ],
            review_approved: true,
            linked_issue_closed: true,
            adr_present: true,
        };
        assert_eq!(
            can_merge(&b, SafetyPreset::Bureaucratic),
            MergeDecision::BlockedBy(BlockReason::AdvisoryCheckFailed("coverage".into())),
        );
    }
}
