//! Top-level orchestration loop.

use std::collections::BTreeSet;

use tracing::{info, warn};

use crate::classifier::{classify, Bucket, Classification};
use crate::types::IssueNumber;
use conductr_core::ports::ScmHost;

pub use conductr_core::types::{CycleReport, OrchestratorConfig};

pub struct Orchestrator<C: ScmHost> {
    pub client: C,
    pub config: OrchestratorConfig,
}

impl<C: ScmHost> Orchestrator<C> {
    pub fn new(client: C, config: OrchestratorConfig) -> Self {
        Self { client, config }
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

        // 2) Trigger Ready issues in parallel (subject to dry_run).
        for c in &classifications {
            if c.bucket == Bucket::Ready {
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
    use conductr_adapters::mock::MockScmHost;
    use conductr_core::types::{Issue, IssueState, RepoSlug};

    #[tokio::test]
    async fn ready_issue_gets_triggered() {
        let client = MockScmHost::new().with_issues([Issue {
            number: 1,
            title: "x".into(),
            body: "no deps".into(),
            labels: vec![],
            state: IssueState::Open,
        }]);
        let cfg = OrchestratorConfig::new(RepoSlug::new("o", "r"));
        let orch = Orchestrator::new(client, cfg);
        let report = orch.run_cycle().await.unwrap();
        assert_eq!(report.triggered, vec![1]);
        assert_eq!(orch.client.posted_comments(), vec![(1, "@claude please implement".to_string())]);
    }

    #[tokio::test]
    async fn dry_run_does_not_act() {
        let client = MockScmHost::new().with_issues([Issue {
            number: 1,
            title: "x".into(),
            body: "no deps".into(),
            labels: vec![],
            state: IssueState::Open,
        }]);
        let mut cfg = OrchestratorConfig::new(RepoSlug::new("o", "r"));
        cfg.dry_run = true;
        let orch = Orchestrator::new(client, cfg);
        let report = orch.run_cycle().await.unwrap();
        assert!(report.triggered.is_empty());
        assert!(orch.client.posted_comments().is_empty());
    }
}
