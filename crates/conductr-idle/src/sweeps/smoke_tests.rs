//! Smoke-test gap sweep.
//!
//! For each recent commit on the development branch, check whether a smoke
//! test workflow run exists for that SHA. Files a Finding for commits with
//! no recorded smoke run.
//!
//! Configuration: the workflow file name is read from `CONDUCTR_SMOKE_WORKFLOW`
//! (default `smoke.yml`). Set it to an empty string to disable the sweep.

use std::process::Stdio;

use async_trait::async_trait;
use tokio::process::Command;

use crate::sweep::{Finding, Severity, Sweep, SweepContext};

#[derive(Debug, Default)]
pub struct SmokeTestSweep;

#[async_trait]
impl Sweep for SmokeTestSweep {
    fn name(&self) -> &'static str { "smoke-tests" }

    async fn run(&self, ctx: &SweepContext) -> anyhow::Result<Vec<Finding>> {
        let workflow = std::env::var("CONDUCTR_SMOKE_WORKFLOW").unwrap_or_else(|_| "smoke.yml".into());
        if workflow.is_empty() {
            return Ok(Vec::new());
        }
        let Some(slug) = &ctx.repo_slug else { return Ok(Vec::new()); };
        let commits = recent_commits(&ctx.repo_root, &ctx.develop_branch, 25).await?;
        let mut findings = Vec::new();
        for sha in commits {
            if has_smoke_run(&ctx.repo_root, slug, &workflow, &sha).await? {
                continue;
            }
            findings.push(
                Finding::new(
                    "smoke-tests",
                    format!("missing:{sha}"),
                    format!("No smoke run for {}", &sha[..7.min(sha.len())]),
                    format!(
                        "Commit `{sha}` on `{}` has no `{workflow}` workflow run yet. \
                         Re-run with: `gh workflow run {workflow} --ref {sha}`.",
                        ctx.develop_branch
                    ),
                    Severity::Low,
                )
                .with_labels(["smoke-tests".into(), "ci-gap".into()]),
            );
        }
        Ok(findings)
    }
}

async fn recent_commits(repo: &std::path::Path, branch: &str, n: u32) -> anyhow::Result<Vec<String>> {
    let out = Command::new("git")
        .args(["log", branch, &format!("-n{n}"), "--pretty=%H"])
        .current_dir(repo)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await?;
    if !out.status.success() {
        return Ok(Vec::new());
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect())
}

async fn has_smoke_run(
    repo: &std::path::Path,
    slug: &conductr_orchestrate::RepoSlug,
    workflow: &str,
    sha: &str,
) -> anyhow::Result<bool> {
    let out = Command::new("gh")
        .args([
            "run", "list",
            "--repo", &slug.to_string(),
            "--workflow", workflow,
            "--commit", sha,
            "--limit", "1",
            "--json", "databaseId",
        ])
        .current_dir(repo)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await?;
    if !out.status.success() {
        return Ok(true); // be conservative — don't spam findings on tooling failures
    }
    let body = String::from_utf8_lossy(&out.stdout);
    Ok(body.trim() != "[]")
}
