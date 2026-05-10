//! Git-flow hygiene sweep.
//!
//! Two checks:
//!
//! 1. **Forward-integration gap.** Commits on `main` not yet on `develop`
//!    indicate hotfixes that need merging back. Files one Finding when a gap
//!    exists.
//! 2. **Wrong base PRs.** Open PRs whose base is `main` when the project's
//!    convention is `develop`. One Finding per PR.
//!
//! Both shell out to `git`. The wrong-base check additionally requires `gh`.

use std::process::Stdio;

use async_trait::async_trait;
use tokio::process::Command;

use crate::sweep::{Finding, Severity, Sweep, SweepContext};

#[derive(Debug, Default)]
pub struct GitFlowSweep;

#[async_trait]
impl Sweep for GitFlowSweep {
    fn name(&self) -> &'static str { "git-flow" }

    async fn run(&self, ctx: &SweepContext) -> anyhow::Result<Vec<Finding>> {
        let mut findings = Vec::new();

        if !branch_exists(&ctx.repo_root, &ctx.develop_branch).await {
            return Ok(findings);
        }

        if let Some(gap) = forward_integration_gap(&ctx.repo_root, &ctx.main_branch, &ctx.develop_branch).await? {
            findings.push(
                Finding::new(
                    "git-flow",
                    format!("forward-integration:{}->{}", ctx.main_branch, ctx.develop_branch),
                    format!(
                        "Forward-integrate {} into {} ({} commit(s) behind)",
                        ctx.develop_branch, ctx.main_branch, gap
                    ),
                    format!(
                        "`{main}` has {n} commit(s) not yet on `{develop}`. \
                         Merge `{main}` into `{develop}` to keep the development branch current.\n\n\
                         git checkout {develop} && git merge --no-ff {main}",
                        main = ctx.main_branch, develop = ctx.develop_branch, n = gap
                    ),
                    Severity::Medium,
                )
                .with_labels(["git-flow".into(), "maintenance".into()]),
            );
        }

        if let Some(slug) = &ctx.repo_slug {
            if let Ok(prs) = wrong_base_prs(&ctx.repo_root, slug, &ctx.main_branch).await {
                for (pr, title) in prs {
                    findings.push(
                        Finding::new(
                            "git-flow",
                            format!("wrong-base:pr-{pr}"),
                            format!(
                                "PR #{pr} targets {} (project convention: {})",
                                ctx.main_branch, ctx.develop_branch
                            ),
                            format!(
                                "PR #{pr} \"{title}\" is open against `{main}`. Retarget to `{develop}` per git-flow convention.",
                                main = ctx.main_branch, develop = ctx.develop_branch
                            ),
                            Severity::Low,
                        )
                        .with_labels(["git-flow".into(), "pr-hygiene".into()]),
                    );
                }
            }
        }

        Ok(findings)
    }
}

async fn branch_exists(repo: &std::path::Path, branch: &str) -> bool {
    Command::new("git")
        .args(["rev-parse", "--verify", "--quiet", branch])
        .current_dir(repo)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

async fn forward_integration_gap(
    repo: &std::path::Path,
    main: &str,
    develop: &str,
) -> anyhow::Result<Option<u32>> {
    let out = Command::new("git")
        .args(["rev-list", "--count", &format!("{develop}..{main}")])
        .current_dir(repo)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await?;
    if !out.status.success() {
        return Ok(None);
    }
    let n: u32 = String::from_utf8_lossy(&out.stdout).trim().parse().unwrap_or(0);
    Ok(if n > 0 { Some(n) } else { None })
}

async fn wrong_base_prs(
    repo: &std::path::Path,
    slug: &conductr_orchestrate::RepoSlug,
    main: &str,
) -> anyhow::Result<Vec<(u64, String)>> {
    let out = Command::new("gh")
        .args([
            "pr", "list",
            "--repo", &slug.to_string(),
            "--base", main,
            "--state", "open",
            "--json", "number,title",
            "--limit", "100",
        ])
        .current_dir(repo)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await?;
    if !out.status.success() {
        return Ok(Vec::new());
    }
    #[derive(serde::Deserialize)]
    struct Raw { number: u64, title: String }
    let raw: Vec<Raw> = serde_json::from_slice(&out.stdout).unwrap_or_default();
    Ok(raw.into_iter().map(|r| (r.number, r.title)).collect())
}
