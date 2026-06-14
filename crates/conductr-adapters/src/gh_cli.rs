//! `gh` CLI implementation of [`ScmHost`].
//!
//! Requires `gh` on PATH and authenticated. Shells out rather than using the
//! REST API directly, to reuse the existing auth credential store.

use std::collections::BTreeSet;
use std::process::Stdio;

use async_trait::async_trait;
use once_cell::sync::Lazy;
use regex::Regex;
use serde::Deserialize;
use tokio::process::Command;

use conductr_core::ports::ScmHost;
use conductr_core::types::{
    CiStatus, ClosedPr, Issue, IssueNumber, IssueState, Pr, PrNumber, PrState, RepoSlug,
};

#[derive(Debug, Clone, Default)]
pub struct GhCli;

#[derive(Debug, Clone, Deserialize)]
struct RawCheck {
    #[serde(default)]
    conclusion: Option<String>,
    #[serde(default)]
    status: Option<String>,
}

#[async_trait]
impl ScmHost for GhCli {
    async fn list_open_issues(&self, repo: &RepoSlug) -> anyhow::Result<Vec<Issue>> {
        #[derive(Deserialize)]
        struct Raw {
            number: u64,
            title: String,
            body: Option<String>,
            labels: Vec<RawLabel>,
        }
        #[derive(Deserialize)]
        struct RawLabel {
            name: String,
        }
        let out = run_gh(&[
            "issue",
            "list",
            "--repo",
            &repo.to_string(),
            "--state",
            "open",
            "--json",
            "number,title,body,labels",
            "--limit",
            "500",
        ])
        .await?;
        let raw: Vec<Raw> = serde_json::from_str(&out)?;
        Ok(raw
            .into_iter()
            .map(|r| Issue {
                number: r.number,
                title: r.title,
                body: r.body.unwrap_or_default(),
                labels: r.labels.into_iter().map(|l| l.name).collect(),
                state: IssueState::Open,
            })
            .collect())
    }

    async fn list_closed_issue_numbers(
        &self,
        repo: &RepoSlug,
    ) -> anyhow::Result<BTreeSet<IssueNumber>> {
        #[derive(Deserialize)]
        struct Raw {
            number: u64,
        }
        let out = run_gh(&[
            "issue",
            "list",
            "--repo",
            &repo.to_string(),
            "--state",
            "closed",
            "--json",
            "number",
            "--limit",
            "1000",
        ])
        .await?;
        let raw: Vec<Raw> = serde_json::from_str(&out)?;
        Ok(raw.into_iter().map(|r| r.number).collect())
    }

    async fn list_open_prs(&self, repo: &RepoSlug) -> anyhow::Result<Vec<Pr>> {
        #[derive(Deserialize)]
        struct Raw {
            number: u64,
            title: String,
            body: Option<String>,
            #[serde(rename = "headRefName")]
            head_ref_name: String,
            #[serde(rename = "statusCheckRollup", default)]
            status: Vec<RawCheck>,
            #[serde(rename = "isCrossRepository", default)]
            is_cross_repository: bool,
        }
        let out = run_gh(&[
            "pr",
            "list",
            "--repo",
            &repo.to_string(),
            "--state",
            "open",
            "--json",
            "number,title,body,headRefName,statusCheckRollup,isCrossRepository",
            "--limit",
            "200",
        ])
        .await?;
        let raw: Vec<Raw> = serde_json::from_str(&out)?;
        Ok(raw
            .into_iter()
            .map(|r| {
                let body = r.body.unwrap_or_default();
                Pr {
                    number: r.number,
                    title: r.title,
                    body: body.clone(),
                    head_ref: r.head_ref_name.clone(),
                    state: PrState::Open,
                    ci: rollup_to_ci(&r.status),
                    linked_issue: linked_issue_from(&r.head_ref_name, &body),
                    is_fork: r.is_cross_repository,
                }
            })
            .collect())
    }

    async fn list_issue_comments(
        &self,
        repo: &RepoSlug,
        n: IssueNumber,
    ) -> anyhow::Result<Vec<String>> {
        #[derive(Deserialize)]
        struct View {
            comments: Vec<Comment>,
        }
        #[derive(Deserialize)]
        struct Comment {
            body: String,
        }
        let out = run_gh(&[
            "issue",
            "view",
            &n.to_string(),
            "--repo",
            &repo.to_string(),
            "--json",
            "comments",
        ])
        .await?;
        let v: View = serde_json::from_str(&out)?;
        Ok(v.comments.into_iter().map(|c| c.body).collect())
    }

    async fn comment_issue(
        &self,
        repo: &RepoSlug,
        n: IssueNumber,
        body: &str,
    ) -> anyhow::Result<()> {
        run_gh(&[
            "issue",
            "comment",
            &n.to_string(),
            "--repo",
            &repo.to_string(),
            "--body",
            body,
        ])
        .await?;
        Ok(())
    }

    async fn assign_issue(
        &self,
        repo: &RepoSlug,
        n: IssueNumber,
        login: &str,
    ) -> anyhow::Result<()> {
        run_gh(&[
            "issue",
            "edit",
            &n.to_string(),
            "--repo",
            &repo.to_string(),
            "--add-assignee",
            login,
        ])
        .await?;
        Ok(())
    }

    async fn merge_pr_squash(&self, repo: &RepoSlug, n: PrNumber) -> anyhow::Result<()> {
        run_gh(&[
            "pr",
            "merge",
            &n.to_string(),
            "--repo",
            &repo.to_string(),
            "--squash",
            "--delete-branch",
        ])
        .await?;
        Ok(())
    }

    async fn add_issue_label(
        &self,
        repo: &RepoSlug,
        n: IssueNumber,
        label: &str,
    ) -> anyhow::Result<()> {
        run_gh(&[
            "issue",
            "edit",
            &n.to_string(),
            "--repo",
            &repo.to_string(),
            "--add-label",
            label,
        ])
        .await?;
        Ok(())
    }

    async fn remove_issue_label(
        &self,
        repo: &RepoSlug,
        n: IssueNumber,
        label: &str,
    ) -> anyhow::Result<()> {
        run_gh(&[
            "issue",
            "edit",
            &n.to_string(),
            "--repo",
            &repo.to_string(),
            "--remove-label",
            label,
        ])
        .await?;
        Ok(())
    }

    async fn list_closed_prs(&self, repo: &RepoSlug) -> anyhow::Result<Vec<ClosedPr>> {
        #[derive(serde::Deserialize)]
        struct Raw {
            number: u64,
            title: String,
            body: Option<String>,
            #[serde(rename = "headRefName")]
            head_ref_name: String,
            #[serde(rename = "createdAt")]
            created_at: chrono::DateTime<chrono::Utc>,
            #[serde(rename = "closedAt")]
            closed_at: Option<chrono::DateTime<chrono::Utc>>,
            #[serde(rename = "mergedAt")]
            merged_at: Option<chrono::DateTime<chrono::Utc>>,
        }
        let out = run_gh(&[
            "pr",
            "list",
            "--repo",
            &repo.to_string(),
            "--state",
            "closed",
            "--limit",
            "100",
            "--json",
            "number,title,body,headRefName,createdAt,closedAt,mergedAt",
        ])
        .await?;
        let raw: Vec<Raw> = serde_json::from_str(&out)?;
        Ok(raw
            .into_iter()
            .filter_map(|r| {
                let closed_at = r.closed_at?;
                let body = r.body.unwrap_or_default();
                let merged = r.merged_at.is_some();
                let state = if merged { PrState::Merged } else { PrState::Closed };
                Some(ClosedPr {
                    number: r.number,
                    title: r.title,
                    body: body.clone(),
                    head_ref: r.head_ref_name.clone(),
                    state,
                    linked_issue: linked_issue_from(&r.head_ref_name, &body),
                    opened_at: r.created_at,
                    closed_at,
                    merged,
                })
            })
            .collect())
    }

    async fn latest_ci_run_minutes(
        &self,
        repo: &RepoSlug,
        head_ref: &str,
    ) -> anyhow::Result<Option<f64>> {
        #[derive(serde::Deserialize)]
        struct Raw {
            #[serde(rename = "createdAt")]
            created_at: chrono::DateTime<chrono::Utc>,
            #[serde(rename = "updatedAt")]
            updated_at: chrono::DateTime<chrono::Utc>,
        }
        let out = run_gh(&[
            "run",
            "list",
            "--repo",
            &repo.to_string(),
            "--branch",
            head_ref,
            "--limit",
            "1",
            "--json",
            "createdAt,updatedAt",
        ])
        .await?;
        let raw: Vec<Raw> = serde_json::from_str(&out)?;
        Ok(raw.into_iter().next().map(|r| {
            let secs = (r.updated_at - r.created_at).num_seconds().max(0);
            secs as f64 / 60.0
        }))
    }

    async fn create_issue(
        &self,
        repo: &RepoSlug,
        title: &str,
        body: &str,
        labels: &[&str],
    ) -> anyhow::Result<IssueNumber> {
        let repo_str = repo.to_string();
        let mut cmd = Command::new("gh");
        cmd.args(["issue", "create", "--repo", &repo_str, "--title", title, "--body", body]);
        for label in labels {
            cmd.arg("--label").arg(label);
        }
        let out = cmd
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await?;
        if !out.status.success() {
            anyhow::bail!(
                "`gh issue create` failed ({}): {}",
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        let url = String::from_utf8(out.stdout)?.trim().to_string();
        url.rsplit('/')
            .next()
            .and_then(|s| s.trim().parse::<IssueNumber>().ok())
            .ok_or_else(|| anyhow::anyhow!("could not parse issue number from gh output: {url}"))
    }
}

async fn run_gh(args: &[&str]) -> anyhow::Result<String> {
    let out = Command::new("gh")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await?;
    if !out.status.success() {
        anyhow::bail!(
            "`gh {}` failed ({}): {}",
            args.join(" "),
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8(out.stdout)?)
}

fn rollup_to_ci(checks: &[RawCheck]) -> CiStatus {
    if checks.is_empty() {
        return CiStatus::Unknown;
    }
    let mut any_failing = false;
    let mut any_pending = false;
    for c in checks {
        match c.conclusion.as_deref() {
            Some("SUCCESS") | Some("NEUTRAL") | Some("SKIPPED") => {}
            Some("FAILURE")
            | Some("CANCELLED")
            | Some("TIMED_OUT")
            | Some("ACTION_REQUIRED") => {
                any_failing = true;
            }
            _ => match c.status.as_deref() {
                Some("COMPLETED") => {}
                _ => any_pending = true,
            },
        }
    }
    if any_failing {
        CiStatus::Failing
    } else if any_pending {
        CiStatus::Pending
    } else {
        CiStatus::Passing
    }
}

/// Extract the issue number linked to a PR. Checks branch name first
/// (`claude/issue-<N>-...`), then `fixes #N` / `closes #N` in the body.
pub fn linked_issue_from(head_ref: &str, body: &str) -> Option<IssueNumber> {
    if let Some(rest) = head_ref.strip_prefix("claude/issue-") {
        if let Some(num_str) = rest.split('-').next() {
            if let Ok(n) = num_str.parse::<IssueNumber>() {
                return Some(n);
            }
        }
    }
    static BODY_RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"(?i)\b(?:fixes|closes|resolves|for|implements)\s+#(\d+)").unwrap()
    });
    BODY_RE.captures(body).and_then(|c| c.get(1)?.as_str().parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linked_issue_from_branch() {
        assert_eq!(linked_issue_from("claude/issue-42-foo-bar", ""), Some(42));
    }

    #[test]
    fn linked_issue_from_body() {
        assert_eq!(linked_issue_from("feature/x", "fixes #7"), Some(7));
    }

    #[test]
    fn no_link_returns_none() {
        assert_eq!(linked_issue_from("feature/x", "see #99 for context"), None);
    }

    #[test]
    fn rollup_passing() {
        let checks =
            vec![RawCheck { conclusion: Some("SUCCESS".into()), status: None }];
        assert_eq!(rollup_to_ci(&checks), CiStatus::Passing);
    }

    #[test]
    fn rollup_failing() {
        let checks = vec![
            RawCheck { conclusion: Some("SUCCESS".into()), status: None },
            RawCheck { conclusion: Some("FAILURE".into()), status: None },
        ];
        assert_eq!(rollup_to_ci(&checks), CiStatus::Failing);
    }

    #[test]
    fn rollup_pending() {
        let checks =
            vec![RawCheck { conclusion: None, status: Some("IN_PROGRESS".into()) }];
        assert_eq!(rollup_to_ci(&checks), CiStatus::Pending);
    }
}
