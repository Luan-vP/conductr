//! Top-level orchestration loop.

use std::collections::BTreeSet;
use std::time::Duration;

use tracing::{info, warn};

use crate::classifier::{classify, Bucket, Classification};
use crate::github::GitHubClient;
use crate::types::{IssueNumber, RepoSlug};

#[derive(Debug, Clone)]
pub struct OrchestratorConfig {
    pub repo: RepoSlug,
    /// Comment text used to trigger the bot.
    pub trigger_comment: String,
    /// Polling interval between cycles.
    pub poll_interval: Duration,
    /// Max cycles to run before giving up (None = unbounded).
    pub max_cycles: Option<u32>,
    /// Default human assignee if CODEOWNERS resolution is unavailable.
    pub default_human_assignee: Option<String>,
    /// If true, only print the plan; do not actually comment / merge.
    pub dry_run: bool,
}

impl OrchestratorConfig {
    pub fn new(repo: RepoSlug) -> Self {
        Self {
            repo,
            trigger_comment: "@claude please implement".into(),
            poll_interval: Duration::from_secs(60),
            max_cycles: None,
            default_human_assignee: None,
            dry_run: false,
        }
    }
}

pub struct Orchestrator<C: GitHubClient> {
    pub client: C,
    pub config: OrchestratorConfig,
}

#[derive(Debug, Default, Clone)]
pub struct CycleReport {
    pub merged: Vec<u64>,
    pub triggered: Vec<IssueNumber>,
    pub waiting: Vec<IssueNumber>,
    pub blocked: Vec<IssueNumber>,
    pub human: Vec<IssueNumber>,
    pub pr_failing: Vec<u64>,
    pub progress_made: bool,
}

impl<C: GitHubClient> Orchestrator<C> {
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

        // Determine which issues have already been triggered by scanning their comments.
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
    use crate::types::{Issue, IssueState, Pr};
    use async_trait::async_trait;
    use std::sync::Mutex;

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
}
