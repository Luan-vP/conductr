//! `conductr` CLI: orchestrate, instance, schedule, tasks.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use conductr_orchestrate::{GhCli, Orchestrator, OrchestratorConfig, RepoSlug};
use conductr_schedule::{parse, render_ascii};
use conductr_tasks::beads::Beads;

#[derive(Debug, Parser)]
#[command(name = "conductr", version, about = "Scheduling and orchestration for agents and people")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    /// Drive `@claude` GitHub-issue orchestration (poorchestrator port).
    Orchestrate(OrchestrateArgs),
    /// Cloud instance management (stubbed; agentic port pending).
    Instance(InstanceArgs),
    /// Musical-notation time patterns.
    Schedule(ScheduleArgs),
    /// Task tracking via beads (`br`) and Notion.
    Tasks(TasksArgs),
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
        Cmd::Orchestrate(a) => run_orchestrate(a).await,
        Cmd::Instance(a) => run_instance(a).await,
        Cmd::Schedule(a) => run_schedule(a),
        Cmd::Tasks(a) => run_tasks(a).await,
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
