//! Sync the host crontab or launchd from `.conductr [cadence]`,
//! and maintain `.conductr-local` recording realized schedules.
//!
//! Crontab entries take the form:
//!
//!     # conductr-cron: <project_tag>-<task>
//!     <cron-expr> bash -lc 'conductr <command>' >> <log-path> 2>&1
//!
//! `.conductr-local` (gitignored) records the realized schedule so that
//! `cadence status` can detect drift and `cadence remove` can clean up.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{anyhow, Context, Result};
use chrono::{Timelike, Utc};
use serde::{Deserialize, Serialize};

const MARKER_PREFIX: &str = "# conductr-cron:";
const LOCAL_FILE: &str = ".conductr-local";

// ── Public types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mechanism {
    Crontab,
    Launchd,
}

impl Mechanism {
    pub fn as_str(self) -> &'static str {
        match self {
            Mechanism::Crontab => "crontab",
            Mechanism::Launchd => "launchd",
        }
    }
}

// ── Config types ──────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct ConductrConfig {
    project_tag: String,
    repo: Option<String>,
    #[serde(default)]
    cadence: BTreeMap<String, String>,
}

// ── .conductr-local schema ────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Default)]
struct LocalFile {
    machine: String,
    generated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    repo: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    schedule: Vec<LocalSchedule>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct LocalSchedule {
    task: String,
    mechanism: String,
    command: String,
    cadence: String,
    installed_at: String,
    log_stdout: String,
    log_stderr: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    marker: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    plist: Option<String>,
}

// ── Public API ────────────────────────────────────────────────────────────────

pub fn sync(repo_path: &Path, dry_run: bool, mechanism: Mechanism) -> Result<String> {
    let cfg = read_config(repo_path)?;
    match mechanism {
        Mechanism::Crontab => {
            let log_dir = log_dir_default()?;
            let new_lines = generate_lines(&cfg, &log_dir);
            sync_crontab(repo_path, &cfg, &new_lines, &log_dir, dry_run)
        }
        Mechanism::Launchd => sync_launchd(repo_path, &cfg, dry_run),
    }
}

pub fn status(repo_path: &Path) -> Result<String> {
    let local_path = repo_path.join(LOCAL_FILE);

    if !local_path.exists() {
        return Ok(
            "no .conductr-local found — run `conductr cadence sync` to install".to_string(),
        );
    }

    let local_raw = std::fs::read_to_string(&local_path)
        .with_context(|| format!("reading {}", local_path.display()))?;
    let local: LocalFile = toml::from_str(&local_raw)
        .with_context(|| format!("parsing {}", local_path.display()))?;

    let cfg = read_config(repo_path).ok();
    let mut lines = Vec::new();

    lines.push(format!("machine:      {}", local.machine));
    lines.push(format!("generated_at: {}", local.generated_at));
    if let Some(repo) = &local.repo {
        lines.push(format!("repo:         {repo}"));
    }

    if local.schedule.is_empty() {
        lines.push(String::new());
        lines.push("no schedules installed".to_string());
        return Ok(lines.join("\n"));
    }

    for sched in &local.schedule {
        lines.push(String::new());
        let drift_tag = if let Some(cfg) = &cfg {
            match cfg.cadence.get(&sched.task) {
                Some(declared) if declared != &sched.cadence => {
                    format!(
                        " [DRIFT: declared '{}', installed '{}']",
                        declared, sched.cadence
                    )
                }
                Some(_) => String::new(),
                None => " [ORPHAN: not in .conductr]".to_string(),
            }
        } else {
            String::new()
        };

        lines.push(format!("  task:      {}", sched.task));
        lines.push(format!("  mechanism: {}", sched.mechanism));
        lines.push(format!("  cadence:   {}{drift_tag}", sched.cadence));
        lines.push(format!("  installed: {}", sched.installed_at));
        if let Some(label) = &sched.label {
            lines.push(format!("  label:     {label}"));
        }
        if let Some(plist) = &sched.plist {
            lines.push(format!("  plist:     {plist}"));
        }
    }

    if let Some(cfg) = &cfg {
        for task in cfg.cadence.keys() {
            if !local.schedule.iter().any(|s| &s.task == task) {
                lines.push(String::new());
                lines.push(format!(
                    "  [MISSING: '{task}' declared in .conductr but not installed \
                     — run `conductr cadence sync`]"
                ));
            }
        }
    }

    Ok(lines.join("\n"))
}

/// Render the cadence schedule as a 24-hour staff with a `↓ HH:MM UTC` marker
/// at the column for the current (or `--at`) UTC time.
///
/// Resolution: 1 column = 1 sixteenth note = 1 h (at q = 4 h, 6/4).
/// Bars: 6 bar sections of 4 h each (one quarter-note beat per visual bar).
pub fn show(repo_path: &Path, at: Option<chrono::DateTime<Utc>>) -> Result<String> {
    let cfg = read_config(repo_path)?;
    let now = at.unwrap_or_else(Utc::now);
    Ok(render_show(&cfg.cadence, now))
}

/// Render every conductr cron entry from the active crontab as a 24-hour staff.
/// Rows are labelled `<tag>-<task>` and ordered alphabetically.
pub fn show_all(at: Option<chrono::DateTime<Utc>>) -> Result<String> {
    let crontab = read_crontab()?;
    let entries = parse_all_cron_entries(&crontab);
    if entries.is_empty() {
        return Ok("no conductr cron entries found in crontab".to_string());
    }
    let now = at.unwrap_or_else(Utc::now);
    Ok(render_show(&entries, now))
}

/// Parse all `# conductr-cron: <label>` blocks from a crontab string.
/// Each block is a marker line followed by the cron command line.
/// Returns a BTreeMap of label → cron_expression (alphabetically ordered).
fn parse_all_cron_entries(crontab: &str) -> BTreeMap<String, String> {
    let mut result = BTreeMap::new();
    let mut lines = crontab.lines();
    while let Some(line) = lines.next() {
        if let Some(label) = line.strip_prefix(MARKER_PREFIX) {
            let label = label.trim().to_string();
            if let Some(next) = lines.next() {
                let fields: Vec<&str> = next.split_whitespace().take(5).collect();
                if fields.len() == 5 {
                    result.insert(label, fields.join(" "));
                }
            }
        }
    }
    result
}

/// Pure rendering of the cadence staff (no I/O). Exposed for tests.
fn render_show(cadence: &BTreeMap<String, String>, now: chrono::DateTime<Utc>) -> String {
    const HOURS_PER_BAR: usize = 4;
    const NUM_BARS: usize = 6; // 6/4 time
    const CELL_WIDTH: usize = 3; // chars per hour column

    let tasks: Vec<(&str, Vec<bool>)> = cadence
        .iter()
        .map(|(name, cron)| (name.as_str(), firing_hours(cron)))
        .collect();

    let label_width = tasks.iter().map(|(n, _)| n.len()).max().unwrap_or(0) + 2;

    // Round to nearest sixteenth-note column (= nearest hour).
    let total_minutes = now.hour() * 60 + now.minute();
    let current_col = ((total_minutes + 30) / 60 % 24) as usize;

    let bar_horiz = "═".repeat(HOURS_PER_BAR * CELL_WIDTH);
    let hline = |left: char, mid: char, right: char| -> String {
        let mut s = " ".repeat(label_width);
        s.push(left);
        for i in 0..NUM_BARS {
            s.push_str(&bar_horiz);
            s.push(if i + 1 < NUM_BARS { mid } else { right });
        }
        s
    };

    let top_line = hline('╔', '╦', '╗');
    let mid_line = hline('╠', '╬', '╣');
    let bot_line = hline('╚', '╩', '╝');

    // Position of ↓ within the output line (0-based character index).
    // Each bar section in the task rows: ║ + HOURS_PER_BAR×CELL_WIDTH chars.
    let bar_section_width = HOURS_PER_BAR * CELL_WIDTH + 1; // the leading ║ counts
    let bar_idx = current_col / HOURS_PER_BAR;
    let col_in_bar = current_col % HOURS_PER_BAR;
    let arrow_pos = label_width + bar_idx * bar_section_width + 1 + col_in_bar * CELL_WIDTH;

    let time_label = format!("↓ {:02}:{:02} UTC", now.hour(), now.minute());
    let marker_line = format!("{}{}", " ".repeat(arrow_pos), time_label);

    let mut out = String::new();
    out.push_str(&marker_line);
    out.push('\n');
    out.push_str(&top_line);
    out.push('\n');

    for (i, (name, fires)) in tasks.iter().enumerate() {
        if i > 0 {
            out.push_str(&mid_line);
            out.push('\n');
        }
        let mut row = format!("{:>label_width$}", name);
        for bar_i in 0..NUM_BARS {
            row.push('║');
            for col_i in 0..HOURS_PER_BAR {
                let hour = bar_i * HOURS_PER_BAR + col_i;
                if fires[hour] {
                    row.push_str("@' ");
                } else {
                    row.push_str("   ");
                }
            }
        }
        row.push('║');
        out.push_str(&row);
        out.push('\n');
    }

    out.push_str(&bot_line);
    out.push('\n');

    out
}

/// Return a 24-element bool vec indicating which hours a cron expression fires.
/// Only the hour field is examined (minute/day/month/weekday are ignored for
/// visual column resolution).
fn firing_hours(cron: &str) -> Vec<bool> {
    let mut fires = vec![false; 24];
    let parts: Vec<&str> = cron.split_whitespace().collect();
    if parts.len() != 5 {
        return fires;
    }
    let hours: Vec<u32> = match expand_cron_field(parts[1], 0, 23) {
        Ok(v) if v.is_empty() => (0..24).collect(), // wildcard = every hour
        Ok(v) => v,
        Err(_) => return fires,
    };
    for h in hours {
        if (h as usize) < 24 {
            fires[h as usize] = true;
        }
    }
    fires
}

pub fn remove(repo_path: &Path, dry_run: bool) -> Result<String> {
    let local_path = repo_path.join(LOCAL_FILE);

    if !local_path.exists() {
        return Ok("no .conductr-local found — nothing to remove".to_string());
    }

    let local_raw = std::fs::read_to_string(&local_path)
        .with_context(|| format!("reading {}", local_path.display()))?;
    let local: LocalFile = toml::from_str(&local_raw)
        .with_context(|| format!("parsing {}", local_path.display()))?;

    for sched in &local.schedule {
        match sched.mechanism.as_str() {
            "crontab" => {
                if let Some(marker) = &sched.marker {
                    remove_crontab_entry(marker, dry_run)?;
                } else {
                    eprintln!("warning: crontab entry for '{}' has no marker", sched.task);
                }
            }
            "launchd" => match (&sched.plist, &sched.label) {
                (Some(plist), Some(label)) => remove_launchd_entry(label, plist, dry_run)?,
                _ => eprintln!(
                    "warning: launchd entry for '{}' missing plist/label",
                    sched.task
                ),
            },
            other => eprintln!(
                "warning: unknown mechanism '{other}' for task '{}' — skipping",
                sched.task
            ),
        }
    }

    if dry_run {
        println!("[dry-run] would delete {}", local_path.display());
    } else {
        std::fs::remove_file(&local_path)
            .with_context(|| format!("deleting {}", local_path.display()))?;
        println!("deleted {}", local_path.display());
    }

    Ok(format!("removed {} task(s)", local.schedule.len()))
}

// ── Crontab sync ──────────────────────────────────────────────────────────────

fn sync_crontab(
    repo_path: &Path,
    cfg: &ConductrConfig,
    new_lines: &[(String, String)],
    log_dir: &str,
    dry_run: bool,
) -> Result<String> {
    let current = read_crontab().unwrap_or_default();
    let merged = merge(&current, new_lines, &cfg.project_tag);

    if dry_run {
        return Ok(format!(
            "would-write crontab ({} lines, {} new from .conductr):\n{merged}",
            merged.lines().count(),
            new_lines.len()
        ));
    }

    write_crontab(&merged)?;

    let now = Utc::now().to_rfc3339();
    let schedules: Vec<LocalSchedule> = cfg
        .cadence
        .iter()
        .map(|(task, cadence)| {
            let marker = format!("{MARKER_PREFIX} {}-{task}", cfg.project_tag);
            let command = task_command(task, &cfg.project_tag, cfg.repo.as_deref());
            let log_path = format!("{log_dir}/{task}.log");
            LocalSchedule {
                task: task.clone(),
                mechanism: "crontab".to_string(),
                command,
                cadence: cadence.clone(),
                installed_at: now.clone(),
                log_stdout: log_path.clone(),
                log_stderr: log_path,
                marker: Some(marker),
                label: None,
                plist: None,
            }
        })
        .collect();

    let local = LocalFile {
        machine: hostname(),
        generated_at: now,
        repo: cfg.repo.clone(),
        schedule: schedules,
    };
    write_local_file(repo_path, &local)?;

    Ok(format!(
        "synced {} cadence task(s) for project_tag '{}'",
        cfg.cadence.len(),
        cfg.project_tag
    ))
}

// ── Launchd sync ──────────────────────────────────────────────────────────────

fn sync_launchd(
    repo_path: &Path,
    cfg: &ConductrConfig,
    dry_run: bool,
) -> Result<String> {
    let home = std::env::var("HOME").context("HOME not set")?;
    let launch_agents_dir = format!("{home}/Library/LaunchAgents");
    let logs_base = format!("{home}/Library/Logs/conductr");

    let now = Utc::now().to_rfc3339();
    let mut schedules: Vec<LocalSchedule> = Vec::new();

    for (task, cron) in &cfg.cadence {
        let label = format!("com.conductr.{}.{task}", cfg.project_tag);
        let plist_path = format!("{launch_agents_dir}/{label}.plist");
        let log_stdout = format!("{logs_base}/{}-{task}.log", cfg.project_tag);
        let log_stderr = format!("{logs_base}/{}-{task}.err.log", cfg.project_tag);
        let command = task_command(task, &cfg.project_tag, cfg.repo.as_deref());
        let plist_content = generate_plist(&label, &command, cron, &log_stdout, &log_stderr)?;

        if dry_run {
            println!("[dry-run] would write {plist_path}");
            println!("[dry-run] plist content:\n{plist_content}");
            println!("[dry-run] would run: launchctl load -w {plist_path}");
            continue;
        }

        std::fs::create_dir_all(&logs_base)
            .with_context(|| format!("creating log directory {logs_base}"))?;

        std::fs::write(&plist_path, &plist_content)
            .with_context(|| format!("writing plist {plist_path}"))?;

        // Unload any previous version (ignore errors — may not be loaded yet).
        let _ = Command::new("launchctl")
            .args(["unload", "-w", &plist_path])
            .output();

        let out = Command::new("launchctl")
            .args(["load", "-w", &plist_path])
            .output()
            .context("running launchctl load")?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            anyhow::bail!("launchctl load failed for {label}: {stderr}");
        }

        println!("installed LaunchAgent: {label}");

        schedules.push(LocalSchedule {
            task: task.clone(),
            mechanism: "launchd".to_string(),
            command,
            cadence: cron.clone(),
            installed_at: now.clone(),
            log_stdout,
            log_stderr,
            marker: None,
            label: Some(label),
            plist: Some(plist_path),
        });
    }

    if dry_run {
        return Ok(format!(
            "would install {} LaunchAgent(s) for project_tag '{}'",
            cfg.cadence.len(),
            cfg.project_tag
        ));
    }

    let local = LocalFile {
        machine: hostname(),
        generated_at: now,
        repo: cfg.repo.clone(),
        schedule: schedules,
    };
    write_local_file(repo_path, &local)?;

    Ok(format!(
        "installed {} LaunchAgent(s) for project_tag '{}'",
        cfg.cadence.len(),
        cfg.project_tag
    ))
}

// ── Removal helpers ───────────────────────────────────────────────────────────

fn remove_crontab_entry(marker: &str, dry_run: bool) -> Result<()> {
    let current = read_crontab().unwrap_or_default();
    let mut out: Vec<String> = Vec::new();
    let mut skip_next = false;
    for line in current.lines() {
        if skip_next {
            skip_next = false;
            continue;
        }
        if line == marker {
            skip_next = true;
            continue;
        }
        out.push(line.to_string());
    }
    let content = out.join("\n") + "\n";

    if dry_run {
        println!("[dry-run] would remove crontab entry: {marker}");
    } else {
        write_crontab(&content)?;
        println!("removed crontab entry: {marker}");
    }
    Ok(())
}

fn remove_launchd_entry(label: &str, plist: &str, dry_run: bool) -> Result<()> {
    if dry_run {
        println!("[dry-run] would run: launchctl unload -w {plist}");
        println!("[dry-run] would delete: {plist}");
    } else {
        let _ = Command::new("launchctl")
            .args(["unload", "-w", plist])
            .output();
        let plist_path = Path::new(plist);
        if plist_path.exists() {
            std::fs::remove_file(plist_path)
                .with_context(|| format!("deleting plist {plist}"))?;
        }
        println!("unloaded and removed LaunchAgent: {label}");
    }
    Ok(())
}

// ── Plist generation ──────────────────────────────────────────────────────────

fn generate_plist(
    label: &str,
    command: &str,
    cron: &str,
    log_stdout: &str,
    log_stderr: &str,
) -> Result<String> {
    let intervals = cron_to_start_calendar_intervals(cron)?;

    let schedule_xml = if intervals.is_empty() {
        // All fields are wildcards (e.g. "* * * * *") — fire every 60 s.
        "\t<key>StartInterval</key>\n\t<integer>60</integer>\n".to_string()
    } else {
        let entries: String = intervals
            .iter()
            .map(|entry| {
                let mut xml = "\t\t<dict>\n".to_string();
                for (key, val) in entry {
                    xml.push_str(&format!(
                        "\t\t\t<key>{key}</key>\n\t\t\t<integer>{val}</integer>\n"
                    ));
                }
                xml.push_str("\t\t</dict>\n");
                xml
            })
            .collect();
        format!("\t<key>StartCalendarInterval</key>\n\t<array>\n{entries}\t</array>\n")
    };

    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>{label}</string>
	<key>ProgramArguments</key>
	<array>
		<string>/bin/bash</string>
		<string>-lc</string>
		<string>{command}</string>
	</array>
{schedule_xml}	<key>StandardOutPath</key>
	<string>{log_stdout}</string>
	<key>StandardErrorPath</key>
	<string>{log_stderr}</string>
	<key>RunAtLoad</key>
	<false/>
</dict>
</plist>
"#
    ))
}

/// Convert a 5-field cron expression to `StartCalendarInterval` dict entries.
/// Returns an empty vec when all fields are wildcards (caller uses `StartInterval`).
fn cron_to_start_calendar_intervals(cron: &str) -> Result<Vec<BTreeMap<String, i64>>> {
    let parts: Vec<&str> = cron.split_whitespace().collect();
    if parts.len() != 5 {
        anyhow::bail!("expected 5-field cron expression, got: {cron:?}");
    }

    let field_specs = [
        ("Minute", parts[0], 0u32, 59u32),
        ("Hour", parts[1], 0, 23),
        ("Day", parts[2], 1, 31),
        ("Month", parts[3], 1, 12),
        ("Weekday", parts[4], 0, 6),
    ];

    // Start with one empty entry and expand each non-wildcard field.
    let mut entries: Vec<BTreeMap<String, i64>> = vec![BTreeMap::new()];
    for (key, field, min, max) in &field_specs {
        let vals = expand_cron_field(field, *min, *max)?;
        if vals.is_empty() {
            continue; // wildcard — omit key, launchd fires on any value
        }
        let mut new_entries = Vec::with_capacity(entries.len() * vals.len());
        for entry in &entries {
            for &val in &vals {
                let mut e = entry.clone();
                e.insert(key.to_string(), val as i64);
                new_entries.push(e);
            }
        }
        entries = new_entries;
    }

    if entries.len() == 1 && entries[0].is_empty() {
        return Ok(Vec::new()); // all wildcards
    }
    Ok(entries)
}

/// Returns an empty vec for `*`. Otherwise expands comma-lists, ranges, and steps.
fn expand_cron_field(s: &str, min: u32, max: u32) -> Result<Vec<u32>> {
    if s == "*" {
        return Ok(Vec::new());
    }
    let mut values = Vec::new();
    for part in s.split(',') {
        if let Some(step_s) = part.strip_prefix("*/") {
            let step: u32 = step_s
                .parse()
                .with_context(|| format!("invalid step in '{part}'"))?;
            if step == 0 {
                anyhow::bail!("step cannot be 0 in '{part}'");
            }
            values.extend((min..=max).step_by(step as usize));
        } else if let Some((start_s, end_s)) = part.split_once('-') {
            let start: u32 = start_s
                .parse()
                .with_context(|| format!("invalid range start in '{part}'"))?;
            let end: u32 = end_s
                .parse()
                .with_context(|| format!("invalid range end in '{part}'"))?;
            values.extend(start..=end);
        } else {
            values.push(
                part.parse()
                    .with_context(|| format!("invalid cron value '{part}'"))?,
            );
        }
    }
    Ok(values)
}

// ── Shared helpers ────────────────────────────────────────────────────────────

fn read_config(repo_path: &Path) -> Result<ConductrConfig> {
    let path = repo_path.join(".conductr");
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("reading {}", path.display()))?;
    let cfg: ConductrConfig = toml::from_str(&raw)
        .with_context(|| format!("parsing {}", path.display()))?;
    if cfg.cadence.is_empty() {
        return Err(anyhow!(
            "{}: [cadence] table is missing or empty",
            path.display()
        ));
    }
    Ok(cfg)
}

fn log_dir_default() -> Result<String> {
    let home = std::env::var("HOME").context("HOME not set")?;
    Ok(format!("{home}/.local/share/conductr"))
}

fn generate_lines(cfg: &ConductrConfig, log_dir: &str) -> Vec<(String, String)> {
    cfg.cadence
        .iter()
        .map(|(task, schedule)| {
            let marker = format!("{MARKER_PREFIX} {}-{task}", cfg.project_tag);
            let inner = task_command(task, &cfg.project_tag, cfg.repo.as_deref());
            let line =
                format!("{schedule} bash -lc '{inner}' >> {log_dir}/{task}.log 2>&1");
            (marker, line)
        })
        .collect()
}

/// `orchestrate` is an alias for `conductr begin` (the cron-friendly entrypoint from #21).
fn task_command(task: &str, project_tag: &str, repo: Option<&str>) -> String {
    match task {
        "orchestrate" => match repo {
            Some(r) => format!("conductr begin --tag {project_tag} --repo {r}"),
            None => format!("conductr begin --tag {project_tag}"),
        },
        other => format!("conductr {other} --tag {project_tag}"),
    }
}

/// Strip any existing `# conductr-cron: <project_tag>-...` block (marker +
/// next line) and append the new ones.
fn merge(current: &str, new: &[(String, String)], project_tag: &str) -> String {
    let our_prefix = format!("{MARKER_PREFIX} {project_tag}-");
    let mut out: Vec<String> = Vec::new();
    let mut skip_next = false;
    for line in current.lines() {
        if skip_next {
            skip_next = false;
            continue;
        }
        if line.starts_with(&our_prefix) {
            skip_next = true;
            continue;
        }
        out.push(line.to_string());
    }
    for (marker, line) in new {
        out.push(marker.clone());
        out.push(line.clone());
    }
    let mut joined = out.join("\n");
    joined.push('\n');
    joined
}

fn read_crontab() -> Result<String> {
    let out = Command::new("crontab").arg("-l").output();
    match out {
        Ok(o) if o.status.success() => Ok(String::from_utf8_lossy(&o.stdout).into_owned()),
        Ok(_) => Ok(String::new()), // empty crontab returns non-zero
        Err(e) => Err(anyhow!("running `crontab -l`: {e}")),
    }
}

fn write_crontab(contents: &str) -> Result<()> {
    let mut child = Command::new("crontab")
        .arg("-")
        .stdin(Stdio::piped())
        .spawn()
        .context("spawning `crontab -`")?;
    use std::io::Write;
    child
        .stdin
        .as_mut()
        .ok_or_else(|| anyhow!("crontab stdin closed"))?
        .write_all(contents.as_bytes())
        .context("writing to crontab stdin")?;
    let status = child.wait().context("waiting on crontab")?;
    if !status.success() {
        return Err(anyhow!("crontab - exited with status {status}"));
    }
    Ok(())
}

fn hostname() -> String {
    Command::new("hostname")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn write_local_file(repo_path: &Path, local: &LocalFile) -> Result<()> {
    let path = repo_path.join(LOCAL_FILE);
    let content = toml::to_string(local).context("serializing .conductr-local")?;
    std::fs::write(&path, content)
        .with_context(|| format!("writing {}", path.display()))?;
    println!("wrote {}", path.display());
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(tag: &str, repo: Option<&str>, entries: &[(&str, &str)]) -> ConductrConfig {
        ConductrConfig {
            project_tag: tag.to_string(),
            repo: repo.map(str::to_string),
            cadence: entries
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    #[test]
    fn orchestrate_aliases_to_begin_with_repo() {
        let lines = generate_lines(
            &cfg("conductr", Some("Luan-vP/conductr"), &[("orchestrate", "0 */2 * * *")]),
            "/tmp/log",
        );
        assert_eq!(lines.len(), 1);
        let (marker, line) = &lines[0];
        assert_eq!(marker, "# conductr-cron: conductr-orchestrate");
        assert!(line.contains("conductr begin --tag conductr --repo Luan-vP/conductr"));
        assert!(line.starts_with("0 */2 * * *"));
        assert!(line.ends_with(">> /tmp/log/orchestrate.log 2>&1"));
    }

    #[test]
    fn unknown_task_falls_back_to_generic_invocation() {
        let lines =
            generate_lines(&cfg("conductr", None, &[("custom", "*/15 * * * *")]), "/tmp/log");
        let (_, line) = &lines[0];
        assert!(line.contains("conductr custom --tag conductr"));
    }

    #[test]
    fn merge_replaces_existing_marker_block() {
        let current = "\
# unrelated comment
0 0 * * * /usr/bin/some-other-job
# conductr-cron: conductr-orchestrate
0 */4 * * * conductr begin --tag conductr --repo old/repo >> /tmp/log/orchestrate.log 2>&1
";
        let new = vec![(
            "# conductr-cron: conductr-orchestrate".to_string(),
            "0 */2 * * * bash -lc 'conductr begin --tag conductr --repo Luan-vP/conductr' >> /tmp/log/orchestrate.log 2>&1"
                .to_string(),
        )];
        let merged = merge(current, &new, "conductr");
        assert!(merged.contains("/usr/bin/some-other-job"));
        assert!(merged.contains("0 */2 * * *"));
        assert!(!merged.contains("0 */4 * * *"));
        assert!(!merged.contains("old/repo"));
    }

    #[test]
    fn merge_preserves_other_marker_blocks() {
        let current = "\
# conductr-cron: otherproject-something
0 1 * * * other-thing
# conductr-cron: conductr-orchestrate
0 */4 * * * conductr begin --tag conductr >> /tmp/log/orchestrate.log 2>&1
";
        let new = vec![(
            "# conductr-cron: conductr-orchestrate".to_string(),
            "0 */2 * * * bash -lc 'conductr begin --tag conductr' >> /tmp/log/orchestrate.log 2>&1"
                .to_string(),
        )];
        let merged = merge(current, &new, "conductr");
        assert!(merged.contains("# conductr-cron: otherproject-something"));
        assert!(merged.contains("other-thing"));
        assert!(merged.contains("0 */2 * * *"));
        assert!(!merged.contains("0 */4 * * *"));
    }

    #[test]
    fn merge_into_empty_crontab() {
        let new = vec![(
            "# conductr-cron: conductr-orchestrate".to_string(),
            "0 */2 * * * bash -lc 'conductr begin' >> /tmp/log/orchestrate.log 2>&1".to_string(),
        )];
        let merged = merge("", &new, "conductr");
        assert!(merged.contains("# conductr-cron: conductr-orchestrate"));
        assert!(merged.ends_with('\n'));
    }

    // ── cron_to_start_calendar_intervals ─────────────────────────────────────

    #[test]
    fn wildcard_cron_returns_empty() {
        let result = cron_to_start_calendar_intervals("* * * * *").unwrap();
        assert!(result.is_empty(), "all wildcards should return empty vec");
    }

    #[test]
    fn simple_hourly_cron() {
        let result = cron_to_start_calendar_intervals("0 * * * *").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].get("Minute"), Some(&0i64));
        assert!(!result[0].contains_key("Hour"));
    }

    #[test]
    fn three_hour_interval_cron() {
        // "0 0,8,16 * * *" → 3 entries
        let result = cron_to_start_calendar_intervals("0 0,8,16 * * *").unwrap();
        assert_eq!(result.len(), 3);
        let hours: Vec<i64> = result.iter().map(|e| *e.get("Hour").unwrap()).collect();
        assert!(hours.contains(&0));
        assert!(hours.contains(&8));
        assert!(hours.contains(&16));
        for e in &result {
            assert_eq!(e.get("Minute"), Some(&0i64));
        }
    }

    #[test]
    fn step_expression_expands() {
        // "0 */2 * * *" → 12 entries (hours 0,2,4,...,22)
        let result = cron_to_start_calendar_intervals("0 */2 * * *").unwrap();
        assert_eq!(result.len(), 12);
    }

    #[test]
    fn range_expression_expands() {
        // "0 9-11 * * *" → 3 entries
        let result = cron_to_start_calendar_intervals("0 9-11 * * *").unwrap();
        assert_eq!(result.len(), 3);
        let hours: Vec<i64> = result.iter().map(|e| *e.get("Hour").unwrap()).collect();
        assert!(hours.contains(&9));
        assert!(hours.contains(&10));
        assert!(hours.contains(&11));
    }

    #[test]
    fn multiple_non_wildcard_fields_cartesian() {
        // "0,30 9,17 * * *" → 4 entries (minute x hour)
        let result = cron_to_start_calendar_intervals("0,30 9,17 * * *").unwrap();
        assert_eq!(result.len(), 4);
    }

    // ── expand_cron_field ─────────────────────────────────────────────────────

    #[test]
    fn expand_wildcard_returns_empty() {
        assert!(expand_cron_field("*", 0, 59).unwrap().is_empty());
    }

    #[test]
    fn expand_single_value() {
        assert_eq!(expand_cron_field("5", 0, 59).unwrap(), vec![5]);
    }

    #[test]
    fn expand_comma_list() {
        assert_eq!(expand_cron_field("0,30", 0, 59).unwrap(), vec![0, 30]);
    }

    #[test]
    fn expand_step() {
        assert_eq!(expand_cron_field("*/15", 0, 59).unwrap(), vec![0, 15, 30, 45]);
    }

    #[test]
    fn expand_range() {
        assert_eq!(expand_cron_field("1-3", 0, 59).unwrap(), vec![1, 2, 3]);
    }

    // ── plist generation ──────────────────────────────────────────────────────

    #[test]
    fn plist_contains_label_and_command() {
        let plist = generate_plist(
            "com.test.job",
            "conductr begin --tag test",
            "0 8 * * *",
            "/tmp/out.log",
            "/tmp/err.log",
        )
        .unwrap();
        assert!(plist.contains("<string>com.test.job</string>"));
        assert!(plist.contains("<string>conductr begin --tag test</string>"));
        assert!(plist.contains("StartCalendarInterval"));
        assert!(plist.contains("<integer>8</integer>")); // Hour
        assert!(plist.contains("<integer>0</integer>")); // Minute
        assert!(plist.contains("<string>/tmp/out.log</string>"));
        assert!(plist.contains("<string>/tmp/err.log</string>"));
    }

    #[test]
    fn plist_uses_start_interval_for_all_wildcards() {
        let plist = generate_plist(
            "com.test.every-minute",
            "conductr begin --tag test",
            "* * * * *",
            "/tmp/out.log",
            "/tmp/err.log",
        )
        .unwrap();
        assert!(plist.contains("StartInterval"));
        assert!(!plist.contains("StartCalendarInterval"));
    }

    // ── firing_hours ─────────────────────────────────────────────────────────

    #[test]
    fn firing_hours_every_two_hours() {
        let fires = firing_hours("0 */2 * * *");
        assert_eq!(fires.len(), 24);
        for h in 0..24usize {
            assert_eq!(fires[h], h % 2 == 0, "hour {h}");
        }
    }

    #[test]
    fn firing_hours_wildcard_fires_all() {
        let fires = firing_hours("* * * * *");
        assert!(fires.iter().all(|&f| f));
    }

    #[test]
    fn firing_hours_specific_hour() {
        let fires = firing_hours("0 8 * * *");
        assert!(fires[8]);
        let count = fires.iter().filter(|&&f| f).count();
        assert_eq!(count, 1);
    }

    // ── parse_all_cron_entries ───────────────────────────────────────────────

    #[test]
    fn parse_all_returns_empty_for_empty_crontab() {
        assert!(parse_all_cron_entries("").is_empty());
    }

    #[test]
    fn parse_all_extracts_single_entry() {
        let crontab = "\
# conductr-cron: conductr-orchestrate
0 */2 * * * bash -lc 'conductr begin' >> /tmp/log 2>&1
";
        let entries = parse_all_cron_entries(crontab);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries.get("conductr-orchestrate").map(String::as_str), Some("0 */2 * * *"));
    }

    #[test]
    fn parse_all_extracts_multiple_tags_alphabetically() {
        let crontab = "\
# conductr-cron: meta-model-analyzer-orchestrate
0 1 * * * bash -lc 'conductr begin --tag meta' >> /tmp/log 2>&1
# conductr-cron: flipbook-orchestrate
0 */4 * * * bash -lc 'conductr begin --tag flipbook' >> /tmp/log 2>&1
# conductr-cron: flipbook-idle
*/30 * * * * bash -lc 'conductr idle --tag flipbook' >> /tmp/log 2>&1
";
        let entries = parse_all_cron_entries(crontab);
        assert_eq!(entries.len(), 3);
        // BTreeMap keys are sorted alphabetically
        let keys: Vec<&str> = entries.keys().map(String::as_str).collect();
        assert_eq!(keys, vec!["flipbook-idle", "flipbook-orchestrate", "meta-model-analyzer-orchestrate"]);
        assert_eq!(entries.get("flipbook-orchestrate").map(String::as_str), Some("0 */4 * * *"));
        assert_eq!(entries.get("flipbook-idle").map(String::as_str), Some("*/30 * * * *"));
    }

    #[test]
    fn parse_all_ignores_non_conductr_lines() {
        let crontab = "\
# unrelated comment
0 0 * * * /usr/bin/some-job
# conductr-cron: conductr-orchestrate
0 */2 * * * bash -lc 'conductr begin' >> /tmp/log 2>&1
";
        let entries = parse_all_cron_entries(crontab);
        assert_eq!(entries.len(), 1);
        assert!(entries.contains_key("conductr-orchestrate"));
    }

    #[test]
    fn parse_all_skips_marker_with_no_following_line() {
        let crontab = "# conductr-cron: orphan-task";
        let entries = parse_all_cron_entries(crontab);
        assert!(entries.is_empty());
    }

    // ── render_show (column-position maths) ──────────────────────────────────

    fn cadence_map(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
        entries.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    fn arrow_col(output: &str) -> usize {
        output.lines().next().unwrap_or("").chars().take_while(|&c| c == ' ').count()
    }

    #[test]
    fn show_marker_at_14_23() {
        use chrono::TimeZone;
        let cadence = cadence_map(&[("orchestrate", "0 */2 * * *")]);
        let at = chrono::Utc.with_ymd_and_hms(2026, 5, 10, 14, 23, 0).unwrap();
        let output = render_show(&cadence, at);

        // First line must contain "↓ 14:23 UTC".
        let first = output.lines().next().unwrap();
        assert!(first.contains("↓ 14:23 UTC"), "got: {first:?}");

        // Column 14: bar_idx=3, col_in_bar=2.
        // label_width = "orchestrate".len() + 2 = 13
        // bar_section_width = 4*3+1 = 13
        // arrow_pos = 13 + 3*13 + 1 + 2*3 = 59
        let label_width = "orchestrate".len() + 2;
        let bsw = 4 * 3 + 1;
        let expected = label_width + 3 * bsw + 1 + 2 * 3; // 59
        assert_eq!(arrow_col(&output), expected, "↓ not at expected column");

        // Task row: first bar starts with ║@'  (hour 0 fires for */2)
        let task_line = output.lines().find(|l| l.contains("orchestrate")).unwrap();
        let staff = &task_line[label_width..];
        assert!(staff.starts_with("║@' "), "got: {staff:?}");
    }

    #[test]
    fn show_marker_rounds_to_nearest_hour() {
        use chrono::TimeZone;
        let cadence = cadence_map(&[("orchestrate", "0 */4 * * *")]);
        // 13:45 → (825+30)/60 = 14 → column 14 (same bar 3, col 2 as 14:xx)
        let at = chrono::Utc.with_ymd_and_hms(2026, 5, 10, 13, 45, 0).unwrap();
        let output = render_show(&cadence, at);
        let first = output.lines().next().unwrap();
        assert!(first.contains("↓ 13:45 UTC"), "got: {first:?}");
        let label_width = "orchestrate".len() + 2;
        let bsw = 4 * 3 + 1;
        let expected = label_width + 3 * bsw + 1 + 2 * 3; // col 14
        assert_eq!(arrow_col(&output), expected);
    }

    #[test]
    fn show_multi_task_has_mid_barlines() {
        use chrono::TimeZone;
        let cadence = cadence_map(&[
            ("orchestrate", "0 */2 * * *"),
            ("sync", "0 8 * * *"),
        ]);
        let at = chrono::Utc.with_ymd_and_hms(2026, 5, 10, 0, 0, 0).unwrap();
        let output = render_show(&cadence, at);
        // Should contain mid-barline (╠) between the two task rows.
        assert!(output.contains('╠'), "missing mid-barline");
        assert!(output.contains("orchestrate"));
        assert!(output.contains("sync"));
    }
}
