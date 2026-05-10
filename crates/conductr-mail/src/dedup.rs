//! Scope-dedup: scan the mailbox for existing `ScopeClaim` messages that
//! overlap with a candidate set of files for a given issue.

use conductr_core::ports::Mailbox;
use conductr_core::types::{IssueNumber, MailKind, MailRef};

/// Outcome of a scope-dedup check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeReport {
    /// No overlapping claim was found; the caller may proceed.
    Clear,
    /// An existing claim overlaps with the candidate files.
    Overlap {
        /// The id of the first conflicting `ScopeClaim` message.
        existing_message_id: MailRef,
        /// The subset of candidate files that are already claimed.
        conflicting_files: Vec<String>,
    },
}

/// Check whether any existing `ScopeClaim` in the mailbox overlaps with
/// `candidate_files` for `issue`.
///
/// Returns `ScopeReport::Overlap` on the first match found.
pub async fn check_scope(
    mailbox: &impl Mailbox,
    issue: IssueNumber,
    candidate_files: &[String],
) -> ScopeReport {
    let messages = match mailbox.inbox(Some("scope_claim"), None).await {
        Ok(m) => m,
        Err(_) => return ScopeReport::Clear,
    };

    for msg in messages {
        if let MailKind::ScopeClaim { issue: claimed_issue, files, .. } = &msg.payload {
            if *claimed_issue != issue {
                continue;
            }
            let conflicts: Vec<String> = candidate_files
                .iter()
                .filter(|f| files.contains(f))
                .cloned()
                .collect();
            if !conflicts.is_empty() {
                return ScopeReport::Overlap {
                    existing_message_id: msg.id.clone(),
                    conflicting_files: conflicts,
                };
            }
        }
    }

    ScopeReport::Clear
}

#[cfg(test)]
mod tests {
    use super::*;
    use conductr_adapters::mock::MockMailbox;

    #[tokio::test]
    async fn no_claims_returns_clear() {
        let mb = MockMailbox::new();
        let report = check_scope(&mb, 1, &["src/foo.rs".to_string()]).await;
        assert_eq!(report, ScopeReport::Clear);
    }

    #[tokio::test]
    async fn non_overlapping_claim_returns_clear() {
        let mb = MockMailbox::new();
        mb.send("agent-a", MailKind::ScopeClaim {
            issue: 1,
            files: vec!["src/bar.rs".to_string()],
            summary: "bar".into(),
        })
        .await
        .unwrap();

        let report = check_scope(&mb, 1, &["src/foo.rs".to_string()]).await;
        assert_eq!(report, ScopeReport::Clear);
    }

    #[tokio::test]
    async fn overlapping_claim_returns_overlap() {
        let mb = MockMailbox::new();
        mb.send("agent-a", MailKind::ScopeClaim {
            issue: 1,
            files: vec!["src/foo.rs".to_string(), "src/bar.rs".to_string()],
            summary: "claim".into(),
        })
        .await
        .unwrap();

        let report = check_scope(&mb, 1, &["src/foo.rs".to_string()]).await;
        match report {
            ScopeReport::Overlap { conflicting_files, .. } => {
                assert_eq!(conflicting_files, vec!["src/foo.rs"]);
            }
            ScopeReport::Clear => panic!("expected overlap"),
        }
    }

    #[tokio::test]
    async fn different_issue_does_not_overlap() {
        let mb = MockMailbox::new();
        mb.send("agent-a", MailKind::ScopeClaim {
            issue: 2,
            files: vec!["src/foo.rs".to_string()],
            summary: "claim for issue 2".into(),
        })
        .await
        .unwrap();

        let report = check_scope(&mb, 1, &["src/foo.rs".to_string()]).await;
        assert_eq!(report, ScopeReport::Clear);
    }
}
