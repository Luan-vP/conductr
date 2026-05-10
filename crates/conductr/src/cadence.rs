//! Sync the host crontab from `.conductr [cadence]`.
//!
//! Each cadence entry becomes one crontab line, prefixed with a marker
//! comment so re-syncs replace the old lines instead of duplicating them:
//!
//!     # conductr-cron: <project_tag>-<task>
//!     <cron-expr> bash -lc 'conductr <command>' >> <log-path> 2>&1
//!
//! Lines without a matching marker are left untouched.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;

const MARKER_PREFIX: &str = "# conductr-cron:";

#[derive(Debug, Deserialize)]
struct ConductrConfig {
    project_tag: String,
    repo: Option<String>,
    #[serde(default)]
    cadence: BTreeMap<String, String>,
}

pub fn sync(repo_path: &Path, dry_run: bool) -> Result<String> {
    let cfg = read_config(repo_path)?;
    let log_dir = log_dir_default()?;
    let new_lines = generate_lines(&cfg, &log_dir);
    let current = read_crontab().unwrap_or_default();
    let merged = merge(&current, &new_lines, &cfg.project_tag);

    if dry_run {
        return Ok(format!(
            "would-write crontab ({} lines, {} new from .conductr):\n{merged}",
            merged.lines().count(),
            new_lines.len()
        ));
    }

    write_crontab(&merged)?;
    Ok(format!(
        "synced {} cadence task(s) for project_tag '{}'",
        cfg.cadence.len(),
        cfg.project_tag
    ))
}

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
            let line = format!(
                "{schedule} bash -lc '{inner}' >> {log_dir}/{task}.log 2>&1"
            );
            (marker, line)
        })
        .collect()
}

/// Map a cadence task name to the conductr CLI invocation that runs it.
///
/// `orchestrate` aliases to `conductr begin` (the cron-friendly wrapper from
/// #21). Any other key falls back to `conductr <task> --tag <project_tag>`.
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
            // Drop the marker and the cron line that follows it.
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
}
