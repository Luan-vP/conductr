//! `conductr` CLI: orchestrate, instance, schedule, tasks, setup, mail, local, cadence.

mod cadence;
mod config;
mod local_detect;
mod wiring;

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use conductr_adapters::gh_cli::GhCli;
use conductr_adapters::local_ci::LocalCiAdapter;
use conductr_adapters::mail_fs::FsMailbox;
use conductr_adapters::tmux::Tmux;
use conductr_adapters::{beads::Beads, notion::Notion};
use conductr_core::ports::{LocalCi, Mailbox, TmuxAgent};
use conductr_core::types::{CiMode, LocalCiConfig, MailKind};
use conductr_mail::dedup::check_scope;
use conductr_mail::synthesise::request_synthesis;
use conductr_orchestrate::{Orchestrator, OrchestratorConfig, RepoSlug};
use conductr_pod::{
    diagnose_all, diagnose_one, ensure_session, heal_all, pick_idle, Diagnosis, FreeOpts, Health,
    SessionState, TmuxError,
};
use conductr_schedule::{parse, parse_plan, pattern_to_dsl, plan_to_pattern, render_ascii};
use conductr_setup::wizard::WizardMode;
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
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    /// Cron-friendly entry point: ensure a tmux session exists and trigger an orchestrate pass.
    Begin(BeginArgs),
    /// Drive `@claude` GitHub-issue orchestration.
    Orchestrate(OrchestrateArgs),
    /// Cloud instance management (stubbed).
    Instance(InstanceArgs),
    /// Musical-notation time patterns.
    Schedule(ScheduleArgs),
    /// Task tracking via beads (`br`) and Notion.
    Tasks(TasksArgs),
    /// Inspect the local Claude Code pod (tmux sessions on this host).
    Diagnose(DiagnoseArgs),
    /// Find an idle Claude Code session and print its tmux attach command.
    Free(FreeArgs),
    /// Restart any crashed Claude Code sessions in the pod.
    Heal(HealArgs),
    /// Snapshot unfinished work to beads then restart pod sessions.
    SaveState(SaveStateArgs),
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
struct BeginArgs {
    /// Project tag — the tmux session will be named `conductr-<tag>`.
    #[arg(long)]
    tag: String,
    /// `owner/repo` slug forwarded to `conductr orchestrate --repo`.
    #[arg(long)]
    repo: Option<String>,
    /// Working directory for the new tmux session (defaults to the current directory).
    #[arg(long)]
    cwd: Option<PathBuf>,
    /// Leave Claude running its own polling loop; omit for a single `--once` pass (cron default).
    #[arg(long)]
    continuous: bool,
    /// Print the plan without creating sessions, starting Claude, or sending keys.
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
    /// Command typed into the pane to bring Claude back up.
    #[arg(long, default_value = "claude")]
    command: String,
    /// Emit machine-readable JSON instead of a table.
    #[arg(long)]
    json: bool,
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
        Cmd::Diagnose(a) => run_diagnose(a).await,
        Cmd::Free(a) => run_free(a).await,
        Cmd::Heal(a) => run_heal(a).await,
        Cmd::SaveState(a) => run_save_state(a).await,
        Cmd::Setup(a) => run_setup(a).await,
        Cmd::Mail(a) => run_mail(a).await,
        Cmd::Local(a) => run_local(a).await,
        Cmd::Cadence(a) => run_cadence(a),
        Cmd::RunTask(a) => run_run_task(a).await,
    }
}

fn pod_pattern<'a>(explicit: Option<&'a str>, all: bool) -> Option<&'a str> {
    if all {
        None
    } else {
        explicit.or(Some("claude"))
    }
}

// ---------------------------------------------------------------------------
// begin
// ---------------------------------------------------------------------------

async fn run_begin(args: BeginArgs) -> Result<()> {
    let session = format!("conductr-{}", args.tag);
    let cwd = args
        .cwd
        .map(|p| p.to_string_lossy().into_owned())
        .or_else(|| std::env::current_dir().ok().map(|p| p.to_string_lossy().into_owned()))
        .unwrap_or_else(|| ".".to_string());

    let orchestrate_prompt = build_orchestrate_prompt(args.repo.as_deref(), args.continuous);

    // Auto mode via --dangerously-skip-permissions (CLI flag, most cron-friendly).
    // Remote control is provided by tmux send-keys injection (what conductr already does).
    let claude_cmd = "claude --dangerously-skip-permissions";

    if args.dry_run {
        let tmux = Tmux::new();
        return run_begin_dry(&tmux, &session, &cwd, claude_cmd, &orchestrate_prompt).await;
    }

    let lock_path = begin_lockfile_path(&args.tag)?;
    if let Some(pid) = acquire_begin_lock(&lock_path)? {
        println!(
            "begin: another conductr begin is already running for tag '{}' (pid {pid}); skipping",
            args.tag
        );
        return Ok(());
    }
    let _lock = LockGuard(lock_path);

    let tmux = Tmux::new();
    let state = ensure_session(&tmux, &session, &cwd)
        .await
        .with_context(|| format!("ensuring tmux session '{session}'"))?;

    match &state {
        SessionState::Existing(Health::Working { activity }) => {
            println!("begin: session '{session}' is busy ({activity}); skipping");
            return Ok(());
        }
        SessionState::Existing(Health::Unknown { reason }) => {
            println!("begin: session '{session}' is unclassified ({reason}); skipping");
            return Ok(());
        }
        SessionState::Existing(Health::Crashed { .. }) => {
            println!("begin: session '{session}' crashed — restarting Claude");
            tmux.send_line(&session, claude_cmd)
                .await
                .context("restarting Claude after crash")?;
            wait_for_idle(&tmux, &session).await?;
        }
        SessionState::Created => {
            println!("begin: created session '{session}' — starting Claude");
            tmux.send_line(&session, claude_cmd)
                .await
                .context("starting Claude in new session")?;
            wait_for_idle(&tmux, &session).await?;
        }
        SessionState::Existing(Health::Idle { .. }) => {
            println!("begin: session '{session}' is idle — sending orchestrate prompt");
        }
    }

    println!("begin: sending: {orchestrate_prompt}");
    tmux.send_line(&session, &orchestrate_prompt)
        .await
        .context("sending orchestrate prompt")?;

    Ok(())
}

fn build_orchestrate_prompt(repo: Option<&str>, continuous: bool) -> String {
    let mut cmd = String::from("conductr orchestrate");
    if let Some(r) = repo {
        cmd.push_str(" --repo ");
        cmd.push_str(r);
    }
    if !continuous {
        cmd.push_str(" --once");
    }
    cmd
}

async fn run_begin_dry(
    tmux: &Tmux,
    session: &str,
    cwd: &str,
    claude_cmd: &str,
    orchestrate_prompt: &str,
) -> Result<()> {
    let sessions = match tmux.list_sessions().await {
        Ok(s) => s,
        Err(TmuxError::NoServer) | Err(TmuxError::NotInstalled) => {
            println!("plan: tmux not running");
            println!("plan: → would create session '{session}' at cwd={cwd}");
            println!("plan: → would start Claude: `{claude_cmd}`");
            println!("plan: → would send: `{orchestrate_prompt}`");
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
                        println!("plan: → would send: `{orchestrate_prompt}`");
                    }
                    Health::Idle { .. } => {
                        println!("plan: → would send: `{orchestrate_prompt}`");
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
        println!("plan: → would send: `{orchestrate_prompt}`");
    }
    Ok(())
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

fn begin_lockfile_path(tag: &str) -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME env var not set")?;
    let dir = PathBuf::from(home).join(".conductr");
    std::fs::create_dir_all(&dir).context("creating ~/.conductr")?;
    Ok(dir.join(format!("begin-{tag}.lock")))
}

/// Try to acquire the lock. Returns `Some(pid)` if another process holds it,
/// `None` if this process now holds it.
fn acquire_begin_lock(path: &PathBuf) -> Result<Option<u32>> {
    if path.exists() {
        if let Ok(content) = std::fs::read_to_string(path) {
            if let Ok(pid) = content.trim().parse::<u32>() {
                if pid_alive(pid) {
                    return Ok(Some(pid));
                }
            }
        }
        // Stale lock — remove it.
        let _ = std::fs::remove_file(path);
    }
    std::fs::write(path, std::process::id().to_string().as_bytes())
        .with_context(|| format!("writing lockfile {}", path.display()))?;
    Ok(None)
}

fn pid_alive(pid: u32) -> bool {
    // Linux: /proc/<pid> exists iff the process is alive.
    std::fs::metadata(format!("/proc/{pid}")).is_ok()
}

struct LockGuard(PathBuf);

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
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

async fn run_heal(args: HealArgs) -> Result<()> {
    let tmux = Tmux::new();
    let pattern = pod_pattern(args.pattern.as_deref(), args.all);
    let outcomes = heal_all(&tmux, pattern, &args.command, args.dry_run).await?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&outcomes)?);
        return Ok(());
    }
    if outcomes.is_empty() {
        println!("no sessions matched (pattern={:?})", pattern.unwrap_or("*"));
        return Ok(());
    }
    let mut healed = 0usize;
    let mut skipped = 0usize;
    for o in &outcomes {
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
    println!("\n{healed} restarted, {skipped} skipped, {} total", outcomes.len());
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
        "Captured by `conductr save-state` at {now}.\n\n\
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
    }
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

    let local_ci: Option<std::sync::Arc<dyn LocalCi>> = if ci_section.commands.is_empty() {
        None
    } else {
        Some(std::sync::Arc::new(LocalCiAdapter::new(
            repo_root,
            LocalCiConfig {
                commands: ci_section.commands,
                timeout_secs: ci_section.timeout_secs,
                mode: ci_mode,
            },
        )))
    };

    let mut cfg = OrchestratorConfig::new(RepoSlug::new(owner, repo));
    cfg.dry_run = args.dry_run;
    cfg.poll_interval = std::time::Duration::from_secs(args.poll_secs);
    cfg.default_human_assignee = args.human_assignee;
    cfg.ci_mode = ci_mode;

    let mut orch = Orchestrator::new(GhCli, cfg);
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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use local_detect::{ProbeRow, ProbeStatus};

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
}
