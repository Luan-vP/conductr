//! `conductr` CLI: orchestrate, instance, schedule, tasks, setup, mail, local, cadence, pod, architect, sync, idle.

mod cadence;
mod config;
mod idle;
mod local_detect;
mod parity;
mod registry;
mod setup_spawn;
mod tempo_profile;
mod wiring;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use conductr_adapters::gh_cli::GhCli;
use conductr_adapters::local_ci::LocalCiAdapter;
use conductr_adapters::mail_fs::FsMailbox;
use conductr_adapters::tmux::Tmux;
use conductr_adapters::{beads::Beads, notion::Notion};
use conductr_core::ports::{LocalCi, Mailbox, TmuxAgent};
use conductr_core::signals::MailKind;
use conductr_core::types::{CiMode, LocalCiConfig};
use conductr_mail::dedup::check_scope;
use conductr_mail::synthesise::request_synthesis;
use conductr_orchestrate::{Orchestrator, OrchestratorConfig, RepoSlug};
use conductr_pod::{
    diagnose_all, diagnose_one, ensure_session, heal_all, pick_idle, Diagnosis, FreeOpts, Health,
    SessionState, TmuxError,
};
use conductr_schedule::{parse, parse_plan, pattern_to_dsl, plan_to_pattern, render_ascii};
use conductr_setup::wizard::WizardMode;
use conductr_adapters::gcal::GcalAdapter;
use conductr_calendar::{reconcile, schedule_test_slot};
use conductr_tasks::sync::{create_task, list_tasks, sync_tasks};
use serde::Serialize;
use wiring::TrackerKind;

const BANNER: &str = r#"
                              ..,,,,,..
                          .,;;;;;;;;;;;;;;,.
                        ,;;;'            `;;;;,       ,,
                       ,;'                 ';;;;,    ;;;;
                      .;.;;;;,               ;;;;;.   ''
                      ;;;;;;;;                ;;;;;
                      `;;;;;;'                ;;;;;
                                              ;;;;'   ,,
                                            .;;;;'   ;;;;
                                           ,;;;'      ''
                                         ,;;;'
                                      ,;;;'
                                  .;;;;'
                              .,;;;''
                          .,;;''

                     |\                         __3__          |
____|\_______________|\\_______________|_______'__|__`___|_____|___|__________
____|/___3_|________@'_\|__|_____|_____|___|___|__|__|___|_|__@'___|___|___|__
___/|____-_|____________|__|_____|____@'___|__@'_@'_@'___|_|______@'___|___|__
__|_/_\__4_|___|_______@'__|____O'_________|____________O'_|__________@'___|__
___\|/_____|___|___________|_______________|_______________|_______________|__
    /         O'

                              c o n d u c t r
                scheduling and orchestration for agents and people
"#;

#[derive(Debug, Parser)]
#[command(
    name = "conductr",
    version,
    about = "Scheduling and orchestration for agents and people",
    before_help = BANNER,
    arg_required_else_help = true
)]
pub(crate) struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    /// Cadence configurator: write defaults or add a skill to `.conductr [cadence]`, then sync.
    Begin(BeginArgs),
    /// Drive `@claude` GitHub-issue orchestration.
    Orchestrate(OrchestrateArgs),
    /// Cloud instance management (stubbed).
    Instance(InstanceArgs),
    /// Musical-notation time patterns.
    Schedule(ScheduleArgs),
    /// Task tracking via beads (`br`) and Notion.
    Tasks(TasksArgs),
    /// Manage the local Claude Code pod (tmux sessions on this host).
    Pod(PodArgs),
    /// Check and improve repository maturity (setup wizard).
    Setup(SetupArgs),
    /// Agent mail: scope-dedup and synthesis bulletin board.
    Mail(MailArgs),
    /// Local AI agent providers: detect installed runtimes and models, install setup scripts.
    Local(LocalArgs),
    /// Sync the host crontab from `.conductr [cadence]`.
    Cadence(CadenceArgs),
    /// Dispatch a prompt to a local AI agent provider and print the response.
    RunTask(RunTaskArgs),
    /// Workspace-wide architectural review, or targeted review of a PR or issue (Claude-required).
    Architect(ArchitectArgs),
    /// Sync Google Calendar with the current issue state (decision + test slots).
    Sync(SyncArgs),
    /// Self-directed scan: architecture check + module clippy scan → file issues (Claude-required).
    Idle(IdleArgs),
    /// Manual diff impact report: topology / LOC % / user flows / compliance / tech debt (Claude-required).
    ChangeOverview(ChangeOverviewArgs),
    /// Draft answers to topically-relevant open `human` issues against a change context (Claude-required).
    HumanTicketDraft(HumanTicketDraftArgs),
}

#[derive(Debug, Parser)]
struct CadenceArgs {
    #[command(subcommand)]
    cmd: CadenceCmd,
}

#[derive(Debug, Subcommand)]
enum CadenceCmd {
    /// Apply or update crontab or LaunchAgent entries from `.conductr [cadence]`.
    Sync(CadenceSyncArgs),
    /// Show realized schedule state and drift from `.conductr [cadence]`.
    Status(CadenceStatusArgs),
    /// Remove installed schedules recorded in `.conductr-local`.
    Remove(CadenceRemoveArgs),
    /// Render the cadence schedule as a 24-hour staff with a current-time marker.
    Show(CadenceShowArgs),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum CadenceMechanism {
    Crontab,
    Launchd,
}

impl From<CadenceMechanism> for cadence::Mechanism {
    fn from(m: CadenceMechanism) -> Self {
        match m {
            CadenceMechanism::Crontab => cadence::Mechanism::Crontab,
            CadenceMechanism::Launchd => cadence::Mechanism::Launchd,
        }
    }
}

#[derive(Debug, Parser)]
struct CadenceSyncArgs {
    /// Path to the repo containing `.conductr` (defaults to current directory).
    #[arg(long)]
    repo: Option<PathBuf>,
    /// Print the planned changes without writing anything.
    #[arg(long)]
    dry_run: bool,
    /// Scheduling mechanism: crontab (default) or launchd (macOS).
    #[arg(long, value_enum, default_value = "crontab")]
    mechanism: CadenceMechanism,
}

#[derive(Debug, Parser)]
struct CadenceStatusArgs {
    /// Path to the repo containing `.conductr` (defaults to current directory).
    #[arg(long)]
    repo: Option<PathBuf>,
}

#[derive(Debug, Parser)]
struct CadenceRemoveArgs {
    /// Path to the repo containing `.conductr` (defaults to current directory).
    #[arg(long)]
    repo: Option<PathBuf>,
    /// Print the planned removal without executing it.
    #[arg(long)]
    dry_run: bool,
}

#[derive(Debug, Parser)]
struct CadenceShowArgs {
    /// Path to the repo containing `.conductr` (defaults to current directory).
    #[arg(long)]
    repo: Option<PathBuf>,
    /// Render as if it were this UTC instant (ISO-8601, e.g. `2026-05-10T14:23:00Z`).
    #[arg(long)]
    at: Option<String>,
    /// Show every conductr cron entry across all project tags (reads the live crontab).
    #[arg(long)]
    all: bool,
}

#[derive(Debug, Parser)]
struct BeginArgs {
    /// Skill to add to `[cadence]`. Omit to write defaults (`orchestrate` + `idle`).
    skill: Option<String>,
    /// Cron expression (e.g. `*/30 * * * *`). Only used with `<skill>`. Defaults to `*/30 * * * *`.
    schedule: Option<String>,
    /// Path to the repo root containing `.conductr` (defaults to current directory).
    #[arg(long)]
    repo: Option<PathBuf>,
    /// Print the planned changes without writing anything or running cadence sync.
    #[arg(long)]
    dry_run: bool,
}

/// CI mode override — maps to `conductr_core::types::CiMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum CiModeArg {
    Local,
    PreferLocal,
    PreferGithub,
    Github,
}

impl From<CiModeArg> for CiMode {
    fn from(m: CiModeArg) -> Self {
        match m {
            CiModeArg::Local => CiMode::Local,
            CiModeArg::PreferLocal => CiMode::PreferLocal,
            CiModeArg::PreferGithub => CiMode::PreferGithub,
            CiModeArg::Github => CiMode::Github,
        }
    }
}

#[derive(Debug, Parser)]
struct OrchestrateArgs {
    /// `owner/repo` slug.
    #[arg(long)]
    repo: String,
    /// Print the plan without commenting or merging.
    #[arg(long)]
    dry_run: bool,
    /// Run a single cycle and exit.
    #[arg(long)]
    once: bool,
    /// Polling interval in seconds between cycles.
    #[arg(long, default_value_t = 60)]
    poll_secs: u64,
    /// Default human assignee when an issue is `human`-labelled.
    #[arg(long)]
    human_assignee: Option<String>,
    /// CI mode override. Overrides `[ci].mode` in `.conductr` for this run.
    #[arg(long, value_enum)]
    ci: Option<CiModeArg>,
    /// Path to the repo root for reading `.conductr` config (defaults to cwd).
    #[arg(long)]
    repo_root: Option<PathBuf>,
}

#[derive(Debug, Parser)]
struct InstanceArgs {
    #[command(subcommand)]
    cmd: InstanceCmd,
}

#[derive(Debug, Subcommand)]
enum InstanceCmd {
    SpinUp { #[arg(long)] name: String },
    List,
}

#[derive(Debug, Parser)]
struct ScheduleArgs {
    #[command(subcommand)]
    cmd: ScheduleCmd,
}

#[derive(Debug, Subcommand)]
enum ScheduleCmd {
    /// Validate a pattern file.
    Validate { path: PathBuf },
    /// Render a pattern file as an ASCII timeline.
    Render { path: PathBuf },
    /// Convert a plan markdown file to a schedule pattern DSL.
    FromPlan {
        path: PathBuf,
        /// Also render the ASCII timeline after the DSL.
        #[arg(long)]
        render: bool,
    },
}

#[derive(Debug, Parser)]
struct TasksArgs {
    #[command(subcommand)]
    cmd: TasksCmd,
}

#[derive(Debug, Parser)]
struct PodArgs {
    #[command(subcommand)]
    cmd: PodCmd,
}

#[derive(Debug, Subcommand)]
enum PodCmd {
    /// Inspect the local Claude Code pod (tmux sessions on this host).
    Diagnose(DiagnoseArgs),
    /// Find an idle Claude Code session and print its tmux attach command.
    Free(FreeArgs),
    /// Restart any crashed Claude Code sessions in the pod.
    Heal(HealArgs),
    /// Snapshot unfinished work to beads then restart pod sessions.
    SaveState(SaveStateArgs),
    /// Ensure the named project tmux session exists and has Claude running.
    /// Idempotent: a no-op if the session is already live and idle.
    EnsureSession(EnsureSessionArgs),
}

async fn run_pod(args: PodArgs) -> Result<()> {
    match args.cmd {
        PodCmd::Diagnose(a) => run_diagnose(a).await,
        PodCmd::Free(a) => run_free(a).await,
        PodCmd::Heal(a) => run_heal(a).await,
        PodCmd::SaveState(a) => run_save_state(a).await,
        PodCmd::EnsureSession(a) => run_ensure_session(a).await,
    }
}

#[derive(Debug, Parser)]
struct DiagnoseArgs {
    /// Substring to filter session names by (default: `claude`).
    /// Pass `--all` to inspect every tmux session.
    #[arg(long)]
    pattern: Option<String>,
    /// Inspect every tmux session, not just the Claude pod.
    #[arg(long)]
    all: bool,
    /// Emit machine-readable JSON instead of a table.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Parser)]
struct FreeArgs {
    /// Substring to filter session names by (default: `claude`).
    #[arg(long)]
    pattern: Option<String>,
    /// Consider every tmux session, not just the Claude pod.
    #[arg(long)]
    all: bool,
    /// Allow picking a session that already has an attached client.
    #[arg(long)]
    include_attached: bool,
    /// Emit machine-readable JSON instead of the plain attach command.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Parser)]
struct HealArgs {
    /// Substring to filter session names by (default: `claude`).
    #[arg(long)]
    pattern: Option<String>,
    /// Heal every tmux session, not just the Claude pod.
    #[arg(long)]
    all: bool,
    /// Show the plan without sending keys.
    #[arg(long)]
    dry_run: bool,
    /// Command typed into the pane to bring Claude back up. Feeds both the
    /// provision pass's session launch and the restart pass. Defaults to Remote
    /// Control + `auto` permission mode so restored sessions are driveable.
    #[arg(long)]
    command: Option<String>,
    /// Emit machine-readable JSON instead of a table.
    #[arg(long)]
    json: bool,
    /// Scope all passes to a single project by `owner/name` slug. Errors if the
    /// slug is not an `active` project in `~/.conductr`.
    #[arg(long)]
    repo: Option<String>,
    /// Skip the idempotent provision pass (clone / `.conductr` / cron / spawn)
    /// and only run the restart pass on already-existing sessions.
    #[arg(long)]
    no_provision: bool,
    /// Path to the registry file (defaults to `~/.conductr`).
    #[arg(long)]
    registry: Option<PathBuf>,
}

#[derive(Debug, Parser)]
struct SaveStateArgs {
    /// Substring to filter session names by (default: `claude`).
    #[arg(long)]
    pattern: Option<String>,
    /// Cover every tmux session, not just the Claude pod.
    #[arg(long)]
    all: bool,
    /// Print the plan without writing to the tracker or restarting sessions.
    #[arg(long)]
    dry_run: bool,
    /// Do not restart sessions after capturing state.
    #[arg(long)]
    no_restart: bool,
    /// Command typed into the pane to bring Claude back up after restart.
    #[arg(long, default_value = "claude")]
    command: String,
    /// Priority for created tracker issues (0-4).
    #[arg(long, default_value_t = 2)]
    priority: u8,
    /// Always emit the JSON manifest of saved state (default behaviour).
    #[arg(long, default_value_t = true)]
    json: bool,
    /// Issue tracker to write recovery issues to.
    #[arg(long, value_enum, default_value = "beads")]
    tracker: TrackerKind,
    /// Notion database ID for recovery issues (required with `--tracker notion`).
    /// Can also be set via `CONDUCTR_NOTION_DATABASE`.
    #[arg(long, env = "CONDUCTR_NOTION_DATABASE")]
    notion_database: Option<String>,
}

#[derive(Debug, Parser)]
struct EnsureSessionArgs {
    /// Project tag. The tmux session is named `conductr-<tag>`.
    tag: String,
    /// Working directory for the tmux session (defaults to current directory).
    #[arg(long)]
    cwd: Option<PathBuf>,
    /// Command used to start the AI agent.
    #[arg(long = "claude-cmd", default_value = "claude --dangerously-skip-permissions")]
    claude_cmd: String,
    /// Command to run once the session is ready.
    /// A `/`-prefixed command is sent as keys to the Claude session (skill dispatch).
    /// Any other command is run as a subprocess (e.g. `conductr orchestrate --repo X --once`).
    #[arg(long = "then")]
    then_cmd: Option<String>,
    /// Print the plan without creating sessions or running commands.
    #[arg(long)]
    dry_run: bool,
}

#[derive(Debug, Subcommand)]
enum TasksCmd {
    /// List all tasks (or just ready tasks with --ready).
    List {
        #[arg(long)]
        ready: bool,
    },
    /// Create a new task.
    Create {
        title: String,
        #[arg(short, long)]
        priority: Option<u8>,
    },
    /// Push beads tasks to a Notion database.
    SyncToNotion {
        #[arg(long)]
        database: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Begin(a) => run_begin(a).await,
        Cmd::Orchestrate(a) => run_orchestrate(a).await,
        Cmd::Instance(a) => run_instance(a).await,
        Cmd::Schedule(a) => run_schedule(a),
        Cmd::Tasks(a) => run_tasks(a).await,
        Cmd::Pod(a) => run_pod(a).await,
        Cmd::Setup(a) => run_setup(a).await,
        Cmd::Mail(a) => run_mail(a).await,
        Cmd::Local(a) => run_local(a).await,
        Cmd::Cadence(a) => run_cadence(a),
        Cmd::RunTask(a) => run_run_task(a).await,
        Cmd::Architect(a) => run_architect(a).await,
        Cmd::Sync(a) => run_sync(a).await,
        Cmd::Idle(a) => run_idle(a).await,
        Cmd::ChangeOverview(a) => run_change_overview(a).await,
        Cmd::HumanTicketDraft(a) => run_human_ticket_draft(a).await,
    }
}

fn pod_pattern<'a>(explicit: Option<&'a str>, all: bool) -> Option<&'a str> {
    if all {
        None
    } else {
        explicit.or(Some("conductr-"))
    }
}

// ---------------------------------------------------------------------------
// begin
// ---------------------------------------------------------------------------

/// Top-level conductr subcommands that can be scheduled via cadence.
/// Validated against the parity predicate (CLI command ↔ skill symmetry).
const SCHEDULABLE_SKILLS: &[&str] = &[
    "orchestrate", "idle", "architect", "instance", "schedule",
    "tasks", "pod", "setup", "mail", "local", "run-task",
];

const DEFAULT_SCHEDULE: &str = "*/30 * * * *";

async fn run_begin(args: BeginArgs) -> Result<()> {
    let repo = args.repo.unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

    if let Some(skill) = &args.skill {
        // Form 2: conductr begin <skill> [schedule]
        if !SCHEDULABLE_SKILLS.contains(&skill.as_str()) {
            anyhow::bail!(
                "'{skill}' is not a known schedulable conductr command.\n\
                 Valid skills: {}",
                SCHEDULABLE_SKILLS.join(", ")
            );
        }
        let schedule = args.schedule.as_deref().unwrap_or(DEFAULT_SCHEDULE);
        if args.dry_run {
            println!("plan: would add '{skill} = \"{schedule}\"' to .conductr [cadence]");
            println!("plan: would run cadence sync");
            return Ok(());
        }
        let added = upsert_cadence_entry(&repo.join(".conductr"), skill, schedule)?;
        if added {
            println!("begin: added '{skill}' to [cadence] with schedule '{schedule}'");
        } else {
            println!("begin: '{skill}' already present in [cadence]; schedule unchanged");
        }
    } else {
        // Form 1: conductr begin (no args) — write defaults
        if args.dry_run {
            println!("plan: would write defaults (orchestrate, idle) to .conductr [cadence] if absent");
            println!("plan: would run cadence sync");
            return Ok(());
        }
        if !repo.join(".conductr").exists() {
            if let Err(e) = config::ensure_dot_conductr(&repo) {
                anyhow::bail!("begin: could not init .conductr in {}: {e}", repo.display());
            }
        }
        let conductr_path = repo.join(".conductr");
        let added_orchestrate = upsert_cadence_entry(&conductr_path, "orchestrate", DEFAULT_SCHEDULE)?;
        let added_idle = upsert_cadence_entry(&conductr_path, "idle", DEFAULT_SCHEDULE)?;
        if added_orchestrate {
            println!("begin: added 'orchestrate' to [cadence] with schedule '{DEFAULT_SCHEDULE}'");
        }
        if added_idle {
            println!("begin: added 'idle' to [cadence] with schedule '{DEFAULT_SCHEDULE}'");
        }
        if !added_orchestrate && !added_idle {
            println!("begin: orchestrate and idle already present in [cadence]");
        }
    }

    let report = cadence::sync(&repo, false, cadence::Mechanism::Crontab)?;
    println!("{report}");
    Ok(())
}

/// Add or verify a `key = "value"` entry in the `[cadence]` section of `.conductr`.
/// Returns `true` if the entry was added, `false` if it was already present.
fn upsert_cadence_entry(conductr_path: &std::path::Path, skill: &str, schedule: &str) -> Result<bool> {
    let content = if conductr_path.exists() {
        std::fs::read_to_string(conductr_path)
            .with_context(|| format!("reading {}", conductr_path.display()))?
    } else {
        anyhow::bail!(
            "{} does not exist; run `conductr begin` with a valid repo root",
            conductr_path.display()
        );
    };

    // Check if already present via TOML parse.
    let existing: toml::Value = toml::from_str(&content)
        .with_context(|| format!("parsing {}", conductr_path.display()))?;
    if let Some(toml::Value::Table(cadence)) = existing.get("cadence") {
        if cadence.contains_key(skill) {
            return Ok(false);
        }
    }

    let new_line = format!("{skill} = \"{schedule}\"");
    let new_content = insert_into_cadence_section(&content, &new_line);
    std::fs::write(conductr_path, new_content)
        .with_context(|| format!("writing {}", conductr_path.display()))?;
    Ok(true)
}

/// Insert `new_line` into the `[cadence]` section of a `.conductr` TOML string.
/// If no `[cadence]` section exists, appends one at the end.
fn insert_into_cadence_section(content: &str, new_line: &str) -> String {
    let lines: Vec<String> = content.lines().map(str::to_string).collect();
    let cadence_idx = lines.iter().position(|l| l.trim() == "[cadence]");

    let mut out: Vec<String> = lines;

    if let Some(idx) = cadence_idx {
        // Find end of [cadence] section: the next bare section header or EOF.
        let end = out[idx + 1..]
            .iter()
            .position(|l| {
                let t = l.trim();
                t.starts_with('[') && !t.starts_with("[[") && !t.is_empty()
            })
            .map(|i| idx + 1 + i)
            .unwrap_or(out.len());
        out.insert(end, new_line.to_string());
    } else {
        // No [cadence] section — append a blank line, header, and the entry.
        if out.last().map_or(false, |l| !l.is_empty()) {
            out.push(String::new());
        }
        out.push("[cadence]".to_string());
        out.push(new_line.to_string());
    }

    let mut result = out.join("\n");
    if !result.ends_with('\n') {
        result.push('\n');
    }
    result
}

async fn wait_for_idle(tmux: &Tmux, session: &str) -> Result<()> {
    use std::time::{Duration, Instant};
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        tokio::time::sleep(Duration::from_secs(3)).await;
        if let Ok(d) = diagnose_one(tmux, session).await {
            if matches!(d.health, Health::Idle { .. }) {
                return Ok(());
            }
        }
        if Instant::now() > deadline {
            anyhow::bail!("timed out waiting for Claude to become idle in session '{session}'");
        }
    }
}

async fn run_diagnose(args: DiagnoseArgs) -> Result<()> {
    let tmux = Tmux::new();
    let pattern = pod_pattern(args.pattern.as_deref(), args.all);
    let diagnoses = diagnose_all(&tmux, pattern).await?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&diagnoses)?);
        return Ok(());
    }
    if diagnoses.is_empty() {
        println!("no sessions matched (pattern={:?})", pattern.unwrap_or("*"));
        return Ok(());
    }
    println!("{:<20} {:<9} {:>8}  {}", "SESSION", "HEALTH", "IDLE", "DETAIL");
    for d in &diagnoses {
        let (label, detail) = match &d.health {
            Health::Idle { last_message, tokens } => (
                "idle",
                format!(
                    "{}{}",
                    last_message.as_deref().unwrap_or("(awaiting first prompt)"),
                    tokens.as_ref().map(|t| format!("  ·  {t}")).unwrap_or_default(),
                ),
            ),
            Health::Working { activity } => ("working", activity.clone()),
            Health::Crashed { last_shell_line } => (
                "crashed",
                last_shell_line.clone().unwrap_or_else(|| "(shell)".into()),
            ),
            Health::Unknown { reason } => ("unknown", reason.clone()),
        };
        println!(
            "{:<20} {:<9} {:>7}s  {}",
            d.session.name,
            label,
            d.idle_seconds,
            truncate(&detail, 80),
        );
    }
    Ok(())
}

async fn run_free(args: FreeArgs) -> Result<()> {
    let tmux = Tmux::new();
    let pattern = pod_pattern(args.pattern.as_deref(), args.all);
    let diagnoses = diagnose_all(&tmux, pattern).await?;

    let opts = FreeOpts { include_attached: args.include_attached };
    match pick_idle(&diagnoses, &opts) {
        Some(d) => {
            let cmd = format!("tmux attach -t {}", d.session.name);
            if args.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "session": d.session.name,
                        "command": cmd,
                    }))?
                );
            } else {
                println!("{cmd}");
            }
            Ok(())
        }
        None => {
            let reason = if diagnoses.is_empty() {
                format!("no sessions matched (pattern={:?})", pattern.unwrap_or("*"))
            } else {
                let working = diagnoses.iter().filter(|d| matches!(d.health, Health::Working { .. })).count();
                let crashed = diagnoses.iter().filter(|d| matches!(d.health, Health::Crashed { .. })).count();
                let idle_attached = diagnoses
                    .iter()
                    .filter(|d| matches!(d.health, Health::Idle { .. }) && d.session.attached)
                    .count();
                let mut parts: Vec<String> = Vec::new();
                if working > 0 {
                    parts.push(format!("{working} working"));
                }
                if crashed > 0 {
                    parts.push(format!("{crashed} crashed"));
                }
                if idle_attached > 0 && !args.include_attached {
                    parts.push(format!("{idle_attached} idle but attached"));
                }
                if parts.is_empty() {
                    "no idle sessions".into()
                } else {
                    format!("no idle session in pod ({})", parts.join(", "))
                }
            };
            eprintln!("{reason}");
            std::process::exit(1);
        }
    }
}

/// The registry-derived scope of a heal invocation.
#[derive(Debug)]
struct HealScope {
    registry: registry::Registry,
    /// `Some(tag)` provisions a single project; `None` provisions all active.
    tag_filter: Option<String>,
    /// `Some(session)` when `--repo` is set: the restart pass is exact-name
    /// matched on this `conductr-<tag>` so sibling sessions aren't collateral.
    scoped_session: Option<String>,
}

/// Resolve which projects this heal invocation provisions.
///
/// - No `--repo`, no registry → `None`: heal degrades to its old "just restart
///   crashed sessions" behaviour (no provision pass).
/// - No `--repo` with a registry → all `active` projects.
/// - `--repo <slug>` → the single matching `active` project, or an error if the
///   slug is missing / `pending` / the registry is absent.
fn resolve_heal_scope(repo: Option<&str>, registry_path: Option<&Path>) -> Result<Option<HealScope>> {
    let reg = match registry::load(registry_path) {
        Ok(r) => r,
        Err(e) => {
            if repo.is_some() {
                return Err(e.context("--repo requires a readable ~/.conductr registry"));
            }
            // No registry: provision pass is a no-op, restart pass still runs.
            return Ok(None);
        }
    };

    match repo {
        None => Ok(Some(HealScope { registry: reg, tag_filter: None, scoped_session: None })),
        Some(slug) => match reg.find_by_repo(slug) {
            Some(p) if p.status == registry::ProjectStatus::Active => {
                let scope = HealScope {
                    tag_filter: Some(p.tag.clone()),
                    scoped_session: Some(p.session_name()),
                    registry: reg,
                };
                Ok(Some(scope))
            }
            Some(_) => anyhow::bail!(
                "project '{slug}' is pending — promote it with `status = \"active\"` in ~/.conductr"
            ),
            None => anyhow::bail!("no active project with repo '{slug}' in ~/.conductr"),
        },
    }
}

async fn run_heal(args: HealArgs) -> Result<()> {
    let tmux = Tmux::new();
    let command = args
        .command
        .clone()
        .unwrap_or_else(|| setup_spawn::DEFAULT_LAUNCH_COMMAND.to_string());

    let scope = resolve_heal_scope(args.repo.as_deref(), args.registry.as_deref())?;

    // Pass 1 — idempotent provision (clone / .conductr / cron / spawn + launch),
    // delegated wholesale to `setup spawn`. Skipped when there's no registry
    // scope or `--no-provision` is set.
    let provision: Vec<setup_spawn::ProjectReport> = match &scope {
        Some(s) if !args.no_provision => {
            let opts = setup_spawn::SpawnOptions {
                dry_run: args.dry_run,
                tag_filter: s.tag_filter.clone(),
                include_pending: false,
                launch_command: Some(command.clone()),
            };
            setup_spawn::run(&s.registry, &opts).await?
        }
        _ => Vec::new(),
    };

    // Sessions the provision pass just created already had `command` launched
    // into them. Exclude them from the restart pass so their not-yet-rendered
    // panes aren't misread as crashed and double-launched.
    let freshly_launched: std::collections::HashSet<&str> = provision
        .iter()
        .filter(|p| {
            p.steps.iter().any(|s| {
                s.step == "launch"
                    && matches!(
                        s.status,
                        setup_spawn::StepStatus::Done | setup_spawn::StepStatus::Planned
                    )
            })
        })
        .map(|p| p.session.as_str())
        .collect();

    // Pass 2 — restart/liveness. With `--repo`, the pattern is the exact scoped
    // session; otherwise the usual pod pattern.
    let scoped_session = scope.as_ref().and_then(|s| s.scoped_session.clone());
    let pattern: Option<&str> = match scoped_session.as_deref() {
        Some(s) => Some(s),
        None => pod_pattern(args.pattern.as_deref(), args.all),
    };
    let heal: Vec<_> = heal_all(&tmux, pattern, &command, args.dry_run)
        .await?
        .into_iter()
        .filter(|o| !freshly_launched.contains(o.plan.session.as_str()))
        // When scoped, substring matching is too loose (`conductr-foo` would
        // also catch `conductr-foo-bar`); require an exact session-name match.
        .filter(|o| match scoped_session.as_deref() {
            Some(exact) => o.plan.session == exact,
            None => true,
        })
        .collect();

    if args.json {
        let manifest = serde_json::json!({ "provision": provision, "heal": heal });
        println!("{}", serde_json::to_string_pretty(&manifest)?);
        return Ok(());
    }

    if !provision.is_empty() {
        println!("provision pass:");
        setup_spawn::print_report(&provision);
        println!();
    }

    println!("restart pass:");
    if heal.is_empty() {
        println!("  no sessions matched (pattern={:?})", pattern.unwrap_or("*"));
        return Ok(());
    }
    let mut healed = 0usize;
    let mut skipped = 0usize;
    for o in &heal {
        match (&o.plan.action, o.executed, o.error.as_deref()) {
            (conductr_pod::heal::HealAction::RestartClaude { command }, true, None) => {
                healed += 1;
                println!("  ↻ {:<20}  sent `{command}`", o.plan.session);
            }
            (conductr_pod::heal::HealAction::RestartClaude { command }, false, None) => {
                println!("  ⋯ {:<20}  would send `{command}` (dry-run)", o.plan.session);
            }
            (conductr_pod::heal::HealAction::RestartClaude { command }, _, Some(err)) => {
                println!("  ✗ {:<20}  failed to send `{command}`: {err}", o.plan.session);
            }
            (conductr_pod::heal::HealAction::Skip { reason }, _, _) => {
                skipped += 1;
                println!("  · {:<20}  skip — {reason}", o.plan.session);
            }
        }
    }
    println!("\n{healed} restarted, {skipped} skipped, {} total", heal.len());
    Ok(())
}

/// One entry per session in the save-state manifest. The skill consumes this
/// JSON to update external trackers before restart.
#[derive(Debug, Serialize)]
struct SaveStateEntry {
    session: String,
    health: &'static str,
    cwd: Option<String>,
    last_message: Option<String>,
    tail: Vec<String>,
    /// Which tracker wrote the recovery issue: `"beads"` or `"notion"`.
    tracker: String,
    /// ID of the tracker issue created for this session (None if no
    /// recoverable work or `--dry-run`).
    tracker_id: Option<String>,
    /// What we did to the pane: `restarted`, `would-restart`, `skipped:<why>`.
    restart: String,
}

async fn run_save_state(args: SaveStateArgs) -> Result<()> {
    let tmux = Tmux::new();
    let pattern = pod_pattern(args.pattern.as_deref(), args.all);
    let diagnoses = diagnose_all(&tmux, pattern).await?;

    let tracker_kind = args.tracker;
    let tracker_label = tracker_kind.as_str().to_string();
    // Only construct the tracker when we'll actually call it.
    let tracker = if args.dry_run {
        None
    } else {
        Some(wiring::issue_tracker(tracker_kind, args.notion_database)?)
    };

    let mut manifest: Vec<SaveStateEntry> = Vec::with_capacity(diagnoses.len());

    for d in &diagnoses {
        let recoverable = recoverable_summary(d);
        let tracker_id = if let Some(summary) = &recoverable {
            if let Some(t) = &tracker {
                Some(create_recovery_issue(t.as_ref(), d, summary, args.priority).await?)
            } else {
                None
            }
        } else {
            None
        };

        let restart = if args.no_restart {
            "skipped:no-restart-flag".to_string()
        } else if args.dry_run {
            restart_label_dry(&d.health)
        } else {
            restart_session(&tmux, d, &args.command).await?
        };

        manifest.push(SaveStateEntry {
            session: d.session.name.clone(),
            health: health_label(&d.health),
            cwd: d.session.cwd.clone(),
            last_message: recoverable,
            tail: d.tail.clone(),
            tracker: tracker_label.clone(),
            tracker_id,
            restart,
        });
    }

    println!("{}", serde_json::to_string_pretty(&manifest)?);
    Ok(())
}

// ── ensure-session ────────────────────────────────────────────────────────────

async fn run_ensure_session(args: EnsureSessionArgs) -> Result<()> {
    let session = format!("conductr-{}", args.tag);
    let cwd = args
        .cwd
        .map(|p| p.to_string_lossy().into_owned())
        .or_else(|| std::env::current_dir().ok().map(|p| p.to_string_lossy().into_owned()))
        .unwrap_or_else(|| ".".to_string());
    let claude_cmd = &args.claude_cmd;

    if args.dry_run {
        let tmux = Tmux::new();
        return run_ensure_session_dry(
            &tmux,
            &session,
            &cwd,
            claude_cmd,
            args.then_cmd.as_deref(),
        )
        .await;
    }

    let tmux = Tmux::new();
    let state = ensure_session(&tmux, &session, &cwd)
        .await
        .with_context(|| format!("ensuring tmux session '{session}'"))?;

    let ready = match &state {
        SessionState::Existing(Health::Working { activity }) => {
            println!("ensure-session: target_stale_or_not_consuming (working: {activity})");
            false
        }
        SessionState::Existing(Health::Unknown { reason }) => {
            println!("ensure-session: target_stale_or_not_consuming (unknown: {reason})");
            false
        }
        SessionState::Existing(Health::Crashed { .. }) => {
            println!("ensure-session: session '{session}' crashed — restarting Claude");
            tmux.send_line(&session, claude_cmd)
                .await
                .context("restarting Claude after crash")?;
            wait_for_idle(&tmux, &session).await?;
            println!("ensure-session: session_missing_created");
            true
        }
        SessionState::Created => {
            println!("ensure-session: session_missing_created — starting Claude in '{session}'");
            tmux.send_line(&session, claude_cmd)
                .await
                .context("starting Claude in new session")?;
            wait_for_idle(&tmux, &session).await?;
            true
        }
        SessionState::Existing(Health::Idle { .. }) => {
            println!("ensure-session: session_reused");
            true
        }
    };

    if let Some(then_cmd) = &args.then_cmd {
        if !ready {
            return Ok(());
        }
        if then_cmd.starts_with('/') {
            // Skill command — dispatch into the live Claude session.
            tmux.send_line(&session, then_cmd)
                .await
                .with_context(|| format!("sending '{then_cmd}' to session '{session}'"))?;
            println!("ensure-session: prompt_written_to_target_pane");
        } else {
            // External command — run as a subprocess.
            println!("ensure-session: subprocess_started: {then_cmd}");
            let status = std::process::Command::new("bash")
                .arg("-c")
                .arg(then_cmd)
                .status()
                .with_context(|| format!("running subprocess: {then_cmd}"))?;
            if !status.success() {
                anyhow::bail!("--then subprocess exited with status {status}: {then_cmd}");
            }
        }
    }

    Ok(())
}

async fn run_ensure_session_dry(
    tmux: &Tmux,
    session: &str,
    cwd: &str,
    claude_cmd: &str,
    then_cmd: Option<&str>,
) -> Result<()> {
    let sessions = match tmux.list_sessions().await {
        Ok(s) => s,
        Err(TmuxError::NoServer) | Err(TmuxError::NotInstalled) => {
            println!("ensure-session: tmux not running");
            println!("ensure-session: → would create session '{session}' at cwd={cwd}");
            println!("ensure-session: → would start Claude: `{claude_cmd}`");
            if let Some(cmd) = then_cmd {
                println!("ensure-session: → would run: `{cmd}`");
            }
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };

    if sessions.iter().any(|s| s.name == session) {
        println!("ensure-session: session '{session}' already exists");
        if let Some(cmd) = then_cmd {
            println!("ensure-session: → would run: `{cmd}`");
        }
    } else {
        println!("ensure-session: session '{session}' does not exist");
        println!("ensure-session: → would create session at cwd={cwd}");
        println!("ensure-session: → would start Claude: `{claude_cmd}`");
        if let Some(cmd) = then_cmd {
            println!("ensure-session: → would run: `{cmd}`");
        }
    }
    Ok(())
}

fn recoverable_summary(d: &Diagnosis) -> Option<String> {
    match &d.health {
        Health::Idle { last_message, .. } => {
            last_message.as_deref().filter(|m| !is_slash_command(m)).map(str::to_string)
        }
        Health::Working { activity } => Some(activity.clone()),
        Health::Crashed { last_shell_line } => Some(format!(
            "crashed at: {}",
            last_shell_line.as_deref().unwrap_or("(no output)")
        )),
        Health::Unknown { .. } => None,
    }
}

fn is_slash_command(msg: &str) -> bool {
    // Pure slash-commands like `/clear`, `/exit`, `/reload-plugins` are
    // session-mgmt noise, not unfinished work. We only treat a message as
    // a real task if it has substantive prose alongside a slash command.
    let trimmed = msg.trim();
    trimmed.starts_with('/') && !trimmed.contains(' ')
}

fn health_label(h: &Health) -> &'static str {
    match h {
        Health::Idle { .. } => "idle",
        Health::Working { .. } => "working",
        Health::Crashed { .. } => "crashed",
        Health::Unknown { .. } => "unknown",
    }
}

async fn create_recovery_issue(
    tracker: &dyn conductr_core::ports::IssueTracker,
    d: &Diagnosis,
    summary: &str,
    priority: u8,
) -> Result<String> {
    let title = format!(
        "[thread-recovery:{}] {}",
        d.session.name,
        truncate(summary, 80),
    );
    let body = format!(
        "Captured by `conductr pod save-state` at {now}.\n\n\
         - session: `{name}`\n\
         - health: `{health}`\n\
         - cwd: `{cwd}`\n\
         - last activity: {activity}s idle\n\n\
         Last message / activity:\n\n```\n{summary}\n```\n\n\
         Pane tail:\n\n```\n{tail}\n```\n",
        now = chrono::Utc::now().to_rfc3339(),
        name = d.session.name,
        health = health_label(&d.health),
        cwd = d.session.cwd.as_deref().unwrap_or(""),
        activity = d.idle_seconds,
        summary = summary,
        tail = d.tail.join("\n"),
    );
    let labels = ["thread-recovery", d.session.name.as_str()];
    let task = create_task(tracker, &title, Some(priority), Some(&body), &labels)
        .await
        .with_context(|| format!("creating beads recovery issue for {}", d.session.name))?;
    Ok(task.id)
}

fn restart_label_dry(h: &Health) -> String {
    match h {
        Health::Idle { .. } => "would-restart:exit-then-relaunch".into(),
        Health::Working { .. } => "would-skip:agent-busy".into(),
        Health::Crashed { .. } => "would-restart:relaunch".into(),
        Health::Unknown { .. } => "would-skip:unclassified".into(),
    }
}

async fn restart_session(tmux: &Tmux, d: &Diagnosis, command: &str) -> Result<String> {
    match &d.health {
        Health::Working { .. } => Ok("skipped:agent-busy".into()),
        Health::Unknown { reason } => Ok(format!("skipped:unclassified-{reason}")),
        Health::Crashed { .. } => {
            tmux.send_line(&d.session.name, command).await?;
            Ok("restarted:relaunch".into())
        }
        Health::Idle { .. } => {
            // /exit cleanly terminates Claude Code back to the shell, then we
            // re-launch. A 400ms gap gives the TUI time to tear down before
            // we type the next command.
            tmux.send_line(&d.session.name, "/exit").await?;
            tokio::time::sleep(std::time::Duration::from_millis(400)).await;
            tmux.send_line(&d.session.name, command).await?;
            Ok("restarted:exit-then-relaunch".into())
        }
    }
}

// ── Mail ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Parser)]
struct MailArgs {
    #[command(subcommand)]
    cmd: MailCmd,
    /// Mail directory (default: `.conductr/mail`).
    #[arg(long, global = true)]
    mail_dir: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
enum MailCmd {
    /// Send a mail message to the shared inbox.
    Send {
        /// Kind of message to send.
        #[arg(long, value_name = "KIND")]
        kind: String,
        /// Issue number this message is about.
        #[arg(long)]
        issue: u64,
        /// Comma-separated list of files (for scope-claim).
        #[arg(long)]
        files: Option<String>,
        /// Short summary text.
        #[arg(long)]
        summary: Option<String>,
        /// Your agent identifier (defaults to the current username).
        #[arg(long)]
        agent: Option<String>,
    },
    /// List messages in the inbox.
    Inbox {
        /// Filter by message kind (e.g. `scope-claim`).
        #[arg(long)]
        kind: Option<String>,
        /// Only show messages sent within this duration (e.g. `1h`, `30m`, `7d`).
        #[arg(long)]
        since: Option<String>,
    },
    /// Check whether an issue has overlapping scope claims in the mailbox.
    Dedup {
        /// Issue number to check.
        #[arg(long)]
        issue: u64,
        /// Comma-separated list of candidate files.
        #[arg(long)]
        files: Option<String>,
    },
    /// Request synthesis of two or more PRs for the same issue.
    Synthesise {
        /// Issue number the PRs address.
        #[arg(long)]
        issue: u64,
        /// Comma-separated PR numbers to synthesise.
        #[arg(long)]
        prs: String,
        /// Your agent identifier (defaults to the current username).
        #[arg(long)]
        agent: Option<String>,
    },
}

async fn run_mail(args: MailArgs) -> Result<()> {
    let dir = args
        .mail_dir
        .unwrap_or_else(FsMailbox::default_path);
    let mailbox = FsMailbox::new(dir);

    match args.cmd {
        MailCmd::Send { kind, issue, files, summary, agent } => {
            let agent = agent.unwrap_or_else(whoami_or_default);
            let payload = build_mail_kind(&kind, issue, files.as_deref(), summary.as_deref())?;
            let id = mailbox.send(&agent, payload).await?;
            println!("sent: {id}");
        }
        MailCmd::Inbox { kind, since } => {
            let since_dur = since.as_deref().map(parse_duration).transpose()?;
            let messages = mailbox.inbox(kind.as_deref(), since_dur).await?;
            println!("{}", serde_json::to_string_pretty(&messages)?);
        }
        MailCmd::Dedup { issue, files } => {
            let files_vec: Vec<String> = files
                .as_deref()
                .map(|f| f.split(',').map(|s| s.trim().to_string()).collect())
                .unwrap_or_default();
            let report = check_scope(&mailbox, issue, &files_vec).await;
            println!("{}", serde_json::to_string_pretty(&match report {
                conductr_mail::dedup::ScopeReport::Clear => serde_json::json!({ "overlap": false }),
                conductr_mail::dedup::ScopeReport::Overlap { existing_message_id, conflicting_files } => {
                    serde_json::json!({
                        "overlap": true,
                        "existing_message_id": existing_message_id,
                        "conflicting_files": conflicting_files,
                    })
                }
            })?);
        }
        MailCmd::Synthesise { issue, prs, agent } => {
            let agent = agent.unwrap_or_else(whoami_or_default);
            let pr_numbers: Vec<u64> = prs
                .split(',')
                .map(|s| s.trim().parse::<u64>())
                .collect::<std::result::Result<_, _>>()
                .with_context(|| format!("invalid PR list: {prs}"))?;
            let id = request_synthesis(&mailbox, &agent, issue, pr_numbers).await?;
            println!("synthesis requested: {id}");
        }
    }
    Ok(())
}

fn whoami_or_default() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "unknown-agent".into())
}

fn build_mail_kind(
    kind: &str,
    issue: u64,
    files: Option<&str>,
    summary: Option<&str>,
) -> Result<MailKind> {
    match kind {
        "scope-claim" | "scope_claim" => {
            let files_vec: Vec<String> = files
                .unwrap_or("")
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            Ok(MailKind::ScopeClaim {
                issue,
                files: files_vec,
                summary: summary.unwrap_or("").to_string(),
            })
        }
        "note" => Ok(MailKind::Note { text: summary.unwrap_or("").to_string() }),
        other => anyhow::bail!("unknown mail kind: {other}. Valid: scope-claim, note"),
    }
}

fn parse_duration(s: &str) -> Result<std::time::Duration> {
    let s = s.trim();
    if let Some(n) = s.strip_suffix('d') {
        return Ok(std::time::Duration::from_secs(n.parse::<u64>()? * 86400));
    }
    if let Some(n) = s.strip_suffix('h') {
        return Ok(std::time::Duration::from_secs(n.parse::<u64>()? * 3600));
    }
    if let Some(n) = s.strip_suffix('m') {
        return Ok(std::time::Duration::from_secs(n.parse::<u64>()? * 60));
    }
    if let Some(n) = s.strip_suffix('s') {
        return Ok(std::time::Duration::from_secs(n.parse::<u64>()?));
    }
    anyhow::bail!("cannot parse duration: {s}. Use e.g. 1h, 30m, 7d, 90s")
}

// ── Setup ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Parser)]
struct SetupArgs {
    #[command(subcommand)]
    cmd: SetupCmd,
}

#[derive(Debug, Subcommand)]
enum SetupCmd {
    /// Report the maturity level of a repository.
    Status {
        /// Path to the repository root (defaults to current directory).
        #[arg(long)]
        repo: Option<PathBuf>,
    },
    /// Interactively bring a repository to a higher maturity level.
    Wizard {
        /// Path to the repository root (defaults to current directory).
        #[arg(long)]
        repo: Option<PathBuf>,
        /// Apply all fixes without prompting.
        #[arg(long)]
        non_interactive: bool,
        /// Print the plan without making any changes.
        #[arg(long)]
        dry_run: bool,
    },
    /// Open the Claude GitHub App install page.
    InstallClaudeApp {
        /// Path to the repository root (defaults to current directory).
        #[arg(long)]
        repo: Option<PathBuf>,
    },
    /// Symlink all skills/<name>/ into ~/.claude/skills/ so Claude Code can discover them.
    InstallSkills {
        /// Path to the repository root (defaults to current directory).
        #[arg(long)]
        repo: Option<PathBuf>,
        /// Print the plan without making any changes.
        #[arg(long)]
        dry_run: bool,
        /// Overwrite conflicting symlinks (never overwrites real directories).
        #[arg(long)]
        force: bool,
    },
    /// Provision all active projects in the machine-wide `~/.conductr` registry.
    Spawn(SpawnArgs),
}

#[derive(Debug, Parser)]
struct SpawnArgs {
    /// Provision only the project with this tag (omit for all active projects).
    tag: Option<String>,
    /// Print plan only; don't make any changes.
    #[arg(long)]
    dry_run: bool,
    /// Also attempt to provision pending entries (skipped by default).
    #[arg(long)]
    include_pending: bool,
    /// Path to the registry file (defaults to `~/.conductr`).
    #[arg(long)]
    registry: Option<PathBuf>,
    /// Override the command booted into freshly-created sessions. Defaults to
    /// Remote Control + `auto` permission mode so the session is immediately
    /// driveable. Ignored for sessions that already exist.
    #[arg(long)]
    command: Option<String>,
    /// Create sessions but leave them at a shell prompt instead of booting Claude.
    #[arg(long)]
    no_launch: bool,
}

async fn run_setup(args: SetupArgs) -> Result<()> {
    match args.cmd {
        SetupCmd::Status { repo } => {
            let repo = resolve_repo(repo)?;
            let results = conductr_setup::checks::run_all(&repo);
            let report = conductr_core::MaturityReport::compute(results);
            println!("Repo:  {}", repo.display());
            println!("Level: {}", report.level);
            println!();
            for r in &report.results {
                let icon = if r.passed { "✓" } else { "✗" };
                println!("  {} [{}] {}", icon, r.check.level, r.check.label);
                if let Some(detail) = &r.detail {
                    println!("    → {}", detail);
                }
            }
        }
        SetupCmd::Wizard { repo, non_interactive, dry_run } => {
            let repo = resolve_repo(repo)?;
            let mode = if dry_run {
                WizardMode::DryRun
            } else if non_interactive {
                WizardMode::NonInteractive
            } else {
                WizardMode::Interactive
            };
            conductr_setup::wizard::run(&repo, mode).await?;
        }
        SetupCmd::InstallClaudeApp { repo } => {
            let repo = resolve_repo(repo)?;
            conductr_setup::fixes::install_claude_app(&repo, false)?;
        }
        SetupCmd::InstallSkills { repo, dry_run, force } => {
            let repo = resolve_repo(repo)?;
            conductr_setup::fixes::install_skills(&repo, dry_run, force)?;
        }
        SetupCmd::Spawn(a) => run_setup_spawn(a).await?,
    }
    Ok(())
}

async fn run_setup_spawn(args: SpawnArgs) -> Result<()> {
    let reg = registry::load(args.registry.as_deref())?;

    let active_count = reg.active().count();
    let pending_count = reg.pending().count();
    println!(
        "registry: {} active, {} pending",
        active_count, pending_count
    );

    let launch_command = if args.no_launch {
        None
    } else {
        Some(
            args.command
                .unwrap_or_else(|| setup_spawn::DEFAULT_LAUNCH_COMMAND.to_string()),
        )
    };

    let opts = setup_spawn::SpawnOptions {
        dry_run: args.dry_run,
        tag_filter: args.tag,
        include_pending: args.include_pending,
        launch_command,
    };

    let reports = setup_spawn::run(&reg, &opts).await?;
    setup_spawn::print_report(&reports);

    if pending_count > 0 && !args.include_pending && opts.tag_filter.is_none() {
        println!("\n{pending_count} project(s) pending — edit ~/.conductr to promote");
    }
    Ok(())
}

// ── Local ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Parser)]
struct LocalArgs {
    #[command(subcommand)]
    cmd: LocalCmd,
}

#[derive(Debug, Subcommand)]
enum LocalCmd {
    /// Probe the host for local AI agent providers (ollama, llamacpp, models).
    Detect(DetectArgs),
    /// Install a local AI provider (or all missing ones) using the bundled scripts.
    Setup(LocalSetupArgs),
}

#[derive(Debug, Parser)]
struct DetectArgs {
    /// Emit machine-readable JSON instead of a table.
    #[arg(long)]
    json: bool,
}

async fn run_local(args: LocalArgs) -> Result<()> {
    match args.cmd {
        LocalCmd::Detect(a) => run_local_detect(a).await,
        LocalCmd::Setup(a) => run_local_setup(a),
    }
}

async fn run_local_detect(args: DetectArgs) -> Result<()> {
    let rows = local_detect::run_all_probes().await;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }
    println!("{:<12}  {:<11}  {}", "PROVIDER", "STATUS", "DETAIL");
    for row in &rows {
        println!("{:<12}  {:<11}  {}", row.provider, row.status.as_str(), row.detail);
    }
    Ok(())
}

fn resolve_repo(repo: Option<PathBuf>) -> Result<PathBuf> {
    match repo {
        Some(p) => Ok(p),
        None => std::env::current_dir().context("could not determine current directory"),
    }
}


#[derive(Debug, Parser)]
struct LocalSetupArgs {
    /// Provider to install: ollama, llamacpp, pi.
    /// Omit to auto-detect and install all missing providers.
    #[arg(long)]
    provider: Option<String>,
    /// Print the plan without executing any scripts.
    #[arg(long)]
    dry_run: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LocalProvider {
    Ollama,
    LlamaCpp,
    Pi,
}

impl LocalProvider {
    fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "ollama" => Some(LocalProvider::Ollama),
            "llamacpp" | "llama-cpp" | "llama.cpp" | "llama_cpp" => Some(LocalProvider::LlamaCpp),
            "pi" => Some(LocalProvider::Pi),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            LocalProvider::Ollama => "ollama",
            LocalProvider::LlamaCpp => "llamacpp",
            LocalProvider::Pi => "pi",
        }
    }

    fn binary(self) -> &'static str {
        match self {
            LocalProvider::Ollama => "ollama",
            LocalProvider::LlamaCpp => "llama-server",
            LocalProvider::Pi => "pi",
        }
    }

    fn script_name(self, os: HostOs) -> &'static str {
        match (self, os) {
            (LocalProvider::Ollama, HostOs::Mac) => "install-ollama-mac.sh",
            (LocalProvider::Ollama, HostOs::Linux) => "install-ollama-linux.sh",
            (LocalProvider::LlamaCpp, HostOs::Mac) => "install-llamacpp-mac.sh",
            (LocalProvider::LlamaCpp, HostOs::Linux) => "install-llamacpp-linux.sh",
            (LocalProvider::Pi, HostOs::Mac) => "install-pi-mac.sh",
            (LocalProvider::Pi, HostOs::Linux) => "install-pi-linux.sh",
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum HostOs {
    Mac,
    Linux,
}

fn detect_host_os() -> Result<HostOs> {
    let out = std::process::Command::new("uname")
        .arg("-s")
        .output()
        .context("running uname -s")?;
    let name = String::from_utf8_lossy(&out.stdout).trim().to_lowercase();
    match name.as_str() {
        "darwin" => Ok(HostOs::Mac),
        "linux" => Ok(HostOs::Linux),
        other => anyhow::bail!(
            "unsupported OS: {other}. conductr local setup supports macOS and Linux only"
        ),
    }
}

fn provider_is_installed(provider: LocalProvider) -> bool {
    let binary = provider.binary();
    std::env::var_os("PATH")
        .map(|path| {
            std::env::split_paths(&path).any(|dir| {
                let candidate = dir.join(binary);
                candidate.is_file()
            })
        })
        .unwrap_or(false)
}

fn find_scripts_local_dir() -> Result<std::path::PathBuf> {
    if let Ok(dir) = std::env::var("CONDUCTR_SCRIPTS_DIR") {
        let p = std::path::PathBuf::from(dir);
        if p.is_dir() {
            return Ok(p);
        }
    }
    let mut candidate = std::env::current_dir().context("determining cwd")?;
    for _ in 0..4 {
        let scripts_local = candidate.join("scripts").join("local");
        if scripts_local.is_dir() {
            return Ok(scripts_local);
        }
        if !candidate.pop() {
            break;
        }
    }
    anyhow::bail!(
        "scripts/local/ directory not found. \
         Set CONDUCTR_SCRIPTS_DIR to the directory containing the install scripts, \
         or run from within the conductr repo."
    )
}


fn run_cadence(args: CadenceArgs) -> Result<()> {
    match args.cmd {
        CadenceCmd::Sync(a) => {
            let repo = a.repo.unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
            let report = cadence::sync(&repo, a.dry_run, cadence::Mechanism::from(a.mechanism))?;
            println!("{report}");
            Ok(())
        }
        CadenceCmd::Status(a) => {
            let repo = a.repo.unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
            let report = cadence::status(&repo)?;
            println!("{report}");
            Ok(())
        }
        CadenceCmd::Remove(a) => {
            let repo = a.repo.unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
            let report = cadence::remove(&repo, a.dry_run)?;
            println!("{report}");
            Ok(())
        }
        CadenceCmd::Show(a) => {
            let at = a.at.as_deref().map(parse_utc_datetime).transpose()?;
            if a.all {
                let report = cadence::show_all(at)?;
                print!("{report}");
            } else {
                let repo = a.repo.unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
                let report = cadence::show(&repo, at)?;
                print!("{report}");
            }
            Ok(())
        }
    }
}

fn parse_utc_datetime(s: &str) -> Result<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .with_context(|| format!("invalid --at value {s:?}: expected ISO-8601, e.g. 2026-05-10T14:23:00Z"))
}

fn run_local_setup(args: LocalSetupArgs) -> Result<()> {
    let os = detect_host_os()?;
    let scripts_dir = find_scripts_local_dir()?;

    let to_install: Vec<LocalProvider> = if let Some(name) = &args.provider {
        let p = LocalProvider::from_str(name).with_context(|| {
            format!("unknown provider '{name}'. Valid: ollama, llamacpp, pi")
        })?;
        vec![p]
    } else {
        let all = [LocalProvider::Ollama, LocalProvider::LlamaCpp, LocalProvider::Pi];
        let missing: Vec<LocalProvider> =
            all.iter().copied().filter(|p| !provider_is_installed(*p)).collect();
        if missing.is_empty() {
            println!("All providers already installed.");
            return Ok(());
        }
        println!(
            "Missing providers: {}",
            missing.iter().map(|p| p.name()).collect::<Vec<_>>().join(", ")
        );
        missing
    };

    for provider in to_install {
        let script_name = provider.script_name(os);
        let script_path = scripts_dir.join(script_name);

        if !script_path.exists() {
            anyhow::bail!(
                "script not found: {}. Is CONDUCTR_SCRIPTS_DIR set correctly?",
                script_path.display()
            );
        }

        if args.dry_run {
            println!("plan: would run {}", script_path.display());
        } else {
            println!("running: {}", script_path.display());
            let status = std::process::Command::new("bash")
                .arg(&script_path)
                .status()
                .with_context(|| format!("running {}", script_path.display()))?;
            if !status.success() {
                anyhow::bail!("{} exited with non-zero status: {}", script_name, status);
            }
        }
    }

    Ok(())
}

// ── Architect ─────────────────────────────────────────────────────────────────

#[derive(Debug, Parser)]
struct ArchitectArgs {
    #[command(subcommand)]
    cmd: ArchitectCmd,
}

#[derive(Debug, Subcommand)]
enum ArchitectCmd {
    /// Workspace-wide architectural audit, or targeted review of a PR or issue.
    ///
    /// Opens or reuses the `conductr-architect` tmux session, starts Claude if
    /// needed, then sends `/architect review [<target>]`.
    Review(ArchitectReviewArgs),
    /// Generate a mermaid architecture diagram (Claude-required).
    ///
    /// Finds or spawns a QA pane (`conductr-<tag>-qa<n>`), starts Claude if needed,
    /// then sends `/architect diagram [--repo-path <path>] [--output <file>] [--tier dependency|ports]`.
    Diagram(ArchitectDiagramArgs),
    /// Plan a batch of issues: infer phrases, estimate complexity, write ARNs.
    ///
    /// Opens or reuses the `conductr-architect` tmux session, starts Claude if
    /// needed, then sends `/architect plan <issues>`.
    Plan(ArchitectPlanArgs),
    /// LLM-driven security audit (Claude-required).
    ///
    /// Opens or reuses the `conductr-architect` tmux session, starts Claude if
    /// needed, then sends `/architect security-review`.
    SecurityReview(ArchitectSecurityReviewArgs),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum DiagramTier {
    /// Service dependency + DI chains (v1).
    Dependency,
    /// Port traits + adapter implementations per service (v2).
    Ports,
}

#[derive(Debug, Parser)]
struct ArchitectDiagramArgs {
    /// Path to the repository to analyse (defaults to cwd).
    #[arg(long, default_value = ".")]
    repo_path: PathBuf,
    /// Output file for the generated diagram, relative to --repo-path.
    /// Defaults to `docs/architecture/dependency-diagram.md` for `--tier dependency`
    /// and `docs/architecture/ports-diagram.md` for `--tier ports`.
    #[arg(long)]
    output: Option<PathBuf>,
    /// Diagram tier to generate: `dependency` (v1) or `ports` (v2).
    #[arg(long, value_enum, default_value = "dependency")]
    tier: DiagramTier,
    /// Working directory for the QA tmux session (defaults to current directory).
    #[arg(long)]
    cwd: Option<PathBuf>,
    /// Print the plan without creating sessions, starting Claude, or sending keys.
    #[arg(long)]
    dry_run: bool,
}

#[derive(Debug, Parser)]
struct ArchitectReviewArgs {
    /// PR number (`123`) or issue number (`#123`) to review.
    /// Omit for a workspace-wide architectural audit.
    target: Option<String>,
    /// Working directory for the tmux session (defaults to current directory).
    #[arg(long)]
    cwd: Option<PathBuf>,
    /// Print the plan without creating sessions, starting Claude, or sending keys.
    #[arg(long)]
    dry_run: bool,
}

#[derive(Debug, Parser)]
struct ArchitectPlanArgs {
    /// Space-separated issue numbers (e.g. `42 43 44` or `#42 #43 #44`).
    /// At least one issue is required.
    #[arg(required = true)]
    issues: Vec<String>,
    /// Working directory for the tmux session (defaults to current directory).
    #[arg(long)]
    cwd: Option<PathBuf>,
    /// Print the plan without creating sessions, starting Claude, or sending keys.
    #[arg(long)]
    dry_run: bool,
}

#[derive(Debug, Parser)]
struct ArchitectSecurityReviewArgs {
    /// Working directory for the tmux session (defaults to current directory).
    #[arg(long)]
    cwd: Option<PathBuf>,
    /// Print the plan without creating sessions, starting Claude, or sending keys.
    #[arg(long)]
    dry_run: bool,
}

// ── RunTask ───────────────────────────────────────────────────────────────────

#[derive(Debug, Parser)]
struct RunTaskArgs {
    /// Local agent provider to use.
    ///
    /// Provider precedence (highest to lowest):
    ///   1. This flag
    ///   2. CONDUCTR_LOCAL_PROVIDER env var
    ///   3. `[local].provider` in `.conductr`
    ///   4. Auto-pick the first `present` provider from `conductr local detect`
    #[arg(long, value_enum)]
    provider: Option<wiring::LocalAgentKind>,

    /// Prompt text to send to the agent (mutually exclusive with --prompt-file).
    #[arg(long, conflicts_with = "prompt_file")]
    prompt: Option<String>,

    /// Path to a file containing the prompt (mutually exclusive with --prompt).
    #[arg(long, conflicts_with = "prompt")]
    prompt_file: Option<PathBuf>,

    /// Model name (ollama only). Overrides CONDUCTR_LOCAL_MODEL and `[local].model` in `.conductr`.
    #[arg(long)]
    model: Option<String>,
}

async fn run_run_task(args: RunTaskArgs) -> Result<()> {
    let prompt = read_prompt_text(&args)?;
    let (kind, config_model) = resolve_provider(args.provider).await?;
    let model = args.model
        .or_else(|| std::env::var("CONDUCTR_LOCAL_MODEL").ok())
        .or(config_model);
    let agent = wiring::local_agent(kind, model);
    eprintln!("provider: {}", kind.as_str());
    let response = agent
        .complete(&prompt)
        .await
        .with_context(|| format!("calling {} agent", kind.as_str()))?;
    println!("{response}");
    Ok(())
}

fn read_prompt_text(args: &RunTaskArgs) -> Result<String> {
    if let Some(text) = &args.prompt {
        return Ok(text.clone());
    }
    if let Some(path) = &args.prompt_file {
        return std::fs::read_to_string(path)
            .with_context(|| format!("reading prompt file {}", path.display()));
    }
    anyhow::bail!("one of --prompt or --prompt-file is required")
}

/// Resolve the local-agent provider using the four-level precedence chain.
async fn resolve_provider(
    explicit: Option<wiring::LocalAgentKind>,
) -> Result<(wiring::LocalAgentKind, Option<String>)> {
    // 1. CLI flag
    if let Some(kind) = explicit {
        return Ok((kind, None));
    }
    // 2. Env var
    if let Ok(val) = std::env::var("CONDUCTR_LOCAL_PROVIDER") {
        let kind = wiring::LocalAgentKind::parse(&val).with_context(|| {
            format!(
                "CONDUCTR_LOCAL_PROVIDER='{val}' is not valid. \
                 Use: ollama, llamacpp, or pi"
            )
        })?;
        return Ok((kind, None));
    }
    // 3. .conductr [local] section
    let cwd = std::env::current_dir().context("getting current directory")?;
    let local_cfg = config::read_local_section(&cwd)?;
    if let Some(provider_str) = local_cfg.provider {
        let kind = wiring::LocalAgentKind::parse(&provider_str).with_context(|| {
            format!(
                "`[local].provider = '{provider_str}'` in .conductr is not valid. \
                 Use: ollama, llamacpp, or pi"
            )
        })?;
        return Ok((kind, local_cfg.model));
    }
    // 4. Auto-detect: first present provider from `conductr local detect`
    eprintln!("no provider specified; running auto-detect…");
    let probes = local_detect::run_all_probes().await;
    let kind = pick_provider_from_probes(&probes).ok_or_else(|| {
        anyhow::anyhow!(
            "no local provider detected. \
             Run `conductr local detect` for details, \
             or specify --provider / CONDUCTR_LOCAL_PROVIDER."
        )
    })?;
    Ok((kind, None))
}

/// Return the first probe row that is `Present` and maps to a known `LocalAgentKind`.
/// Skips rows whose provider name is not a valid kind (e.g. "qwen3:27b").
fn pick_provider_from_probes(probes: &[local_detect::ProbeRow]) -> Option<wiring::LocalAgentKind> {
    probes
        .iter()
        .filter(|r| r.status == local_detect::ProbeStatus::Present)
        .filter_map(|r| wiring::LocalAgentKind::parse(r.provider))
        .next()
}

// ─────────────────────────────────────────────────────────────────────────────

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

async fn run_orchestrate(args: OrchestrateArgs) -> Result<()> {
    let (owner, repo) = args
        .repo
        .split_once('/')
        .with_context(|| format!("invalid --repo `{}` (expected owner/repo)", args.repo))?;

    let repo_root = args
        .repo_root
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    // Read [ci] section and optionally build a LocalCi adapter.
    let ci_section = config::read_ci_section(&repo_root)?;
    let ci_mode: CiMode = args
        .ci
        .map(CiMode::from)
        .unwrap_or(ci_section.mode);

    let allow_fork_prs = ci_section.allow_fork_prs;
    let local_ci: Option<std::sync::Arc<dyn LocalCi>> = if ci_section.commands.is_empty() {
        None
    } else {
        Some(std::sync::Arc::new(LocalCiAdapter::new(
            repo_root.clone(),
            LocalCiConfig {
                commands: ci_section.commands,
                timeout_secs: ci_section.timeout_secs,
                mode: ci_mode,
            },
        )))
    };

    // Read [orchestrate] section for cap and tmux config.
    let orch_section = config::read_orchestrate_section(&repo_root)?;

    let mut cfg = OrchestratorConfig::new(RepoSlug::new(owner, repo));
    cfg.dry_run = args.dry_run;
    cfg.poll_interval = std::time::Duration::from_secs(args.poll_secs);
    cfg.default_human_assignee = args.human_assignee;
    cfg.ci_mode = ci_mode;
    cfg.allow_fork_local_ci = allow_fork_prs;
    cfg.max_parallel_beats = orch_section.max_parallel_beats as usize;
    cfg.max_parallel_qa = orch_section.max_parallel_qa.unwrap_or(2) as usize;
    cfg.tmux_cwd = Some(repo_root.to_string_lossy().into_owned());

    let tmux = std::sync::Arc::new(Tmux::new());
    let mut orch = Orchestrator::new(GhCli, cfg);
    orch = orch.with_tmux(tmux);
    if let Some(lci) = local_ci {
        orch = orch.with_local_ci(lci);
    }

    if args.once {
        let report = orch.run_cycle().await?;
        println!("{}", serde_json::to_string_pretty(&report_to_json(&report))?);
    } else {
        let history = orch.run_to_completion().await?;
        let cycles: Vec<_> = history.iter().map(report_to_json).collect();
        println!("{}", serde_json::to_string_pretty(&cycles)?);
    }
    Ok(())
}

fn report_to_json(r: &conductr_orchestrate::orchestrator::CycleReport) -> serde_json::Value {
    serde_json::json!({
        "merged": r.merged,
        "triggered": r.triggered,
        "waiting": r.waiting,
        "blocked": r.blocked,
        "human": r.human,
        "pr_failing": r.pr_failing,
        "soft_chord_deferred": r.soft_chord_deferred,
        "progress_made": r.progress_made,
        "local_ci": r.local_ci,
    })
}

async fn run_instance(args: InstanceArgs) -> Result<()> {
    match args.cmd {
        InstanceCmd::SpinUp { name } => {
            anyhow::bail!(
                "instance spin-up not implemented yet. Requested: {name}"
            );
        }
        InstanceCmd::List => {
            let _provider = conductr_adapters::mock::MockInstanceProvider;
            println!("[]");
            Ok(())
        }
    }
}

fn run_schedule(args: ScheduleArgs) -> Result<()> {
    match args.cmd {
        ScheduleCmd::Validate { path } => {
            let src = std::fs::read_to_string(&path)
                .with_context(|| format!("reading {}", path.display()))?;
            let p = parse(&src)?;
            println!("ok: {} bar(s), total {:?}", p.bars.len(), p.total_duration());
            Ok(())
        }
        ScheduleCmd::Render { path } => {
            let src = std::fs::read_to_string(&path)
                .with_context(|| format!("reading {}", path.display()))?;
            let p = parse(&src)?;
            print!("{}", render_ascii(&p));
            Ok(())
        }
        ScheduleCmd::FromPlan { path, render } => {
            let src = std::fs::read_to_string(&path)
                .with_context(|| format!("reading {}", path.display()))?;
            let plan = parse_plan(&src)?;
            let pattern = plan_to_pattern(&plan);
            let dsl = pattern_to_dsl(&pattern);
            print!("{}", dsl);
            if render {
                println!();
                print!("{}", render_ascii(&pattern));
            }
            Ok(())
        }
    }
}


async fn run_tasks(args: TasksArgs) -> Result<()> {
    let beads = Beads::new();
    match args.cmd {
        TasksCmd::List { ready } => {
            let tasks = list_tasks(&beads, ready).await?;
            println!("{}", serde_json::to_string_pretty(&tasks)?);
            Ok(())
        }
        TasksCmd::Create { title, priority } => {
            let t = create_task(&beads, &title, priority, None, &[]).await?;
            println!("{}", serde_json::to_string_pretty(&t)?);
            Ok(())
        }
        TasksCmd::SyncToNotion { database } => {
            let notion = Notion::from_env()?.with_database(&database);
            let report = sync_tasks(&beads, &notion, Some(&database)).await?;
            println!(
                "pushed {} tasks ({} failed)",
                report.pushed.len(),
                report.failed.len()
            );
            if !report.failed.is_empty() {
                for (id, err) in &report.failed {
                    eprintln!("  failed: {id} — {err}");
                }
            }
            Ok(())
        }
    }
}

// ── Architect ─────────────────────────────────────────────────────────────────

async fn run_architect(args: ArchitectArgs) -> Result<()> {
    match args.cmd {
        ArchitectCmd::Review(a) => run_architect_review(a).await,
        ArchitectCmd::Diagram(a) => run_architect_diagram(a).await,
        ArchitectCmd::Plan(a) => run_architect_plan(a).await,
        ArchitectCmd::SecurityReview(a) => run_architect_security_review(a).await,
    }
}

async fn run_architect_review(args: ArchitectReviewArgs) -> Result<()> {
    let session = "conductr-architect";
    let cwd = args
        .cwd
        .as_deref()
        .map(|p| p.to_string_lossy().into_owned())
        .or_else(|| std::env::current_dir().ok().map(|p| p.to_string_lossy().into_owned()))
        .unwrap_or_else(|| ".".to_string());

    // Workspace-wide mode: run deterministic predicates before handing off to
    // Claude, so findings are available regardless of whether the tmux session
    // starts successfully.
    if args.target.is_none() {
        let repo_path = args.cwd.as_deref().map(PathBuf::from)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));

        // Check .claude/base.md — emits one finding (with hex default) if absent
        // or unstructured, so every repo gets useful output on first idle pass.
        let base_findings = idle::check_base_md(&repo_path);
        if base_findings.is_empty() {
            println!("architect: .claude/base.md OK");
        } else {
            for f in &base_findings {
                println!(
                    "architect: [{}] {}",
                    idle::severity_label(&f.severity),
                    f.title
                );
            }
        }

        // CLI–skill parity: gate on skills/ directory presence; repos without a
        // skills surface produce no parity findings (not an error).
        match parity::Workspace::open(&repo_path) {
            Ok(ws) => {
                let findings = parity::check_cli_skill_parity(&ws);
                if findings.is_empty() {
                    println!("architect: CLI–skill parity OK");
                } else {
                    for f in &findings {
                        println!(
                            "architect: [{}] {}",
                            idle::severity_label(&f.severity),
                            f.title
                        );
                    }
                    println!("architect: {} parity finding(s)", findings.len());
                }
            }
            Err(_) => println!("architect: no skills/ surface — parity check skipped"),
        }
    }

    let skill_cmd = build_architect_review_prompt(args.target.as_deref());
    let claude_cmd = "claude --dangerously-skip-permissions";

    if args.dry_run {
        let tmux = Tmux::new();
        return run_architect_review_dry(&tmux, session, &cwd, claude_cmd, &skill_cmd).await;
    }

    let tmux = Tmux::new();
    let state = ensure_session(&tmux, session, &cwd)
        .await
        .with_context(|| format!("ensuring tmux session '{session}'"))?;

    match &state {
        SessionState::Existing(Health::Working { activity }) => {
            println!("architect: session '{session}' is busy ({activity}); skipping");
            return Ok(());
        }
        SessionState::Existing(Health::Unknown { reason }) => {
            println!("architect: session '{session}' is unclassified ({reason}); skipping");
            return Ok(());
        }
        SessionState::Existing(Health::Crashed { .. }) => {
            println!("architect: session '{session}' crashed — restarting Claude");
            tmux.send_line(session, claude_cmd)
                .await
                .context("restarting Claude after crash")?;
            wait_for_idle(&tmux, session).await?;
        }
        SessionState::Created => {
            println!("architect: created session '{session}' — starting Claude");
            tmux.send_line(session, claude_cmd)
                .await
                .context("starting Claude in new session")?;
            wait_for_idle(&tmux, session).await?;
        }
        SessionState::Existing(Health::Idle { .. }) => {
            println!("architect: session '{session}' is idle — sending review prompt");
        }
    }

    println!("architect: sending: {skill_cmd}");
    tmux.send_line(session, &skill_cmd)
        .await
        .context("sending architect review prompt")?;

    Ok(())
}

fn build_architect_review_prompt(target: Option<&str>) -> String {
    match target {
        Some(t) => format!("/architect review {t}"),
        None => "/architect review".to_string(),
    }
}

async fn run_architect_review_dry(
    tmux: &Tmux,
    session: &str,
    cwd: &str,
    claude_cmd: &str,
    skill_cmd: &str,
) -> Result<()> {
    let sessions = match tmux.list_sessions().await {
        Ok(s) => s,
        Err(TmuxError::NoServer) | Err(TmuxError::NotInstalled) => {
            println!("plan: tmux not running");
            println!("plan: → would create session '{session}' at cwd={cwd}");
            println!("plan: → would start Claude: `{claude_cmd}`");
            println!("plan: → would send: `{skill_cmd}`");
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };

    if sessions.iter().any(|s| s.name == session) {
        match diagnose_one(tmux, session).await {
            Ok(d) => {
                println!("plan: session '{session}' exists ({})", health_label(&d.health));
                match &d.health {
                    Health::Working { activity } => {
                        println!("plan: → would skip (busy: {activity})");
                    }
                    Health::Unknown { reason } => {
                        println!("plan: → would skip (unclassified: {reason})");
                    }
                    Health::Crashed { .. } => {
                        println!("plan: → would restart Claude: `{claude_cmd}`");
                        println!("plan: → would send: `{skill_cmd}`");
                    }
                    Health::Idle { .. } => {
                        println!("plan: → would send: `{skill_cmd}`");
                    }
                }
            }
            Err(e) => println!("plan: session '{session}' exists but diagnose failed: {e}"),
        }
    } else {
        println!("plan: session '{session}' does not exist");
        println!("plan: cwd = {cwd}");
        println!("plan: → would create session '{session}'");
        println!("plan: → would start Claude: `{claude_cmd}`");
        println!("plan: → would send: `{skill_cmd}`");
    }
    Ok(())
}

// ── Architect security-review ─────────────────────────────────────────────────

async fn run_architect_security_review(args: ArchitectSecurityReviewArgs) -> Result<()> {
    let session = "conductr-architect";
    let cwd = args
        .cwd
        .as_deref()
        .map(|p| p.to_string_lossy().into_owned())
        .or_else(|| std::env::current_dir().ok().map(|p| p.to_string_lossy().into_owned()))
        .unwrap_or_else(|| ".".to_string());

    let skill_cmd = "/architect security-review";
    let claude_cmd = "claude --dangerously-skip-permissions";

    if args.dry_run {
        let tmux = Tmux::new();
        return run_architect_review_dry(&tmux, session, &cwd, claude_cmd, skill_cmd).await;
    }

    let tmux = Tmux::new();
    let state = ensure_session(&tmux, session, &cwd)
        .await
        .with_context(|| format!("ensuring tmux session '{session}'"))?;

    match &state {
        SessionState::Existing(Health::Working { activity }) => {
            println!("architect: session '{session}' is busy ({activity}); skipping");
            return Ok(());
        }
        SessionState::Existing(Health::Unknown { reason }) => {
            println!("architect: session '{session}' is unclassified ({reason}); skipping");
            return Ok(());
        }
        SessionState::Existing(Health::Crashed { .. }) => {
            println!("architect: session '{session}' crashed — restarting Claude");
            tmux.send_line(session, claude_cmd)
                .await
                .context("restarting Claude after crash")?;
            wait_for_idle(&tmux, session).await?;
        }
        SessionState::Created => {
            println!("architect: created session '{session}' — starting Claude");
            tmux.send_line(session, claude_cmd)
                .await
                .context("starting Claude in new session")?;
            wait_for_idle(&tmux, session).await?;
        }
        SessionState::Existing(Health::Idle { .. }) => {
            println!("architect: session '{session}' is idle — sending security-review prompt");
        }
    }

    println!("architect: sending: {skill_cmd}");
    tmux.send_line(session, skill_cmd)
        .await
        .context("sending architect security-review prompt")?;

    Ok(())
}

// ── Architect diagram ─────────────────────────────────────────────────────────

async fn run_architect_diagram(args: ArchitectDiagramArgs) -> Result<()> {
    let cwd = args
        .cwd
        .map(|p| p.to_string_lossy().into_owned())
        .or_else(|| std::env::current_dir().ok().map(|p| p.to_string_lossy().into_owned()))
        .unwrap_or_else(|| ".".to_string());

    let effective_output = args.output.clone().unwrap_or_else(|| match args.tier {
        DiagramTier::Dependency => PathBuf::from("docs/architecture/dependency-diagram.md"),
        DiagramTier::Ports => PathBuf::from("docs/architecture/ports-diagram.md"),
    });

    let tag = derive_qa_tag(&args.repo_path);
    let max_qa = read_max_qa(&args.repo_path);
    let skill_cmd = build_diagram_skill_cmd(&args.repo_path, &effective_output, args.tier);
    let claude_cmd = "claude --dangerously-skip-permissions";

    if args.dry_run {
        let tmux = Tmux::new();
        return run_architect_diagram_dry(&tmux, &tag, max_qa, &cwd, claude_cmd, &skill_cmd).await;
    }

    let tmux = Tmux::new();
    let session = find_or_spawn_qa_session(&tmux, &tag, max_qa, &cwd, claude_cmd).await?;

    println!("architect diagram: sending to '{session}': {skill_cmd}");
    tmux.send_line(&session, &skill_cmd)
        .await
        .context("sending diagram skill command to QA pane")?;

    Ok(())
}

// ── architect plan ────────────────────────────────────────────────────────────

async fn run_architect_plan(args: ArchitectPlanArgs) -> Result<()> {
    let session = "conductr-architect";
    let cwd = args
        .cwd
        .map(|p| p.to_string_lossy().into_owned())
        .or_else(|| std::env::current_dir().ok().map(|p| p.to_string_lossy().into_owned()))
        .unwrap_or_else(|| ".".to_string());

    let skill_cmd = build_architect_plan_prompt(&args.issues);
    let claude_cmd = "claude --dangerously-skip-permissions";

    if args.dry_run {
        let tmux = Tmux::new();
        return run_architect_plan_dry(&tmux, session, &cwd, claude_cmd, &skill_cmd).await;
    }

    let tmux = Tmux::new();
    let state = ensure_session(&tmux, session, &cwd)
        .await
        .with_context(|| format!("ensuring tmux session '{session}'"))?;

    match &state {
        SessionState::Existing(Health::Working { activity }) => {
            println!("architect: session '{session}' is busy ({activity}); skipping");
            return Ok(());
        }
        SessionState::Existing(Health::Unknown { reason }) => {
            println!("architect: session '{session}' is unclassified ({reason}); skipping");
            return Ok(());
        }
        SessionState::Existing(Health::Crashed { .. }) => {
            println!("architect: session '{session}' crashed — restarting Claude");
            tmux.send_line(session, claude_cmd)
                .await
                .context("restarting Claude after crash")?;
            wait_for_idle(&tmux, session).await?;
        }
        SessionState::Created => {
            println!("architect: created session '{session}' — starting Claude");
            tmux.send_line(session, claude_cmd)
                .await
                .context("starting Claude in new session")?;
            wait_for_idle(&tmux, session).await?;
        }
        SessionState::Existing(Health::Idle { .. }) => {
            println!("architect: session '{session}' is idle — sending plan prompt");
        }
    }

    println!("architect: sending: {skill_cmd}");
    tmux.send_line(session, &skill_cmd)
        .await
        .context("sending architect plan prompt")?;

    Ok(())
}

/// Derive the project tag for QA session naming.
/// Reads `project_tag` from `.conductr`; falls back to the directory name.
fn derive_qa_tag(repo_path: &std::path::Path) -> String {
    if let Ok(Some(tag)) = config::read_project_tag(repo_path) {
        return tag;
    }
    repo_path
        .canonicalize()
        .unwrap_or_else(|_| repo_path.to_path_buf())
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| {
            s.to_lowercase()
                .chars()
                .map(|c| if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' { c } else { '-' })
                .collect::<String>()
                .trim_matches('-')
                .to_string()
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "arch".to_string())
}

/// Read the QA slot cap from `[orchestrate] max_parallel_qa` in `.conductr`.
/// Defaults to 2 per the band schema.
fn read_max_qa(repo_path: &std::path::Path) -> u32 {
    config::read_orchestrate_section(repo_path)
        .ok()
        .and_then(|s| s.max_parallel_qa)
        .unwrap_or(2)
}

fn build_diagram_skill_cmd(
    repo_path: &std::path::Path,
    output: &std::path::Path,
    tier: DiagramTier,
) -> String {
    let tier_str = match tier {
        DiagramTier::Dependency => "dependency",
        DiagramTier::Ports => "ports",
    };
    format!(
        "/architect diagram --repo-path {} --output {} --tier {}",
        repo_path.display(),
        output.display(),
        tier_str,
    )
}

/// Find an idle QA session matching `conductr-<tag>-qa*`, or spawn a new one
/// within `max_qa` slots. Returns the session name to use.
async fn find_or_spawn_qa_session(
    tmux: &Tmux,
    tag: &str,
    max_qa: u32,
    cwd: &str,
    claude_cmd: &str,
) -> Result<String> {
    let qa_prefix = format!("conductr-{tag}-qa");

    // Probe existing sessions matching the prefix.
    let existing = diagnose_all(tmux, Some(&qa_prefix)).await.unwrap_or_default();

    // Reuse the first idle slot.
    for d in &existing {
        if matches!(d.health, Health::Idle { .. }) {
            println!("architect diagram: reusing idle QA session '{}'", d.session.name);
            return Ok(d.session.name.clone());
        }
    }

    // No idle slot — try to spawn a new one within cap.
    for n in 1..=max_qa {
        let name = format!("{qa_prefix}{n}");
        let slot_busy = existing.iter().any(|d| {
            d.session.name == name && !matches!(d.health, Health::Idle { .. })
        });
        if slot_busy {
            continue;
        }
        let state = ensure_session(tmux, &name, cwd)
            .await
            .with_context(|| format!("ensuring QA session '{name}'"))?;
        match &state {
            SessionState::Existing(Health::Crashed { .. }) => {
                println!("architect diagram: QA session '{name}' crashed — restarting Claude");
                tmux.send_line(&name, claude_cmd)
                    .await
                    .context("restarting Claude after crash")?;
                wait_for_idle(tmux, &name).await?;
            }
            SessionState::Created => {
                println!("architect diagram: created QA session '{name}' — starting Claude");
                tmux.send_line(&name, claude_cmd)
                    .await
                    .context("starting Claude in new QA session")?;
                wait_for_idle(tmux, &name).await?;
            }
            _ => {}
        }
        return Ok(name);
    }

    anyhow::bail!(
        "all {max_qa} QA slot(s) for tag '{tag}' are busy; \
         try again when a slot frees up, or raise [orchestrate] max_parallel_qa in .conductr"
    )
}

async fn run_architect_diagram_dry(
    tmux: &Tmux,
    tag: &str,
    max_qa: u32,
    cwd: &str,
    claude_cmd: &str,
    skill_cmd: &str,
) -> Result<()> {
    let qa_prefix = format!("conductr-{tag}-qa");

    let sessions = match tmux.list_sessions().await {
        Ok(s) => s,
        Err(TmuxError::NoServer) | Err(TmuxError::NotInstalled) => {
            let target = format!("{qa_prefix}1");
            println!("plan: tmux not running");
            println!("plan: → would create QA session '{target}' at cwd={cwd}");
            println!("plan: → would start Claude: `{claude_cmd}`");
            println!("plan: → would send: `{skill_cmd}`");
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };

    let qa_sessions: Vec<_> = sessions.iter().filter(|s| s.name.starts_with(&qa_prefix)).collect();

    if qa_sessions.is_empty() {
        let target = format!("{qa_prefix}1");
        println!("plan: no QA sessions matching '{qa_prefix}*'");
        println!("plan: → would create '{target}' at cwd={cwd}");
        println!("plan: → would start Claude: `{claude_cmd}`");
        println!("plan: → would send: `{skill_cmd}`");
        return Ok(());
    }

    // Look for an idle slot among existing sessions.
    for s in &qa_sessions {
        match diagnose_one(tmux, &s.name).await {
            Ok(d) if matches!(d.health, Health::Idle { .. }) => {
                println!("plan: QA session '{}' is idle → would reuse it", s.name);
                println!("plan: → would send: `{skill_cmd}`");
                return Ok(());
            }
            Ok(d) => println!("plan: QA session '{}' is {} — skipping", s.name, health_label(&d.health)),
            Err(e) => println!("plan: QA session '{}' probe failed: {e}", s.name),
        }
    }

    let count = qa_sessions.len();
    if (count as u32) < max_qa {
        let n = count + 1;
        let target = format!("{qa_prefix}{n}");
        println!("plan: all existing QA slots busy → would spawn '{target}'");
        println!("plan: → would start Claude: `{claude_cmd}`");
        println!("plan: → would send: `{skill_cmd}`");
    } else {
        println!("plan: all {max_qa} QA slot(s) for tag '{tag}' busy — would bail");
    }

    Ok(())
}

// ── Sync ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Parser)]
struct SyncArgs {
    #[command(subcommand)]
    cmd: Option<SyncCmd>,
    /// Print plan without writing to Google Calendar.
    #[arg(long)]
    dry_run: bool,
    /// `owner/repo` slug (defaults to `repo` in `.conductr`).
    #[arg(long)]
    repo: Option<String>,
    /// Google Calendar ID (defaults to $GCAL_CALENDAR_ID or `primary`).
    #[arg(long)]
    calendar_id: Option<String>,
    /// Path to the repo root for reading `.conductr` (defaults to cwd).
    #[arg(long)]
    repo_root: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
enum SyncCmd {
    /// Manually place a single test slot in the next eligible window.
    ScheduleTest(ScheduleTestArgs),
}

#[derive(Debug, Parser)]
struct ScheduleTestArgs {
    /// Tag for the test slot (use `*` for no specific tag).
    #[arg(long, default_value = "*")]
    tag: String,
    /// Subject / description for the test slot.
    #[arg(long)]
    subject: String,
    /// Print plan without writing to Google Calendar.
    #[arg(long)]
    dry_run: bool,
}

async fn run_sync(args: SyncArgs) -> Result<()> {
    let token = std::env::var("GCAL_OAUTH_TOKEN")
        .with_context(|| "GCAL_OAUTH_TOKEN is not set — obtain a Google OAuth access token")?;
    let repo_root = args
        .repo_root
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let cal_cfg = config::read_calendar_section(&repo_root).unwrap_or_default();
    let calendar_id = args
        .calendar_id
        .or_else(|| std::env::var("GCAL_CALENDAR_ID").ok())
        .or(cal_cfg.calendar_id)
        .unwrap_or_else(|| "primary".to_string());
    let calendar = GcalAdapter::new(token, calendar_id);

    match args.cmd {
        None => {
            // Default: full reconcile
            let repo_slug = resolve_sync_repo(args.repo.as_deref(), &repo_root)?;
            let project_tag = config::read_project_tag(&repo_root)?;

            let scm = GhCli;
            let report = reconcile(
                &calendar,
                &scm,
                &repo_slug,
                project_tag.as_deref(),
                args.dry_run,
            )
            .await?;
            println!("{report}");
        }
        Some(SyncCmd::ScheduleTest(st)) => {
            let tag = if st.tag == "*" { None } else { Some(st.tag.as_str()) };
            let report =
                schedule_test_slot(&calendar, tag, &st.subject, st.dry_run || args.dry_run)
                    .await?;
            println!("{report}");
        }
    }

    Ok(())
}

fn resolve_sync_repo(explicit: Option<&str>, repo_root: &std::path::Path) -> Result<conductr_orchestrate::RepoSlug> {
    if let Some(slug) = explicit {
        let (owner, repo) = slug
            .split_once('/')
            .with_context(|| format!("invalid --repo `{slug}` (expected owner/repo)"))?;
        return Ok(conductr_orchestrate::RepoSlug::new(owner, repo));
    }
    // Fall back to `.conductr`
    let raw = std::fs::read_to_string(repo_root.join(".conductr")).ok();
    if let Some(content) = raw {
        #[derive(serde::Deserialize)]
        struct Minimal { repo: Option<String> }
        if let Ok(cfg) = toml::from_str::<Minimal>(&content) {
            if let Some(slug) = cfg.repo {
                let (owner, repo) = slug
                    .split_once('/')
                    .with_context(|| format!("`.conductr` repo field `{slug}` is not owner/repo"))?;
                return Ok(conductr_orchestrate::RepoSlug::new(owner, repo));
            }
        }
    }
    anyhow::bail!(
        "no repo specified. Pass --repo owner/repo or set `repo` in .conductr"
    )
}

// ── Idle ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Parser)]
struct IdleArgs {
    /// Path to the repo root containing `.conductr` (defaults to current directory).
    #[arg(long)]
    repo_path: Option<PathBuf>,
    /// Print what would happen without creating a session or sending commands.
    #[arg(long)]
    dry_run: bool,
}

async fn run_idle(args: IdleArgs) -> Result<()> {
    let session = "conductr-idle";
    let cwd = args
        .repo_path
        .map(|p| p.to_string_lossy().into_owned())
        .or_else(|| std::env::current_dir().ok().map(|p| p.to_string_lossy().into_owned()))
        .unwrap_or_else(|| ".".to_string());

    let skill_cmd = "/idle";
    let claude_cmd = "claude --dangerously-skip-permissions";

    if args.dry_run {
        let tmux = Tmux::new();
        return run_idle_dry(&tmux, session, &cwd, claude_cmd, skill_cmd).await;
    }

    let tmux = Tmux::new();
    let state = ensure_session(&tmux, session, &cwd)
        .await
        .with_context(|| format!("ensuring tmux session '{session}'"))?;

    match &state {
        SessionState::Existing(Health::Working { activity }) => {
            println!("idle: session '{session}' is busy ({activity}); skipping");
            return Ok(());
        }
        SessionState::Existing(Health::Unknown { reason }) => {
            println!("idle: session '{session}' is unclassified ({reason}); skipping");
            return Ok(());
        }
        SessionState::Existing(Health::Crashed { .. }) => {
            println!("idle: session '{session}' crashed — restarting Claude");
            tmux.send_line(session, claude_cmd)
                .await
                .context("restarting Claude after crash")?;
            wait_for_idle(&tmux, session).await?;
        }
        SessionState::Created => {
            println!("idle: created session '{session}' — starting Claude");
            tmux.send_line(session, claude_cmd)
                .await
                .context("starting Claude in new session")?;
            wait_for_idle(&tmux, session).await?;
        }
        SessionState::Existing(Health::Idle { .. }) => {
            println!("idle: session '{session}' is idle — sending idle prompt");
        }
    }

    println!("idle: sending: {skill_cmd}");
    tmux.send_line(session, skill_cmd)
        .await
        .context("sending idle skill command")?;

    Ok(())
}

async fn run_idle_dry(
    tmux: &Tmux,
    session: &str,
    cwd: &str,
    claude_cmd: &str,
    skill_cmd: &str,
) -> Result<()> {
    let sessions = match tmux.list_sessions().await {
        Ok(s) => s,
        Err(TmuxError::NoServer) | Err(TmuxError::NotInstalled) => {
            println!("idle: tmux not running");
            println!("idle: → would create session '{session}' at cwd={cwd}");
            println!("idle: → would start Claude: `{claude_cmd}`");
            println!("idle: → would send: `{skill_cmd}`");
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };

    if sessions.iter().any(|s| s.name == session) {
        match diagnose_one(tmux, session).await {
            Ok(d) => {
                println!("idle: session '{session}' exists ({})", health_label(&d.health));
                match &d.health {
                    Health::Working { activity } => {
                        println!("idle: → would skip (busy: {activity})");
                    }
                    Health::Unknown { reason } => {
                        println!("idle: → would skip (unclassified: {reason})");
                    }
                    Health::Crashed { .. } => {
                        println!("idle: → would restart Claude: `{claude_cmd}`");
                        println!("idle: → would send: `{skill_cmd}`");
                    }
                    Health::Idle { .. } => {
                        println!("idle: → would send: `{skill_cmd}`");
                    }
                }
            }
            Err(e) => println!("idle: session '{session}' exists but diagnose failed: {e}"),
        }
    } else {
        println!("idle: session '{session}' does not exist");
        println!("idle: cwd = {cwd}");
        println!("idle: → would create session '{session}'");
        println!("idle: → would start Claude: `{claude_cmd}`");
        println!("idle: → would send: `{skill_cmd}`");
    }

    Ok(())
}

fn build_architect_plan_prompt(issues: &[String]) -> String {
    format!("/architect plan {}", issues.join(" "))
}

async fn run_architect_plan_dry(
    tmux: &Tmux,
    session: &str,
    cwd: &str,
    claude_cmd: &str,
    skill_cmd: &str,
) -> Result<()> {
    let sessions = match tmux.list_sessions().await {
        Ok(s) => s,
        Err(TmuxError::NoServer) | Err(TmuxError::NotInstalled) => {
            println!("plan: tmux not running");
            println!("plan: → would create session '{session}' at cwd={cwd}");
            println!("plan: → would start Claude: `{claude_cmd}`");
            println!("plan: → would send: `{skill_cmd}`");
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };


    if sessions.iter().any(|s| s.name == session) {
        match diagnose_one(tmux, session).await {
            Ok(d) => {
                println!("plan: session '{session}' exists ({})", health_label(&d.health));
                match &d.health {
                    Health::Working { activity } => {
                        println!("plan: → would skip (busy: {activity})");
                    }
                    Health::Unknown { reason } => {
                        println!("plan: → would skip (unclassified: {reason})");
                    }
                    Health::Crashed { .. } => {
                        println!("plan: → would restart Claude: `{claude_cmd}`");
                        println!("plan: → would send: `{skill_cmd}`");
                    }
                    Health::Idle { .. } => {
                        println!("plan: → would send: `{skill_cmd}`");
                    }
                }
            }
            Err(e) => println!("plan: session '{session}' exists but diagnose failed: {e}"),
        }
    } else {
        println!("plan: session '{session}' does not exist");
        println!("plan: cwd = {cwd}");
        println!("plan: → would create session '{session}'");
        println!("plan: → would start Claude: `{claude_cmd}`");
        println!("plan: → would send: `{skill_cmd}`");
    }
    Ok(())
}

// ── Change overview ───────────────────────────────────────────────────────────

#[derive(Debug, Parser)]
struct ChangeOverviewArgs {
    /// Comparison base (branch, tag, commit SHA, or `#<pr>`). Defaults to `origin/main`.
    base: Option<String>,
    /// Path to the repo root containing `.conductr` (defaults to current directory).
    #[arg(long)]
    repo_path: Option<PathBuf>,
    /// Print what would happen without creating a session or sending commands.
    #[arg(long)]
    dry_run: bool,
}

async fn run_change_overview(args: ChangeOverviewArgs) -> Result<()> {
    let session = "conductr-change-overview";
    let cwd = args
        .repo_path
        .map(|p| p.to_string_lossy().into_owned())
        .or_else(|| std::env::current_dir().ok().map(|p| p.to_string_lossy().into_owned()))
        .unwrap_or_else(|| ".".to_string());

    let skill_cmd = match args.base.as_deref() {
        Some(b) => format!("/change-overview {b}"),
        None => "/change-overview".to_string(),
    };
    let claude_cmd = "claude --dangerously-skip-permissions";

    if args.dry_run {
        let tmux = Tmux::new();
        return run_claude_skill_dry(&tmux, "change-overview", session, &cwd, claude_cmd, &skill_cmd).await;
    }

    let tmux = Tmux::new();
    let state = ensure_session(&tmux, session, &cwd)
        .await
        .with_context(|| format!("ensuring tmux session '{session}'"))?;

    match &state {
        SessionState::Existing(Health::Working { activity }) => {
            println!("change-overview: session '{session}' is busy ({activity}); skipping");
            return Ok(());
        }
        SessionState::Existing(Health::Unknown { reason }) => {
            println!("change-overview: session '{session}' is unclassified ({reason}); skipping");
            return Ok(());
        }
        SessionState::Existing(Health::Crashed { .. }) => {
            println!("change-overview: session '{session}' crashed — restarting Claude");
            tmux.send_line(session, claude_cmd)
                .await
                .context("restarting Claude after crash")?;
            wait_for_idle(&tmux, session).await?;
        }
        SessionState::Created => {
            println!("change-overview: created session '{session}' — starting Claude");
            tmux.send_line(session, claude_cmd)
                .await
                .context("starting Claude in new session")?;
            wait_for_idle(&tmux, session).await?;
        }
        SessionState::Existing(Health::Idle { .. }) => {
            println!("change-overview: session '{session}' is idle — sending change-overview prompt");
        }
    }

    println!("change-overview: sending: {skill_cmd}");
    tmux.send_line(session, &skill_cmd)
        .await
        .context("sending change-overview skill command")?;

    Ok(())
}

// ── Human-ticket draft ────────────────────────────────────────────────────────

#[derive(Debug, Parser)]
struct HumanTicketDraftArgs {
    /// Change summary to compare tickets against. Inline text or a path to a markdown file.
    #[arg(long)]
    context: Option<String>,
    /// Repository to scan, as `owner/repo`. Defaults to the current repo.
    #[arg(long)]
    repo: Option<String>,
    /// Cap the number of drafted tickets (default 5).
    #[arg(long)]
    limit: Option<usize>,
    /// Print what would happen without creating a session or sending commands.
    #[arg(long)]
    dry_run: bool,
}

async fn run_human_ticket_draft(args: HumanTicketDraftArgs) -> Result<()> {
    let session = "conductr-human-ticket-draft";
    let cwd = std::env::current_dir()
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| ".".to_string());

    let mut skill_cmd = String::from("/human-ticket-draft");
    if let Some(c) = &args.context {
        skill_cmd.push_str(&format!(" --context {c}"));
    }
    if let Some(r) = &args.repo {
        skill_cmd.push_str(&format!(" --repo {r}"));
    }
    if let Some(l) = args.limit {
        skill_cmd.push_str(&format!(" --limit {l}"));
    }
    let claude_cmd = "claude --dangerously-skip-permissions";

    if args.dry_run {
        let tmux = Tmux::new();
        return run_claude_skill_dry(&tmux, "human-ticket-draft", session, &cwd, claude_cmd, &skill_cmd).await;
    }

    let tmux = Tmux::new();
    let state = ensure_session(&tmux, session, &cwd)
        .await
        .with_context(|| format!("ensuring tmux session '{session}'"))?;

    match &state {
        SessionState::Existing(Health::Working { activity }) => {
            println!("human-ticket-draft: session '{session}' is busy ({activity}); skipping");
            return Ok(());
        }
        SessionState::Existing(Health::Unknown { reason }) => {
            println!("human-ticket-draft: session '{session}' is unclassified ({reason}); skipping");
            return Ok(());
        }
        SessionState::Existing(Health::Crashed { .. }) => {
            println!("human-ticket-draft: session '{session}' crashed — restarting Claude");
            tmux.send_line(session, claude_cmd)
                .await
                .context("restarting Claude after crash")?;
            wait_for_idle(&tmux, session).await?;
        }
        SessionState::Created => {
            println!("human-ticket-draft: created session '{session}' — starting Claude");
            tmux.send_line(session, claude_cmd)
                .await
                .context("starting Claude in new session")?;
            wait_for_idle(&tmux, session).await?;
        }
        SessionState::Existing(Health::Idle { .. }) => {
            println!("human-ticket-draft: session '{session}' is idle — sending prompt");
        }
    }

    println!("human-ticket-draft: sending: {skill_cmd}");
    tmux.send_line(session, &skill_cmd)
        .await
        .context("sending human-ticket-draft skill command")?;

    Ok(())
}

async fn run_claude_skill_dry(
    tmux: &Tmux,
    label: &str,
    session: &str,
    cwd: &str,
    claude_cmd: &str,
    skill_cmd: &str,
) -> Result<()> {
    let sessions = match tmux.list_sessions().await {
        Ok(s) => s,
        Err(TmuxError::NoServer) | Err(TmuxError::NotInstalled) => {
            println!("{label}: tmux not running");
            println!("{label}: → would create session '{session}' at cwd={cwd}");
            println!("{label}: → would start Claude: `{claude_cmd}`");
            println!("{label}: → would send: `{skill_cmd}`");
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };

    if sessions.iter().any(|s| s.name == session) {
        println!("{label}: session '{session}' exists; → would send: `{skill_cmd}`");
    } else {
        println!("{label}: → would create session '{session}' at cwd={cwd}");
        println!("{label}: → would start Claude: `{claude_cmd}`");
        println!("{label}: → would send: `{skill_cmd}`");
    }

    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
    use local_detect::{ProbeRow, ProbeStatus};

    const HEAL_REGISTRY: &str = r#"
[[projects]]
tag    = "alpha"
repo   = "owner/alpha"
path   = "/tmp/alpha"
status = "active"

[[projects]]
tag    = "beta"
repo   = "owner/beta"
path   = "/tmp/beta"
status = "pending"
"#;

    fn write_registry(body: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".conductr"), body).unwrap();
        dir
    }

    #[test]
    fn heal_scope_no_repo_with_registry_provisions_all_active() {
        let dir = write_registry(HEAL_REGISTRY);
        let scope = resolve_heal_scope(None, Some(&dir.path().join(".conductr")))
            .unwrap()
            .expect("registry present → Some scope");
        assert!(scope.tag_filter.is_none(), "no --repo → provision all active");
        assert!(scope.scoped_session.is_none());
    }

    #[test]
    fn heal_scope_repo_scopes_to_active_project() {
        let dir = write_registry(HEAL_REGISTRY);
        let scope = resolve_heal_scope(Some("owner/alpha"), Some(&dir.path().join(".conductr")))
            .unwrap()
            .unwrap();
        assert_eq!(scope.tag_filter.as_deref(), Some("alpha"));
        assert_eq!(scope.scoped_session.as_deref(), Some("conductr-alpha"));
    }

    #[test]
    fn heal_scope_repo_rejects_pending_project() {
        let dir = write_registry(HEAL_REGISTRY);
        let err = resolve_heal_scope(Some("owner/beta"), Some(&dir.path().join(".conductr")))
            .unwrap_err();
        assert!(err.to_string().contains("pending"), "{err}");
    }

    #[test]
    fn heal_scope_repo_errors_on_unknown_slug() {
        let dir = write_registry(HEAL_REGISTRY);
        let err = resolve_heal_scope(Some("owner/ghost"), Some(&dir.path().join(".conductr")))
            .unwrap_err();
        assert!(err.to_string().contains("no active project"), "{err}");
    }

    #[test]
    fn heal_scope_no_registry_degrades_to_none_without_repo() {
        let missing = std::path::Path::new("/nonexistent/dir/.conductr");
        assert!(resolve_heal_scope(None, Some(missing)).unwrap().is_none());
    }

    #[test]
    fn heal_scope_repo_without_registry_errors() {
        let missing = std::path::Path::new("/nonexistent/dir/.conductr");
        assert!(resolve_heal_scope(Some("owner/alpha"), Some(missing)).is_err());
    }

    /// Every skill with `cli_parity: true` in its SKILL.md frontmatter must
    /// have a matching top-level CLI subcommand.  This test picks up new skills
    /// automatically once they set the flag — no manual registration needed.
    #[test]
    fn cli_parity_skills_have_matching_subcommands() {
        let skills_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("skills"));

        let skills_dir = match skills_dir {
            Some(p) if p.is_dir() => p,
            _ => return, // running outside the repo — skip
        };

        let cmd = Cli::command();

        for entry in std::fs::read_dir(&skills_dir).unwrap() {
            let entry = entry.unwrap();
            let skill_md = entry.path().join("SKILL.md");
            if !skill_md.exists() {
                continue;
            }
            let content = std::fs::read_to_string(&skill_md).unwrap_or_default();
            if !content.contains("cli_parity: true") {
                continue;
            }
            let skill_name = entry.file_name();
            let skill_name = skill_name.to_string_lossy();
            assert!(
                cmd.find_subcommand(skill_name.as_ref()).is_some(),
                "skill '{skill_name}' declares cli_parity: true but no matching CLI subcommand exists"
            );
        }
    }

    fn probe(provider: &'static str, status: ProbeStatus) -> ProbeRow {
        ProbeRow { provider, status, detail: String::new() }
    }

    #[test]
    fn pick_first_present_provider() {
        let probes = vec![
            probe("ollama", ProbeStatus::Missing),
            probe("llamacpp", ProbeStatus::Present),
            probe("qwen3:27b", ProbeStatus::Missing),
        ];
        let kind = pick_provider_from_probes(&probes);
        assert_eq!(kind, Some(wiring::LocalAgentKind::LlamaCpp));
    }

    #[test]
    fn pick_ollama_when_first_present() {
        let probes = vec![
            probe("ollama", ProbeStatus::Present),
            probe("llamacpp", ProbeStatus::Present),
        ];
        let kind = pick_provider_from_probes(&probes);
        assert_eq!(kind, Some(wiring::LocalAgentKind::Ollama));
    }

    #[test]
    fn pick_returns_none_when_all_missing() {
        let probes = vec![
            probe("ollama", ProbeStatus::Missing),
            probe("llamacpp", ProbeStatus::Missing),
        ];
        let kind = pick_provider_from_probes(&probes);
        assert_eq!(kind, None);
    }

    #[test]
    fn pick_skips_unknown_provider_names() {
        let probes = vec![
            probe("qwen3:27b", ProbeStatus::Present),
            probe("llamacpp", ProbeStatus::Present),
        ];
        // "qwen3:27b" is not a valid LocalAgentKind; should skip to "llamacpp"
        let kind = pick_provider_from_probes(&probes);
        assert_eq!(kind, Some(wiring::LocalAgentKind::LlamaCpp));
    }

    #[tokio::test]
    async fn resolve_provider_honours_explicit_flag() {
        // When an explicit provider is given, it wins regardless of env/config.
        let (kind, model) =
            resolve_provider(Some(wiring::LocalAgentKind::Ollama)).await.unwrap();
        assert_eq!(kind, wiring::LocalAgentKind::Ollama);
        assert!(model.is_none());
    }

    // ── insert_into_cadence_section ───────────────────────────────────────────

    #[test]
    fn insert_adds_entry_when_cadence_section_exists() {
        let content = "project_tag = \"foo\"\n\n[cadence]\norchestrate = \"*/30 * * * *\"\n";
        let result = insert_into_cadence_section(content, "idle = \"*/30 * * * *\"");
        assert!(result.contains("idle = \"*/30 * * * *\""), "got: {result}");
        assert!(result.contains("orchestrate = \"*/30 * * * *\""), "got: {result}");
        assert!(result.contains("[cadence]"), "got: {result}");
        // Only one [cadence] header
        assert_eq!(result.matches("[cadence]").count(), 1);
    }

    #[test]
    fn insert_creates_cadence_section_when_absent() {
        let content = "project_tag = \"bar\"\nrepo = \"owner/bar\"\n";
        let result = insert_into_cadence_section(content, "orchestrate = \"*/30 * * * *\"");
        assert!(result.contains("[cadence]"), "got: {result}");
        assert!(result.contains("orchestrate = \"*/30 * * * *\""), "got: {result}");
        assert!(result.ends_with('\n'));
    }

    #[test]
    fn insert_before_next_section_header() {
        let content = "project_tag = \"foo\"\n\n[cadence]\norchestrate = \"*/30 * * * *\"\n\n[local]\nprovider = \"ollama\"\n";
        let result = insert_into_cadence_section(content, "idle = \"*/30 * * * *\"");
        // idle should appear before [local]
        let cadence_pos = result.find("[cadence]").unwrap();
        let idle_pos = result.find("idle = ").unwrap();
        let local_pos = result.find("[local]").unwrap();
        assert!(cadence_pos < idle_pos, "idle should be after [cadence]");
        assert!(idle_pos < local_pos, "idle should be before [local]");
    }

    #[test]
    fn insert_result_ends_with_newline() {
        let content = "project_tag = \"foo\"";
        let result = insert_into_cadence_section(content, "orchestrate = \"*/30 * * * *\"");
        assert!(result.ends_with('\n'));
    }
}
