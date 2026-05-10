//! `conductr` CLI: orchestrate, instance, schedule, tasks.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use conductr_orchestrate::{GhCli, Orchestrator, OrchestratorConfig, RepoSlug};
use conductr_pod::{
    diagnose_all, diagnose_one, ensure_session, heal_all, pick_idle, Diagnosis, FreeOpts, Health,
    SessionState, Tmux, TmuxError,
};
use conductr_schedule::{parse, render_ascii};
use conductr_tasks::beads::Beads;
use serde::Serialize;

#[derive(Debug, Parser)]
#[command(name = "conductr", version, about = "Scheduling and orchestration for agents and people")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    /// Cron-friendly entry point: ensure a tmux session exists and trigger an orchestrate pass.
    Begin(BeginArgs),
    /// Drive `@claude` GitHub-issue orchestration (poorchestrator port).
    Orchestrate(OrchestrateArgs),
    /// Cloud instance management (stubbed; agentic port pending).
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
    /// Print the plan without writing to beads or restarting sessions.
    #[arg(long)]
    dry_run: bool,
    /// Do not restart sessions after capturing state.
    #[arg(long)]
    no_restart: bool,
    /// Command typed into the pane to bring Claude back up after restart.
    #[arg(long, default_value = "claude")]
    command: String,
    /// Priority for created beads issues (0-4).
    #[arg(long, default_value_t = 2)]
    priority: u8,
    /// Always emit the JSON manifest of saved state (default behaviour).
    #[arg(long, default_value_t = true)]
    json: bool,
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
/// JSON to update Notion (or other external trackers) before restart.
#[derive(Debug, Serialize)]
struct SaveStateEntry {
    session: String,
    health: &'static str,
    cwd: Option<String>,
    last_message: Option<String>,
    tail: Vec<String>,
    /// `br-…` id of the beads issue we created for this session (None if no
    /// recoverable work or `--dry-run`).
    beads_id: Option<String>,
    /// What we did to the pane: `restarted`, `would-restart`, `skipped:<why>`.
    restart: String,
}

async fn run_save_state(args: SaveStateArgs) -> Result<()> {
    let tmux = Tmux::new();
    let pattern = pod_pattern(args.pattern.as_deref(), args.all);
    let diagnoses = diagnose_all(&tmux, pattern).await?;

    let beads = Beads::new();
    let mut manifest: Vec<SaveStateEntry> = Vec::with_capacity(diagnoses.len());

    for d in &diagnoses {
        let recoverable = recoverable_summary(d);
        let beads_id = if let Some(summary) = &recoverable {
            if args.dry_run {
                None
            } else {
                Some(create_recovery_issue(&beads, d, summary, args.priority).await?)
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
            beads_id,
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
    beads: &Beads,
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
    let labels = ["thread-recovery", &d.session.name];
    let task = beads
        .create_full(&title, Some(priority), Some(&body), &labels)
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
    let mut cfg = OrchestratorConfig::new(RepoSlug::new(owner, repo));
    cfg.dry_run = args.dry_run;
    cfg.poll_interval = std::time::Duration::from_secs(args.poll_secs);
    cfg.default_human_assignee = args.human_assignee;
    let orch = Orchestrator::new(GhCli, cfg);
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
    })
}

async fn run_instance(args: InstanceArgs) -> Result<()> {
    match args.cmd {
        InstanceCmd::SpinUp { name } => {
            anyhow::bail!(
                "instance spin-up not implemented yet (port from agentic). Requested: {name}"
            );
        }
        InstanceCmd::List => {
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
    }
}

async fn run_tasks(args: TasksArgs) -> Result<()> {
    let beads = Beads::new();
    match args.cmd {
        TasksCmd::List { ready } => {
            let tasks = if ready { beads.list_ready().await? } else { beads.list().await? };
            println!("{}", serde_json::to_string_pretty(&tasks)?);
            Ok(())
        }
        TasksCmd::Create { title, priority } => {
            let t = beads.create(&title, priority).await?;
            println!("{}", serde_json::to_string_pretty(&t)?);
            Ok(())
        }
        TasksCmd::SyncToNotion { database } => {
            let notion = conductr_tasks::notion::Notion::from_env()?;
            let report = conductr_tasks::sync::beads_to_notion(&beads, &notion, &database).await?;
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
