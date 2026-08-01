//! `conductr self-update` — keep the installed `conductr` binary current.
//!
//! Fetches the conductr checkout, fast-forwards the branch it is on (the branch
//! the running binary was installed from — `develop` on the orchestrator), and
//! re-runs `cargo install --path crates/conductr --locked`. Rate-limited to at
//! most once per `interval_hours` (default 24) via a small state file, so it can
//! be wired into the idle pass — which runs several times a day — while only
//! rebuilding daily.
//!
//! Invoked as its own subcommand and as phase 0 of the idle pass (both the
//! `conductr idle` CLI and the `/idle` skill), so a long-lived pod converges on
//! the latest binary without manual intervention.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Duration, Utc};

use crate::registry;

/// Options for a self-update pass.
pub struct SelfUpdateOpts {
    /// Explicit conductr checkout. When `None`, resolved from the registry
    /// (the project whose repo is `…/conductr`) and finally the cwd.
    pub repo_path: Option<PathBuf>,
    /// Registry path override (`~/.conductr` when `None`).
    pub registry_path: Option<PathBuf>,
    /// Update even if the interval has not elapsed / HEAD did not move.
    pub force: bool,
    /// Print the plan without fetching, pulling, or installing.
    pub dry_run: bool,
    /// Minimum hours between real updates.
    pub interval_hours: i64,
}

impl Default for SelfUpdateOpts {
    fn default() -> Self {
        Self {
            repo_path: None,
            registry_path: None,
            force: false,
            dry_run: false,
            interval_hours: 24,
        }
    }
}

/// Whether an update is due: never updated, or `interval` has elapsed since
/// the last one. Pure — the scheduling policy, isolated for testing.
pub fn is_due(last: Option<DateTime<Utc>>, now: DateTime<Utc>, interval: Duration) -> bool {
    match last {
        None => true,
        Some(l) => now - l >= interval,
    }
}

/// Run one self-update pass. Never panics; returns an error only for a genuine
/// misconfiguration (no conductr checkout found). A failed fetch/pull is
/// reported and swallowed so the caller (idle) can continue.
pub fn run(opts: &SelfUpdateOpts) -> Result<()> {
    let repo = resolve_repo(opts.repo_path.clone(), opts.registry_path.as_deref())?;

    let now = Utc::now();
    let last = read_state();
    if !opts.force && !is_due(last, now, Duration::hours(opts.interval_hours)) {
        let ago = last.map(|l| human_duration(now - l)).unwrap_or_else(|| "never".into());
        println!(
            "self-update: skipped — last update {ago} ago (interval {}h)",
            opts.interval_hours
        );
        return Ok(());
    }

    let branch = git(&repo, &["rev-parse", "--abbrev-ref", "HEAD"])
        .context("determining current branch")?;

    if opts.dry_run {
        println!("self-update: → would fetch origin and pull --ff-only {branch}");
        println!(
            "self-update: → would run `cargo install --path crates/conductr --locked` in {}",
            repo.display()
        );
        return Ok(());
    }

    let before = git(&repo, &["rev-parse", "HEAD"]).context("reading HEAD")?;

    println!("self-update: fetching origin in {}…", repo.display());
    if let Err(e) = git(&repo, &["fetch", "--quiet", "origin"]) {
        println!("self-update: fetch failed ({e}); skipping");
        return Ok(());
    }
    if let Err(e) = git(&repo, &["pull", "--ff-only", "origin", &branch]) {
        println!("self-update: pull --ff-only {branch} failed ({e}); skipping install");
        return Ok(());
    }

    let after = git(&repo, &["rev-parse", "HEAD"]).context("reading HEAD after pull")?;
    if after == before && !opts.force {
        println!("self-update: already current at {} ({branch}); no rebuild", short(&after));
        write_state(now);
        return Ok(());
    }

    println!(
        "self-update: {} → {} ({branch}); installing…",
        short(&before),
        short(&after)
    );
    cargo_install(&repo)?;
    write_state(now);
    println!("self-update: installed {} ({branch})", short(&after));
    Ok(())
}

// ── repo resolution ─────────────────────────────────────────────────────────

fn is_conductr_checkout(p: &Path) -> bool {
    p.join("crates/conductr/Cargo.toml").is_file()
}

/// Locate the conductr checkout: explicit path → registry project whose repo
/// basename is `conductr` → current directory. Verifies the result actually
/// looks like a conductr checkout so we never `cargo install` the wrong tree.
fn resolve_repo(explicit: Option<PathBuf>, registry_path: Option<&Path>) -> Result<PathBuf> {
    if let Some(p) = explicit {
        if is_conductr_checkout(&p) {
            return Ok(p);
        }
        bail!("{} is not a conductr checkout (no crates/conductr/Cargo.toml)", p.display());
    }

    if let Ok(reg) = registry::load(registry_path) {
        if let Some(p) = reg
            .projects
            .iter()
            .find(|p| p.repo.rsplit('/').next() == Some("conductr"))
            .map(|p| p.path.clone())
        {
            if is_conductr_checkout(&p) {
                return Ok(p);
            }
        }
    }

    if let Ok(cwd) = std::env::current_dir() {
        if is_conductr_checkout(&cwd) {
            return Ok(cwd);
        }
    }

    bail!("could not locate the conductr checkout; pass --repo-path")
}

// ── git / cargo ──────────────────────────────────────────────────────────────

fn git(repo: &Path, args: &[&str]) -> Result<String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .with_context(|| format!("running git {}", args.join(" ")))?;
    if !out.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn cargo_install(repo: &Path) -> Result<()> {
    let status = std::process::Command::new("cargo")
        .args(["install", "--path", "crates/conductr", "--locked"])
        .current_dir(repo)
        .status()
        .context("running cargo install")?;
    if !status.success() {
        bail!("cargo install exited with {status}");
    }
    Ok(())
}

fn short(sha: &str) -> &str {
    &sha[..sha.len().min(8)]
}

// ── state ────────────────────────────────────────────────────────────────────

fn state_path() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."));
            home.join(".local/share")
        });
    base.join("conductr").join("self-update.state")
}

fn read_state() -> Option<DateTime<Utc>> {
    let s = std::fs::read_to_string(state_path()).ok()?;
    DateTime::parse_from_rfc3339(s.trim())
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

fn write_state(now: DateTime<Utc>) {
    let p = state_path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(&p, now.to_rfc3339()) {
        println!("self-update: warning — could not persist state to {}: {e}", p.display());
    }
}

fn human_duration(d: Duration) -> String {
    let mins = d.num_minutes().max(0);
    let h = mins / 60;
    let m = mins % 60;
    if h > 0 {
        format!("{h}h {m}m")
    } else {
        format!("{m}m")
    }
}

// ── tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn due_when_never_updated() {
        assert!(is_due(None, Utc::now(), Duration::hours(24)));
    }

    #[test]
    fn due_after_interval_elapsed() {
        let now = Utc::now();
        let last = now - Duration::hours(25);
        assert!(is_due(Some(last), now, Duration::hours(24)));
    }

    #[test]
    fn not_due_within_interval() {
        let now = Utc::now();
        let last = now - Duration::hours(3);
        assert!(!is_due(Some(last), now, Duration::hours(24)));
    }

    #[test]
    fn due_exactly_at_interval_boundary() {
        let now = Utc::now();
        let last = now - Duration::hours(24);
        assert!(is_due(Some(last), now, Duration::hours(24)));
    }

    #[test]
    fn resolve_rejects_non_conductr_explicit_path() {
        let tmp = tempfile::tempdir().unwrap();
        let err = resolve_repo(Some(tmp.path().to_path_buf()), None).unwrap_err();
        assert!(err.to_string().contains("not a conductr checkout"));
    }

    #[test]
    fn resolve_accepts_conductr_shaped_explicit_path() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("crates/conductr")).unwrap();
        std::fs::write(tmp.path().join("crates/conductr/Cargo.toml"), "[package]\n").unwrap();
        let got = resolve_repo(Some(tmp.path().to_path_buf()), None).unwrap();
        assert_eq!(got, tmp.path());
    }

    #[test]
    fn short_sha_truncates_to_eight() {
        assert_eq!(short("0123456789abcdef"), "01234567");
        assert_eq!(short("abc"), "abc");
    }

    #[test]
    fn human_duration_formats_hours_and_minutes() {
        assert_eq!(human_duration(Duration::minutes(0)), "0m");
        assert_eq!(human_duration(Duration::minutes(45)), "45m");
        assert_eq!(human_duration(Duration::minutes(150)), "2h 30m");
    }
}
