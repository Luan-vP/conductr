//! Top-level orchestration loop.

use std::collections::BTreeSet;
use std::sync::Arc;

use tracing::{info, warn};

use crate::classifier::{classify, Bucket, Classification};
use crate::github::GitHubClient;
use crate::types::{IssueNumber, RepoSlug};

use conductr_core::ports::Mailbox;
use conductr_core::types::MailKind;

pub use conductr_core::types::{CycleReport, OrchestratorConfig};

pub struct Orchestrator<C: GitHubClient> {
    pub client: C,
    pub config: OrchestratorConfig,
    mailbox: Option<Arc<dyn Mailbox>>,
}

impl<C: GitHubClient> Orchestrator<C> {
    pub fn new(client: C, config: OrchestratorConfig) -> Self {
        Self { client, config, mailbox: None }
    }

    /// Attach an optional mailbox for scope-dedup. When set, `run_cycle` will
    /// skip Ready issues that have an existing `ScopeClaim` overlap.
    pub fn with_mailbox(mut self, mailbox: Arc<dyn Mailbox>) -> Self {
        self.mailbox = Some(mailbox);
        self
    }

    /// Check the mailbox for an existing scope claim that overlaps with `issue`.
    /// Returns `Some(message_id)` if an overlap is found.
    async fn check_scope_overlap(
        mailbox: &dyn Mailbox,
        issue: IssueNumber,
    ) -> Option<String> {
        let messages = mailbox.inbox(Some("scope_claim"), None).await.ok()?;
        for msg in messages {
            if let MailKind::ScopeClaim { issue: claimed_issue, .. } = &msg.payload {
                if *claimed_issue == issue {
                    return Some(msg.id.clone());
                }
            }
        }
        None
    }

    /// Run a single cycle of survey → classify → act.
    pub async fn run_cycle(&self) -> anyhow::Result<CycleReport> {
        let repo = &self.config.repo;
        info!(%repo, "surveying repo state");

        let open_issues = self.client.list_open_issues(repo).await?;
        let closed = self.client.list_closed_issue_numbers(repo).await?;
        let prs = self.client.list_open_prs(repo).await?;

        let mut triggered: BTreeSet<IssueNumber> = BTreeSet::new();
        for issue in &open_issues {
            let comments = self.client.list_issue_comments(repo, issue.number).await.unwrap_or_default();
            if comments.iter().any(|c| c.contains(&self.config.trigger_comment)) {
                triggered.insert(issue.number);
            }
        }

        let classifications: Vec<Classification> = open_issues
            .iter()
            .map(|i| classify(i, &closed, &prs, &triggered))
            .collect();

        let mut report = CycleReport::default();

        // 1) Merge any PRs that are passing CI.
        for c in &classifications {
            if c.bucket == Bucket::PrOpen {
                if let Some(pr) = c.pr {
                    if self.config.dry_run {
                        info!(pr, "would merge");
                    } else {
                        match self.client.merge_pr_squash(repo, pr).await {
                            Ok(_) => {
                                info!(pr, "merged");
                                report.merged.push(pr);
                                report.progress_made = true;
                            }
                            Err(e) => warn!(pr, error=%e, "merge failed"),
                        }
                    }
                }
            } else if c.bucket == Bucket::PrFailing {
                if let Some(pr) = c.pr {
                    report.pr_failing.push(pr);
                }
            }
        }

        // 2) Trigger Ready issues in parallel (subject to dry_run / scope dedup).
        for c in &classifications {
            if c.bucket == Bucket::Ready {
                // Scope dedup: skip if another agent has claimed this issue.
                if let Some(mb) = &self.mailbox {
                    if let Some(msg_id) = Self::check_scope_overlap(mb.as_ref(), c.issue).await {
                        info!(issue=c.issue, msg_id=%msg_id, "skipping — scope overlap");
                        report.scope_overlap.push(c.issue);
                        continue;
                    }
                }

                if self.config.dry_run {
                    info!(issue=c.issue, "would trigger");
                } else {
                    match self
                        .client
                        .comment_issue(repo, c.issue, &self.config.trigger_comment)
                        .await
                    {
                        Ok(_) => {
                            info!(issue=c.issue, "triggered");
                            report.triggered.push(c.issue);
                            report.progress_made = true;
                        }
                        Err(e) => warn!(issue=c.issue, error=%e, "trigger failed"),
                    }
                }
            } else if c.bucket == Bucket::TriggeredWaiting {
                report.waiting.push(c.issue);
            } else if c.bucket == Bucket::Blocked {
                report.blocked.push(c.issue);
            } else if c.bucket == Bucket::Human {
                report.human.push(c.issue);
                if let Some(login) = &self.config.default_human_assignee {
                    if !self.config.dry_run {
                        let _ = self.client.assign_issue(repo, c.issue, login).await;
                    }
                }
            }
        }

        Ok(report)
    }

    /// Drive the orchestration to completion (or until `max_cycles`).
    pub async fn run_to_completion(&self) -> anyhow::Result<Vec<CycleReport>> {
        let mut history = Vec::new();
        let mut cycle = 0u32;
        loop {
            let report = self.run_cycle().await?;
            let nothing_left = report.triggered.is_empty()
                && report.merged.is_empty()
                && report.waiting.is_empty()
                && report.pr_failing.is_empty();
            history.push(report);
            cycle += 1;
            if nothing_left {
                info!("nothing left to do");
                break;
            }
            if let Some(max) = self.config.max_cycles {
                if cycle >= max {
                    info!(cycles = cycle, "reached max cycles");
                    break;
                }
            }
            tokio::time::sleep(self.config.poll_interval).await;
        }
        Ok(history)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Issue, IssueState, Pr};
    use async_trait::async_trait;
    use chrono::Utc;
    use std::sync::{Arc, Mutex, RwLock};
    use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
    use conductr_core::ports::{Mailbox, MailboxError};
    use conductr_core::types::{MailKind, MailMessage, MailRef};

    #[derive(Default)]
    struct FakeClient {
        pub triggered: Mutex<Vec<IssueNumber>>,
        pub merged: Mutex<Vec<u64>>,
    }

    #[async_trait]
    impl GitHubClient for FakeClient {
        async fn list_open_issues(&self, _: &RepoSlug) -> anyhow::Result<Vec<Issue>> {
            Ok(vec![Issue {
                number: 1,
                title: "x".into(),
                body: "no deps".into(),
                labels: vec![],
                state: IssueState::Open,
            }])
        }
        async fn list_closed_issue_numbers(&self, _: &RepoSlug) -> anyhow::Result<BTreeSet<IssueNumber>> {
            Ok(BTreeSet::new())
        }
        async fn list_open_prs(&self, _: &RepoSlug) -> anyhow::Result<Vec<Pr>> {
            Ok(vec![])
        }
        async fn list_issue_comments(&self, _: &RepoSlug, _: IssueNumber) -> anyhow::Result<Vec<String>> {
            Ok(vec![])
        }
        async fn comment_issue(&self, _: &RepoSlug, n: IssueNumber, _: &str) -> anyhow::Result<()> {
            self.triggered.lock().unwrap().push(n);
            Ok(())
        }
        async fn assign_issue(&self, _: &RepoSlug, _: IssueNumber, _: &str) -> anyhow::Result<()> {
            Ok(())
        }
        async fn merge_pr_squash(&self, _: &RepoSlug, n: u64) -> anyhow::Result<()> {
            self.merged.lock().unwrap().push(n);
            Ok(())
        }
    }

    /// Minimal in-test mailbox that pre-seeds messages.
    #[derive(Default)]
    struct FakeMailbox {
        messages: Arc<RwLock<Vec<MailMessage>>>,
        counter: AtomicU64,
    }

    impl FakeMailbox {
        fn with_scope_claim(self, issue: u64) -> Self {
            self.messages.write().unwrap().push(MailMessage {
                id: format!("pre-{issue}"),
                agent: "agent-a".into(),
                sent_at: Utc::now(),
                payload: MailKind::ScopeClaim {
                    issue,
                    files: vec!["src/lib.rs".into()],
                    summary: "claimed".into(),
                },
            });
            self
        }
    }

    #[async_trait]
    impl Mailbox for FakeMailbox {
        async fn send(&self, agent: &str, payload: MailKind) -> Result<MailRef, MailboxError> {
            let id = format!("msg-{}", self.counter.fetch_add(1, AtomicOrdering::SeqCst));
            self.messages.write().unwrap().push(MailMessage {
                id: id.clone(),
                agent: agent.to_string(),
                sent_at: Utc::now(),
                payload,
            });
            Ok(id)
        }

        async fn inbox(
            &self,
            kind_filter: Option<&str>,
            _since: Option<std::time::Duration>,
        ) -> Result<Vec<MailMessage>, MailboxError> {
            let all = self.messages.read().unwrap().clone();
            Ok(match kind_filter {
                Some("scope_claim") => all
                    .into_iter()
                    .filter(|m| matches!(m.payload, MailKind::ScopeClaim { .. }))
                    .collect(),
                _ => all,
            })
        }

        async fn thread(&self, _: &MailRef) -> Result<Vec<MailMessage>, MailboxError> {
            Ok(vec![])
        }
    }

    #[tokio::test]
    async fn ready_issue_gets_triggered() {
        let client = FakeClient::default();
        let cfg = OrchestratorConfig::new(RepoSlug::new("o", "r"));
        let orch = Orchestrator::new(client, cfg);
        let report = orch.run_cycle().await.unwrap();
        assert_eq!(report.triggered, vec![1]);
        assert_eq!(orch.client.triggered.lock().unwrap().clone(), vec![1]);
    }

    #[tokio::test]
    async fn dry_run_does_not_act() {
        let client = FakeClient::default();
        let mut cfg = OrchestratorConfig::new(RepoSlug::new("o", "r"));
        cfg.dry_run = true;
        let orch = Orchestrator::new(client, cfg);
        let report = orch.run_cycle().await.unwrap();
        assert!(report.triggered.is_empty());
        assert!(orch.client.triggered.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn scope_overlap_skips_ready_issue() {
        let client = FakeClient::default();
        let cfg = OrchestratorConfig::new(RepoSlug::new("o", "r"));
        let mb = Arc::new(FakeMailbox::default().with_scope_claim(1));
        let orch = Orchestrator::new(client, cfg).with_mailbox(mb);
        let report = orch.run_cycle().await.unwrap();
        // Issue 1 is Ready but has a scope claim → skipped.
        assert!(report.triggered.is_empty());
        assert_eq!(report.scope_overlap, vec![1]);
        assert!(orch.client.triggered.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn no_mailbox_behaves_as_before() {
        let client = FakeClient::default();
        let cfg = OrchestratorConfig::new(RepoSlug::new("o", "r"));
        let orch = Orchestrator::new(client, cfg);
        let report = orch.run_cycle().await.unwrap();
        assert_eq!(report.triggered, vec![1]);
        assert!(report.scope_overlap.is_empty());
    }
}
