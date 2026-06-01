//! `conductr setup spawn` — provision all active projects in the machine-wide registry.
//!
//! For each in-scope project, four idempotent steps run in order:
//!   1. `clone`        — verify the local clone exists; git-clone if missing.
//!   2. `dot-conductr` — verify the per-repo `.conductr` exists; generate from defaults if missing.
//!   3. `cadence`      — run `cadence sync` from the project path.
//!   4. `session`      — verify the `conductr-<tag>` tmux session exists; create it if missing,
//!                       then boot Claude into a freshly-created session (`launch`).
//!
//! Every step records a structured [`StepReport`] instead of printing inline, so the
//! same provisioning logic backs both `conductr setup spawn` (which prints a table)
//! and `conductr pod heal` (which consumes the reports and adds a liveness/restart
//! pass on top). Pending projects are skipped unless `include_pending` is set; a
//! clone failure aborts that one project but never the whole pass.

use anyhow::{Context, Result};
use serde::Serialize;

use crate::cadence;
use crate::registry::{Registry, RegistryProject};

/// Command sent into a freshly-created session so it boots Claude with Remote
/// Control enabled in `auto` permission mode — immediately driveable by
/// orchestrate and other tools. Shared with `pod heal`'s restart pass so both
/// surfaces relaunch sessions identically.
pub const DEFAULT_LAUNCH_COMMAND: &str = "claude --remote-control --permission-mode auto";

// ── Step reporting ────────────────────────────────────────────────────────────

/// Outcome of a single provisioning step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum StepStatus {
    /// Already satisfied — nothing to do (repo present, session exists, …).
    Present,
    /// Action performed this pass (cloned, generated, created, launched, synced).
    Done,
    /// Dry-run: this is what *would* happen.
    Planned,
    /// Step failed; the project continues unless it was the clone step.
    Error,
}

/// One provisioning step's result. `step` is a stable identifier
/// (`clone` | `dot-conductr` | `cadence` | `session` | `launch`).
#[derive(Debug, Clone, Serialize)]
pub struct StepReport {
    pub step: &'static str,
    pub status: StepStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl StepReport {
    fn new(step: &'static str, status: StepStatus, detail: Option<String>) -> Self {
        Self { step, status, detail }
    }
}

// ── Project reporting ─────────────────────────────────────────────────────────

/// Coarse per-project summary, derived from the step results.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "kebab-case", tag = "kind", content = "reason")]
pub enum ProjectOutcome {
    /// Every step found its target already satisfied.
    AlreadyProvisioned,
    /// At least one step performed (or would perform) an action.
    Provisioned,
    /// The clone step failed — nothing downstream could run.
    Skipped(String),
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectReport {
    pub tag: String,
    /// `conductr-<tag>` tmux session name.
    pub session: String,
    pub outcome: ProjectOutcome,
    pub steps: Vec<StepReport>,
}

impl ProjectReport {
    fn finish(tag: String, session: String, steps: Vec<StepReport>) -> Self {
        let outcome = derive_outcome(&steps);
        ProjectReport { tag, session, outcome, steps }
    }
}

/// A hard clone failure means the project couldn't be provisioned at all;
/// otherwise any `Done`/`Planned` step counts as provisioned this pass.
fn derive_outcome(steps: &[StepReport]) -> ProjectOutcome {
    if let Some(s) = steps
        .iter()
        .find(|s| s.step == "clone" && s.status == StepStatus::Error)
    {
        let reason = s.detail.clone().unwrap_or_else(|| "clone failed".to_string());
        return ProjectOutcome::Skipped(reason);
    }
    if steps
        .iter()
        .any(|s| matches!(s.status, StepStatus::Done | StepStatus::Planned))
    {
        ProjectOutcome::Provisioned
    } else {
        ProjectOutcome::AlreadyProvisioned
    }
}

// ── Options ───────────────────────────────────────────────────────────────────

pub struct SpawnOptions {
    pub dry_run: bool,
    pub tag_filter: Option<String>,
    pub include_pending: bool,
    /// Command booted into freshly-created sessions. `None` creates the session
    /// but leaves it at a shell prompt (used by tests and `--no-launch`).
    pub launch_command: Option<String>,
}

// ── Entry point ───────────────────────────────────────────────────────────────

/// Provision every in-scope project and return one [`ProjectReport`] each.
///
/// Pure: prints nothing, so callers control presentation (the `setup spawn`
/// CLI prints a table; `pod heal --json` serializes the reports). Returns an
/// error only when an explicit `tag_filter` names a project absent from the
/// registry — a per-project failure is captured in that project's report.
pub async fn run(registry: &Registry, opts: &SpawnOptions) -> Result<Vec<ProjectReport>> {
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
        reports.push(provision_project(project, registry, opts).await);
    }

    Ok(reports)
}

// ── Per-project provisioning ──────────────────────────────────────────────────

async fn provision_project(
    project: &RegistryProject,
    registry: &Registry,
    opts: &SpawnOptions,
) -> ProjectReport {
    let session = project.session_name();
    let path = &project.path;
    let mut steps: Vec<StepReport> = Vec::new();

    // Step 1: verify / clone local repo. A real clone failure aborts the
    // project — the path won't exist for the steps that follow.
    if path.exists() {
        steps.push(StepReport::new(
            "clone",
            StepStatus::Present,
            Some(path.display().to_string()),
        ));
    } else if opts.dry_run {
        steps.push(StepReport::new(
            "clone",
            StepStatus::Planned,
            Some(format!("git clone git@github.com:{} → {}", project.repo, path.display())),
        ));
    } else {
        let clone_url = format!("git@github.com:{}", project.repo);
        let status = std::process::Command::new("git")
            .args(["clone", &clone_url, &path.to_string_lossy()])
            .status();
        match status {
            Ok(s) if s.success() => {
                steps.push(StepReport::new("clone", StepStatus::Done, Some(clone_url)));
            }
            Ok(s) => {
                steps.push(StepReport::new(
                    "clone",
                    StepStatus::Error,
                    Some(format!("git clone exited with {s}")),
                ));
                return ProjectReport::finish(project.tag.clone(), session, steps);
            }
            Err(e) => {
                steps.push(StepReport::new(
                    "clone",
                    StepStatus::Error,
                    Some(format!("git clone failed: {e}")),
                ));
                return ProjectReport::finish(project.tag.clone(), session, steps);
            }
        }
    }

    // Step 2: verify / generate per-repo .conductr.
    let dot_conductr = path.join(".conductr");
    if dot_conductr.exists() {
        steps.push(StepReport::new("dot-conductr", StepStatus::Present, None));
    } else if opts.dry_run {
        steps.push(StepReport::new(
            "dot-conductr",
            StepStatus::Planned,
            Some("generate from registry defaults".to_string()),
        ));
    } else {
        match generate_dot_conductr(project, registry) {
            Ok(()) => steps.push(StepReport::new(
                "dot-conductr",
                StepStatus::Done,
                Some("generated from defaults".to_string()),
            )),
            Err(e) => steps.push(StepReport::new(
                "dot-conductr",
                StepStatus::Error,
                Some(e.to_string()),
            )),
        }
    }

    // Step 3: run cadence sync. Idempotent — rewrites the host crontab markers.
    if opts.dry_run {
        steps.push(StepReport::new(
            "cadence",
            StepStatus::Planned,
            Some("cadence sync".to_string()),
        ));
    } else {
        match cadence::sync(path, false, cadence::Mechanism::Crontab) {
            Ok(report) => {
                let trimmed = report.trim();
                let detail = (!trimmed.is_empty()).then(|| trimmed.to_string());
                steps.push(StepReport::new("cadence", StepStatus::Done, detail));
            }
            Err(e) => steps.push(StepReport::new(
                "cadence",
                StepStatus::Error,
                Some(e.to_string()),
            )),
        }
    }

    // Step 4: verify / create the tmux session, and boot Claude into a freshly
    // created one. An *existing* session is never relaunched — that would
    // disrupt whatever is already running in it. The existence check is
    // read-only, so we run it even in dry-run to report `present` faithfully
    // (which lets `pod heal` route existing sessions to its restart pass).
    if check_tmux_session(&session) {
        steps.push(StepReport::new("session", StepStatus::Present, Some(session.clone())));
    } else if opts.dry_run {
        steps.push(StepReport::new(
            "session",
            StepStatus::Planned,
            Some(format!("create tmux session '{session}'")),
        ));
        if let Some(cmd) = &opts.launch_command {
            steps.push(StepReport::new("launch", StepStatus::Planned, Some(cmd.clone())));
        }
    } else {
        let cwd = path.to_string_lossy().into_owned();
        match create_tmux_session(&session, &cwd) {
            Ok(()) => {
                steps.push(StepReport::new(
                    "session",
                    StepStatus::Done,
                    Some(format!("created '{session}'")),
                ));
                if let Some(cmd) = &opts.launch_command {
                    match tmux_send_line(&session, cmd) {
                        Ok(()) => steps.push(StepReport::new(
                            "launch",
                            StepStatus::Done,
                            Some(cmd.clone()),
                        )),
                        Err(e) => steps.push(StepReport::new(
                            "launch",
                            StepStatus::Error,
                            Some(e.to_string()),
                        )),
                    }
                }
            }
            Err(e) => steps.push(StepReport::new(
                "session",
                StepStatus::Error,
                Some(e.to_string()),
            )),
        }
    }

    ProjectReport::finish(project.tag.clone(), session, steps)
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

/// Type `line` into `name`'s pane followed by Enter. Used to boot Claude into a
/// session right after creating it, so the pane is driveable rather than parked
/// at a shell prompt.
fn tmux_send_line(name: &str, line: &str) -> anyhow::Result<()> {
    let status = std::process::Command::new("tmux")
        .args(["send-keys", "-t", name, line, "Enter"])
        .status()
        .context("running tmux send-keys")?;
    if status.success() {
        Ok(())
    } else {
        anyhow::bail!("tmux send-keys exited with {status}");
    }
}

// ── Report ────────────────────────────────────────────────────────────────────

fn step_glyph(status: StepStatus) -> char {
    match status {
        StepStatus::Done => '+',
        StepStatus::Planned => '⋯',
        StepStatus::Present => '·',
        StepStatus::Error => '✗',
    }
}

pub fn print_report(reports: &[ProjectReport]) {
    if reports.is_empty() {
        return;
    }
    println!();
    println!("{:<20}  {}", "PROJECT", "STATUS");
    for r in reports {
        let summary = match &r.outcome {
            ProjectOutcome::AlreadyProvisioned => "✓ already provisioned".to_string(),
            ProjectOutcome::Provisioned => "↺ provisioned this pass".to_string(),
            ProjectOutcome::Skipped(reason) => format!("⚠ skipped ({reason})"),
        };
        println!("{:<20}  {summary}", r.tag);
        for s in &r.steps {
            match &s.detail {
                Some(d) => println!("  {} {:<14}  {d}", step_glyph(s.status), s.step),
                None => println!("  {} {}", step_glyph(s.status), s.step),
            }
        }
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

    fn opts(tag_filter: Option<String>, include_pending: bool) -> SpawnOptions {
        SpawnOptions {
            dry_run: true,
            tag_filter,
            include_pending,
            launch_command: None,
        }
    }

    #[tokio::test]
    async fn dry_run_against_fixture_produces_plan() {
        let reg = registry::parse(FIXTURE).unwrap();
        let reports = run(&reg, &opts(None, false)).await.unwrap();
        assert_eq!(reports.len(), 2, "only active projects processed");
        for r in &reports {
            assert!(
                matches!(r.outcome, ProjectOutcome::Provisioned),
                "dry-run: non-existent path should be Provisioned (would-clone)"
            );
            // Every step is a plan; the clone step would clone the missing path.
            assert_eq!(r.steps[0].step, "clone");
            assert_eq!(r.steps[0].status, StepStatus::Planned);
        }
    }

    #[tokio::test]
    async fn dry_run_includes_launch_step_when_command_set() {
        let reg = registry::parse(FIXTURE).unwrap();
        let mut o = opts(Some("alpha".to_string()), false);
        o.launch_command = Some(DEFAULT_LAUNCH_COMMAND.to_string());
        let reports = run(&reg, &o).await.unwrap();
        let launch = reports[0].steps.iter().find(|s| s.step == "launch");
        assert!(launch.is_some(), "launch step planned when command set");
        assert_eq!(launch.unwrap().status, StepStatus::Planned);
        assert_eq!(reports[0].session, "conductr-alpha");
    }

    #[tokio::test]
    async fn dry_run_omits_launch_step_without_command() {
        let reg = registry::parse(FIXTURE).unwrap();
        let reports = run(&reg, &opts(Some("alpha".to_string()), false)).await.unwrap();
        assert!(reports[0].steps.iter().all(|s| s.step != "launch"));
    }

    #[tokio::test]
    async fn dry_run_single_tag_filter() {
        let reg = registry::parse(FIXTURE).unwrap();
        let reports = run(&reg, &opts(Some("alpha".to_string()), false)).await.unwrap();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].tag, "alpha");
    }

    #[tokio::test]
    async fn dry_run_include_pending() {
        let reg = registry::parse(FIXTURE).unwrap();
        let reports = run(&reg, &opts(None, true)).await.unwrap();
        assert_eq!(reports.len(), 3, "all projects (active + pending) processed");
    }

    #[tokio::test]
    async fn unknown_tag_errors() {
        let reg = registry::parse(FIXTURE).unwrap();
        assert!(run(&reg, &opts(Some("nonexistent".to_string()), false)).await.is_err());
    }

    #[tokio::test]
    async fn report_serializes_to_stable_json_shape() {
        // `pod heal --json` consumes these reports, so the wire shape is a
        // contract: per-project `tag`/`session`/`outcome`/`steps`, each step a
        // `{step, status, detail?}` with kebab-case statuses.
        let reg = registry::parse(FIXTURE).unwrap();
        let reports = run(&reg, &opts(Some("alpha".to_string()), false)).await.unwrap();
        let v = serde_json::to_value(&reports[0]).unwrap();
        assert_eq!(v["tag"], "alpha");
        assert_eq!(v["session"], "conductr-alpha");
        assert_eq!(v["outcome"]["kind"], "provisioned");
        let clone = &v["steps"][0];
        assert_eq!(clone["step"], "clone");
        assert_eq!(clone["status"], "planned");
        assert!(clone["detail"].is_string());
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
