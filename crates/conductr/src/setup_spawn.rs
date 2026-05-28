//! `conductr setup spawn` — provision all active projects in the machine-wide registry.
//!
//! For each active project:
//!   1. Verify the local clone exists; git-clone if missing.
//!   2. Verify the per-repo `.conductr` exists; generate from registry defaults if missing.
//!   3. Run `cadence sync` from the project path.
//!   4. Verify the `conductr-<tag>` tmux session exists; create it if missing.
//!
//! Pending projects are skipped with a one-line reminder. Clone failures log and
//! continue — one bad project does not abort the whole pass.

use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::cadence;
use crate::registry::{Registry, RegistryProject};

// ── Outcome types ─────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum ProjectOutcome {
    AlreadyProvisioned,
    Provisioned,
    Skipped { reason: String },
}

#[derive(Debug)]
pub struct ProjectReport {
    pub tag: String,
    pub outcome: ProjectOutcome,
}

// ── Options ───────────────────────────────────────────────────────────────────

pub struct SpawnOptions {
    pub dry_run: bool,
    pub tag_filter: Option<String>,
    pub include_pending: bool,
}

// ── Entry point ───────────────────────────────────────────────────────────────

pub async fn run(registry: &Registry, opts: &SpawnOptions) -> Result<Vec<ProjectReport>> {
    let pending_count = registry.pending().count();

    let projects: Vec<&RegistryProject> = if let Some(tag) = &opts.tag_filter {
        match registry.find_by_tag(tag) {
            Some(p) => vec![p],
            None => anyhow::bail!("no project with tag '{}' found in registry", tag),
        }
    } else {
        let mut ps: Vec<&RegistryProject> = registry.active().collect();
        if opts.include_pending {
            ps.extend(registry.pending());
        }
        ps
    };

    let mut reports = Vec::new();
    for project in projects {
        let outcome = provision_project(project, registry, opts).await;
        reports.push(ProjectReport { tag: project.tag.clone(), outcome });
    }

    if pending_count > 0 && !opts.include_pending && opts.tag_filter.is_none() {
        println!("\n{pending_count} project(s) pending — edit ~/.conductr to promote");
    }

    Ok(reports)
}

// ── Per-project provisioning ──────────────────────────────────────────────────

async fn provision_project(
    project: &RegistryProject,
    registry: &Registry,
    opts: &SpawnOptions,
) -> ProjectOutcome {
    let path = &project.path;
    let tag = &project.tag;

    // Step 1: Verify / clone local repo.
    if !path.exists() {
        let clone_url = format!("git@github.com:{}", project.repo);
        if opts.dry_run {
            println!("  {tag}: plan: would clone {clone_url} → {}", path.display());
        } else {
            println!("  {tag}: cloning {clone_url}…");
            let status = std::process::Command::new("git")
                .args(["clone", &clone_url, &path.to_string_lossy()])
                .status();
            match status {
                Ok(s) if s.success() => println!("  {tag}: ✓ cloned"),
                Ok(s) => {
                    let reason = format!("git clone exited with {s}");
                    eprintln!("  {tag}: ⚠ {reason}");
                    return ProjectOutcome::Skipped { reason };
                }
                Err(e) => {
                    let reason = format!("git clone failed: {e}");
                    eprintln!("  {tag}: ⚠ {reason}");
                    return ProjectOutcome::Skipped { reason };
                }
            }
        }
    }

    // Step 2: Verify / generate per-repo .conductr.
    let dot_conductr = path.join(".conductr");
    if !dot_conductr.exists() {
        if opts.dry_run {
            println!("  {tag}: plan: would generate .conductr from registry defaults");
        } else {
            match generate_dot_conductr(project, registry) {
                Ok(()) => println!("  {tag}: ✓ generated .conductr"),
                Err(e) => eprintln!("  {tag}: ⚠ could not generate .conductr: {e}"),
            }
        }
    }

    // Step 3: Run cadence sync.
    if opts.dry_run {
        println!("  {tag}: plan: would run cadence sync");
    } else {
        match cadence::sync(path, false, cadence::Mechanism::Crontab) {
            Ok(report) => {
                let trimmed = report.trim();
                if !trimmed.is_empty() {
                    println!("  {tag}: cadence: {trimmed}");
                }
            }
            Err(e) => eprintln!("  {tag}: ⚠ cadence sync: {e}"),
        }
    }

    // Step 4: Verify / create tmux session.
    let session_name = format!("conductr-{tag}");
    if opts.dry_run {
        println!("  {tag}: plan: would ensure tmux session '{session_name}'");
        return ProjectOutcome::Provisioned;
    }

    if check_tmux_session(&session_name) {
        ProjectOutcome::AlreadyProvisioned
    } else {
        let cwd = path.to_string_lossy().into_owned();
        match create_tmux_session(&session_name, &cwd) {
            Ok(()) => {
                println!("  {tag}: ↺ created tmux session '{session_name}'");
                ProjectOutcome::Provisioned
            }
            Err(e) => {
                eprintln!("  {tag}: ⚠ tmux: {e}");
                ProjectOutcome::Provisioned
            }
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn generate_dot_conductr(project: &RegistryProject, registry: &Registry) -> anyhow::Result<()> {
    let dot_conductr = project.path.join(".conductr");
    let d = &registry.defaults;

    let cadence_orchestrate = d
        .cadence_orchestrate
        .as_deref()
        .unwrap_or("*/30 * * * *");
    let cadence_idle = d.cadence_idle.as_deref().unwrap_or("17 * * * *");

    let mut content = format!(
        "# Project config for {tag} — generated by `conductr setup spawn`.\n\
         # Edit fields as needed.\n\
         \n\
         project_tag = \"{tag}\"\n\
         repo        = \"{repo}\"\n",
        tag = project.tag,
        repo = project.repo,
    );

    if let Some(ha) = &d.human_assignee {
        content.push_str(&format!("\n[band]\nhuman_assignee = \"{ha}\"\n"));
    }

    if let Some(lp) = &d.local_provider {
        content.push_str(&format!("\n[local]\nprovider = \"{lp}\"\n"));
    }

    content.push_str(&format!(
        "\n[cadence]\norchestrate = \"{cadence_orchestrate}\"\nidle        = \"{cadence_idle}\"\n"
    ));

    std::fs::write(&dot_conductr, content)
        .with_context(|| format!("writing {}", dot_conductr.display()))
}

fn check_tmux_session(name: &str) -> bool {
    std::process::Command::new("tmux")
        .args(["has-session", "-t", name])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn create_tmux_session(name: &str, cwd: &str) -> anyhow::Result<()> {
    let status = std::process::Command::new("tmux")
        .args(["new-session", "-d", "-s", name, "-c", cwd])
        .status()
        .context("running tmux new-session")?;
    if status.success() {
        Ok(())
    } else {
        anyhow::bail!("tmux new-session exited with {status}");
    }
}

// ── Report ────────────────────────────────────────────────────────────────────

pub fn print_report(reports: &[ProjectReport]) {
    if reports.is_empty() {
        return;
    }
    println!();
    println!("{:<20}  {}", "PROJECT", "STATUS");
    for r in reports {
        let status = match &r.outcome {
            ProjectOutcome::AlreadyProvisioned => "✓ already provisioned".to_string(),
            ProjectOutcome::Provisioned => "↺ provisioned this pass".to_string(),
            ProjectOutcome::Skipped { reason } => format!("⚠ skipped ({reason})"),
        };
        println!("{:<20}  {status}", r.tag);
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry;

    const FIXTURE: &str = r#"
[defaults]
human_assignee      = "Luan-vP"
local_provider      = "ollama"
cadence_orchestrate = "*/30 * * * *"
cadence_idle        = "17 * * * *"

[[projects]]
tag    = "alpha"
repo   = "owner/alpha"
path   = "/nonexistent/alpha"
status = "active"

[[projects]]
tag    = "beta"
repo   = "owner/beta"
path   = "/nonexistent/beta"
status = "active"

[[projects]]
tag    = "gamma"
repo   = "owner/gamma"
path   = "/nonexistent/gamma"
status = "pending"
"#;

    #[tokio::test]
    async fn dry_run_against_fixture_produces_plan() {
        let reg = registry::parse(FIXTURE).unwrap();
        let opts = SpawnOptions {
            dry_run: true,
            tag_filter: None,
            include_pending: false,
        };
        let reports = run(&reg, &opts).await.unwrap();
        assert_eq!(reports.len(), 2, "only active projects processed");
        for r in &reports {
            assert!(
                matches!(r.outcome, ProjectOutcome::Provisioned),
                "dry-run: non-existent path should be Provisioned (would-clone)"
            );
        }
    }

    #[tokio::test]
    async fn dry_run_single_tag_filter() {
        let reg = registry::parse(FIXTURE).unwrap();
        let opts = SpawnOptions {
            dry_run: true,
            tag_filter: Some("alpha".to_string()),
            include_pending: false,
        };
        let reports = run(&reg, &opts).await.unwrap();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].tag, "alpha");
    }

    #[tokio::test]
    async fn dry_run_include_pending() {
        let reg = registry::parse(FIXTURE).unwrap();
        let opts = SpawnOptions {
            dry_run: true,
            tag_filter: None,
            include_pending: true,
        };
        let reports = run(&reg, &opts).await.unwrap();
        assert_eq!(reports.len(), 3, "all projects (active + pending) processed");
    }

    #[tokio::test]
    async fn unknown_tag_errors() {
        let reg = registry::parse(FIXTURE).unwrap();
        let opts = SpawnOptions {
            dry_run: true,
            tag_filter: Some("nonexistent".to_string()),
            include_pending: false,
        };
        assert!(run(&reg, &opts).await.is_err());
    }

    #[test]
    fn generate_dot_conductr_uses_defaults() {
        let reg = registry::parse(FIXTURE).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let project = registry::RegistryProject {
            tag: "test".to_string(),
            repo: "owner/test".to_string(),
            path: tmp.path().to_path_buf(),
            status: registry::ProjectStatus::Active,
        };
        generate_dot_conductr(&project, &reg).unwrap();
        let content = std::fs::read_to_string(tmp.path().join(".conductr")).unwrap();
        assert!(content.contains("project_tag = \"test\""));
        assert!(content.contains("repo        = \"owner/test\""));
        assert!(content.contains("human_assignee = \"Luan-vP\""));
        assert!(content.contains("provider = \"ollama\""));
        assert!(content.contains("orchestrate = \"*/30 * * * *\""));
        assert!(content.contains("idle        = \"17 * * * *\""));
    }
}
