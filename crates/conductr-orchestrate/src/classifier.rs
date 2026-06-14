//! Bucket each open issue per the SKILL.md rules.

use std::collections::BTreeSet;

use crate::deps::parse_dependencies;
use crate::types::{CiStatus, Issue, IssueNumber, IssueState, Pr, PrState};

pub use conductr_core::types::{Bucket, Classification};

/// Classify a single issue given the surrounding state.
///
/// `closed_issues` is the set of issue numbers known to be closed/merged.
/// `prs` are *all* open PRs in the repo; we filter by linked issue / branch.
/// `triggered` is the set of issues that have been commented `@claude please implement`.
pub fn classify(
    issue: &Issue,
    closed_issues: &BTreeSet<IssueNumber>,
    prs: &[Pr],
    triggered: &BTreeSet<IssueNumber>,
) -> Classification {
    if issue.state == IssueState::Closed {
        return Classification {
            issue: issue.number,
            bucket: Bucket::AlreadyClosed,
            blocking: Vec::new(),
            pr: None,
        };
    }

    if issue.is_human() {
        return Classification {
            issue: issue.number,
            bucket: Bucket::Human,
            blocking: Vec::new(),
            pr: None,
        };
    }

    let deps: BTreeSet<IssueNumber> = parse_dependencies(&issue.body)
        .into_iter()
        .map(|d| d.0)
        .collect();
    let blocking: Vec<IssueNumber> = deps
        .iter()
        .copied()
        .filter(|d| !closed_issues.contains(d))
        .collect();

    let linked_pr = prs
        .iter()
        .find(|p| {
            p.state == PrState::Open
                && (p.linked_issue == Some(issue.number)
                    || p.head_ref.starts_with(&format!("claude/issue-{}-", issue.number)))
        });

    if let Some(pr) = linked_pr {
        let bucket = match pr.ci {
            CiStatus::Failing => Bucket::PrFailing,
            _ => Bucket::PrOpen,
        };
        return Classification {
            issue: issue.number,
            bucket,
            blocking,
            pr: Some(pr.number),
        };
    }

    if !blocking.is_empty() {
        return Classification {
            issue: issue.number,
            bucket: Bucket::Blocked,
            blocking,
            pr: None,
        };
    }

    if triggered.contains(&issue.number) {
        return Classification {
            issue: issue.number,
            bucket: Bucket::TriggeredWaiting,
            blocking,
            pr: None,
        };
    }

    Classification {
        issue: issue.number,
        bucket: Bucket::Ready,
        blocking,
        pr: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issue(n: IssueNumber, body: &str, labels: &[&str]) -> Issue {
        Issue {
            number: n,
            title: format!("issue {n}"),
            body: body.into(),
            labels: labels.iter().map(|s| s.to_string()).collect(),
            state: IssueState::Open,
        }
    }

    #[test]
    fn ready_when_no_deps_no_pr() {
        let i = issue(1, "no deps here", &[]);
        let c = classify(&i, &BTreeSet::new(), &[], &BTreeSet::new());
        assert_eq!(c.bucket, Bucket::Ready);
    }

    #[test]
    fn human_label_short_circuits() {
        let i = issue(1, "depends on #2", &["human"]);
        let c = classify(&i, &BTreeSet::new(), &[], &BTreeSet::new());
        assert_eq!(c.bucket, Bucket::Human);
    }

    #[test]
    fn blocked_by_open_dep() {
        let i = issue(1, "depends on #2", &[]);
        let c = classify(&i, &BTreeSet::new(), &[], &BTreeSet::new());
        assert_eq!(c.bucket, Bucket::Blocked);
        assert_eq!(c.blocking, vec![2]);
    }

    #[test]
    fn unblocked_when_dep_already_closed() {
        let i = issue(1, "depends on #2", &[]);
        let mut closed = BTreeSet::new();
        closed.insert(2);
        let c = classify(&i, &closed, &[], &BTreeSet::new());
        assert_eq!(c.bucket, Bucket::Ready);
    }

    #[test]
    fn pr_open_with_passing_ci() {
        let i = issue(1, "", &[]);
        let pr = Pr {
            number: 100,
            title: "PR".into(),
            body: "fixes #1".into(),
            head_ref: "claude/issue-1-foo".into(),
            state: PrState::Open,
            ci: CiStatus::Passing,
            linked_issue: Some(1),
            is_fork: false,
        };
        let c = classify(&i, &BTreeSet::new(), &[pr], &BTreeSet::new());
        assert_eq!(c.bucket, Bucket::PrOpen);
        assert_eq!(c.pr, Some(100));
    }

    #[test]
    fn pr_open_with_failing_ci() {
        let i = issue(1, "", &[]);
        let pr = Pr {
            number: 100,
            title: "PR".into(),
            body: "".into(),
            head_ref: "claude/issue-1-foo".into(),
            state: PrState::Open,
            ci: CiStatus::Failing,
            linked_issue: Some(1),
            is_fork: false,
        };
        let c = classify(&i, &BTreeSet::new(), &[pr], &BTreeSet::new());
        assert_eq!(c.bucket, Bucket::PrFailing);
    }

    #[test]
    fn triggered_waiting_when_no_pr_yet() {
        let i = issue(1, "", &[]);
        let mut triggered = BTreeSet::new();
        triggered.insert(1);
        let c = classify(&i, &BTreeSet::new(), &[], &triggered);
        assert_eq!(c.bucket, Bucket::TriggeredWaiting);
    }

    #[test]
    fn scope_overlap_variant_is_not_ready() {
        let bucket = Bucket::ScopeOverlap { existing_message_id: "msg-1".into() };
        assert_ne!(bucket, Bucket::Ready);
    }
}
