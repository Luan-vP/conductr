//! Abstract GitHub client used by the orchestrator.
//!
//! Implementations:
//! - [`GhCli`]: shells out to the `gh` CLI. Requires `gh` on PATH and
//!   authenticated. Used at runtime.
//! - Tests construct mock clients via the trait.

use std::collections::BTreeSet;
use std::process::Stdio;

use async_trait::async_trait;
use serde::Deserialize;
use tokio::process::Command;

use crate::types::{CiStatus, Issue, IssueNumber, IssueState, Pr, PrNumber, PrState, RepoSlug};

#[async_trait]
pub trait GitHubClient: Send + Sync {
    async fn list_open_issues(&self, repo: &RepoSlug) -> anyhow::Result<Vec<Issue>>;
    async fn list_closed_issue_numbers(&self, repo: &RepoSlug) -> anyhow::Result<BTreeSet<IssueNumber>>;
    async fn list_open_prs(&self, repo: &RepoSlug) -> anyhow::Result<Vec<Pr>>;
    async fn list_issue_comments(&self, repo: &RepoSlug, n: IssueNumber) -> anyhow::Result<Vec<String>>;
    async fn comment_issue(&self, repo: &RepoSlug, n: IssueNumber, body: &str) -> anyhow::Result<()>;
    async fn assign_issue(&self, repo: &RepoSlug, n: IssueNumber, login: &str) -> anyhow::Result<()>;
    async fn merge_pr_squash(&self, repo: &RepoSlug, n: PrNumber) -> anyhow::Result<()>;
}

/// `gh` CLI implementation.
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
impl GitHubClient for GhCli {
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
            "issue", "list", "--repo", &repo.to_string(), "--state", "open",
            "--json", "number,title,body,labels", "--limit", "500",
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

    async fn list_closed_issue_numbers(&self, repo: &RepoSlug) -> anyhow::Result<BTreeSet<IssueNumber>> {
        #[derive(Deserialize)]
        struct Raw { number: u64 }
        let out = run_gh(&[
            "issue", "list", "--repo", &repo.to_string(), "--state", "closed",
            "--json", "number", "--limit", "1000",
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
        }
        let out = run_gh(&[
            "pr", "list", "--repo", &repo.to_string(), "--state", "open",
            "--json", "number,title,body,headRefName,statusCheckRollup",
            "--limit", "200",
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
                }
            })
            .collect())
    }

    async fn list_issue_comments(&self, repo: &RepoSlug, n: IssueNumber) -> anyhow::Result<Vec<String>> {
        #[derive(Deserialize)]
        struct View { comments: Vec<Comment> }
        #[derive(Deserialize)]
        struct Comment { body: String }
        let out = run_gh(&[
            "issue", "view", &n.to_string(), "--repo", &repo.to_string(),
            "--json", "comments",
        ])
        .await?;
        let v: View = serde_json::from_str(&out)?;
        Ok(v.comments.into_iter().map(|c| c.body).collect())
    }

    async fn comment_issue(&self, repo: &RepoSlug, n: IssueNumber, body: &str) -> anyhow::Result<()> {
        let _ = run_gh(&[
            "issue", "comment", &n.to_string(), "--repo", &repo.to_string(),
            "--body", body,
        ])
        .await?;
        Ok(())
    }

    async fn assign_issue(&self, repo: &RepoSlug, n: IssueNumber, login: &str) -> anyhow::Result<()> {
        let _ = run_gh(&[
            "issue", "edit", &n.to_string(), "--repo", &repo.to_string(),
            "--add-assignee", login,
        ])
        .await?;
        Ok(())
    }

    async fn merge_pr_squash(&self, repo: &RepoSlug, n: PrNumber) -> anyhow::Result<()> {
        let _ = run_gh(&[
            "pr", "merge", &n.to_string(), "--repo", &repo.to_string(),
            "--squash", "--delete-branch",
        ])
        .await?;
        Ok(())
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
            Some("FAILURE") | Some("CANCELLED") | Some("TIMED_OUT") | Some("ACTION_REQUIRED") => {
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

/// Best-effort: extract the issue number a PR is for. Looks at branch name
/// (`claude/issue-<N>-...`) first, then `fixes #N` / `closes #N` / `for #N`
/// in the body.
pub fn linked_issue_from(head_ref: &str, body: &str) -> Option<IssueNumber> {
    if let Some(rest) = head_ref.strip_prefix("claude/issue-") {
        if let Some(num_str) = rest.split('-').next() {
            if let Ok(n) = num_str.parse::<IssueNumber>() {
                return Some(n);
            }
        }
    }
    use once_cell::sync::Lazy;
    use regex::Regex;
    static BODY_RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"(?i)\b(?:fixes|closes|resolves|for|implements)\s+#(\d+)").unwrap()
    });
    BODY_RE
        .captures(body)
        .and_then(|c| c.get(1)?.as_str().parse().ok())
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
        let checks = vec![RawCheck { conclusion: Some("SUCCESS".into()), status: None }];
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
        let checks = vec![RawCheck { conclusion: None, status: Some("IN_PROGRESS".into()) }];
        assert_eq!(rollup_to_ci(&checks), CiStatus::Pending);
    }
}
