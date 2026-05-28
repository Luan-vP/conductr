//! `conductr idle` — architecture scan, module clippy scan, and issue filing.
//!
//! Pure helpers only — the CLI bootstraps a tmux+Claude session and sends
//! `/idle`; the skill drives the workflow from inside that session.

use std::collections::HashSet;
use std::path::Path;

use anyhow::Result;
use conductr_core::types::{Finding, FindingSeverity, TmuxSession};

/// Ordered list of use-case crates for round-robin scanning.
pub const USE_CASE_CRATES: &[&str] = &[
    "conductr-orchestrate",
    "conductr-pod",
    "conductr-tasks",
    "conductr-instance",
    "conductr-schedule",
    "conductr-mail",
    "conductr-setup",
];

/// Dependencies forbidden in `conductr-core` (Rule 4 — no I/O).
const IO_DENY_DEPS: &[&str] = &["reqwest", "hyper"];

// ── Stale-pane reconciliation ─────────────────────────────────────────────────

/// Names of `agent<n>` tmux sessions that are candidates for release during
/// the idle sweep.
///
/// A pane is stale when the number of active agent sessions exceeds
/// `in_flight_count` — the number of open issues that still carry the
/// `conductr:in-flight` label (i.e. implementation is still ongoing).
///
/// The highest-indexed slots are freed first (LIFO).  Call this from the idle
/// sweep to identify which sessions to `tmux kill-session` before the next
/// orchestrate pass picks them up as false slots.
pub fn stale_agent_panes(sessions: &[TmuxSession], in_flight_count: usize) -> Vec<String> {
    let agent_prefix = "agent";
    let mut agent_sessions: Vec<&TmuxSession> = sessions
        .iter()
        .filter(|s| {
            s.name
                .strip_prefix(agent_prefix)
                .and_then(|n| n.parse::<usize>().ok())
                .is_some()
        })
        .collect();

    // Descending order so we free the highest-indexed (most-recently-spawned) first.
    agent_sessions.sort_by(|a, b| b.name.cmp(&a.name));

    let active_count = agent_sessions.len();
    if active_count <= in_flight_count {
        return Vec::new();
    }
    agent_sessions[..active_count - in_flight_count]
        .iter()
        .map(|s| s.name.clone())
        .collect()
}

// ── Module round-robin ────────────────────────────────────────────────────────

/// Return the module to scan in this pass.
///
/// `last_module` is the module scanned in the previous pass (empty = first ever).
/// Advances one step; wraps at the end of the list.
pub fn pick_module(last_module: &str) -> &'static str {
    if last_module.is_empty() {
        return USE_CASE_CRATES[0];
    }
    if let Some(pos) = USE_CASE_CRATES.iter().position(|&c| c == last_module) {
        USE_CASE_CRATES[(pos + 1) % USE_CASE_CRATES.len()]
    } else {
        USE_CASE_CRATES[0]
    }
}

// ── Architecture rules ────────────────────────────────────────────────────────

/// Rule 1: use-case crates must not depend on `conductr-adapters`.
pub fn check_rule1(repo_path: &Path) -> Vec<Finding> {
    let mut findings = Vec::new();
    for crate_name in USE_CASE_CRATES {
        let cargo_path = repo_path.join("crates").join(crate_name).join("Cargo.toml");
        let Ok(content) = std::fs::read_to_string(&cargo_path) else { continue };
        let Ok(table) = content.parse::<toml::Value>() else { continue };
        if has_dep(&table, "conductr-adapters") {
            let fp = format!("arch/rule1/{crate_name}/conductr-adapters");
            findings.push(Finding {
                title: format!("arch: `{crate_name}` must not depend on `conductr-adapters`"),
                body: format!(
                    "## Finding\n\n\
                     Hexagonal rule 1 violated: use-case crate `{crate_name}` \
                     depends on `conductr-adapters`.\n\n\
                     Use-case crates must depend only on `conductr-core`.\n\n\
                     ## Acceptance criteria\n\n\
                     - [ ] Remove `conductr-adapters` from `crates/{crate_name}/Cargo.toml`.\n\
                     - [ ] `cargo check --workspace` passes.\n\n\
                     <!-- conductr-idle-fingerprint: {fp} -->"
                ),
                severity: FindingSeverity::Architecture,
                fingerprint: fp,
            });
        }
    }
    findings
}

/// Rule 3: `conductr-adapters` must not depend on any use-case crate.
pub fn check_rule3(repo_path: &Path) -> Vec<Finding> {
    let cargo_path = repo_path
        .join("crates")
        .join("conductr-adapters")
        .join("Cargo.toml");
    let Ok(content) = std::fs::read_to_string(&cargo_path) else {
        return vec![];
    };
    let Ok(table) = content.parse::<toml::Value>() else {
        return vec![];
    };
    let mut findings = Vec::new();
    for crate_name in USE_CASE_CRATES {
        if has_dep(&table, crate_name) {
            let fp = format!("arch/rule3/conductr-adapters/{crate_name}");
            findings.push(Finding {
                title: format!(
                    "arch: `conductr-adapters` must not depend on `{crate_name}`"
                ),
                body: format!(
                    "## Finding\n\n\
                     Hexagonal rule 3 violated: `conductr-adapters` depends on \
                     use-case crate `{crate_name}`.\n\n\
                     Adapters must only speak port traits from `conductr-core::ports`.\n\n\
                     ## Acceptance criteria\n\n\
                     - [ ] Remove `{crate_name}` from `crates/conductr-adapters/Cargo.toml`.\n\
                     - [ ] `cargo check --workspace` passes.\n\n\
                     <!-- conductr-idle-fingerprint: {fp} -->"
                ),
                severity: FindingSeverity::Architecture,
                fingerprint: fp,
            });
        }
    }
    findings
}

/// Rule 4: `conductr-core` must have no I/O dependencies.
pub fn check_rule4(repo_path: &Path) -> Vec<Finding> {
    let cargo_path = repo_path
        .join("crates")
        .join("conductr-core")
        .join("Cargo.toml");
    let Ok(content) = std::fs::read_to_string(&cargo_path) else {
        return vec![];
    };
    let Ok(table) = content.parse::<toml::Value>() else {
        return vec![];
    };
    let mut findings = Vec::new();
    for dep in IO_DENY_DEPS {
        if has_dep(&table, dep) {
            let fp = format!("arch/rule4/conductr-core/{dep}");
            findings.push(Finding {
                title: format!("arch: `conductr-core` must not depend on I/O crate `{dep}`"),
                body: format!(
                    "## Finding\n\n\
                     Hexagonal rule 4 violated: `conductr-core` has an I/O \
                     dependency on `{dep}`.\n\n\
                     The core crate must have no I/O \
                     (no tokio::process, reqwest, hyper, etc.).\n\n\
                     ## Acceptance criteria\n\n\
                     - [ ] Remove `{dep}` from `crates/conductr-core/Cargo.toml`.\n\
                     - [ ] `cargo check --workspace` passes.\n\n\
                     <!-- conductr-idle-fingerprint: {fp} -->"
                ),
                severity: FindingSeverity::Architecture,
                fingerprint: fp,
            });
        }
    }
    // tokio with `process` feature is also forbidden
    if has_dep_with_feature(&table, "tokio", "process") {
        let fp = "arch/rule4/conductr-core/tokio/process".to_string();
        findings.push(Finding {
            title: "arch: `conductr-core` must not use tokio `process` feature".to_string(),
            body: format!(
                "## Finding\n\n\
                 Hexagonal rule 4 violated: `conductr-core` uses `tokio` with \
                 the `process` feature, which enables subprocess I/O.\n\n\
                 ## Acceptance criteria\n\n\
                 - [ ] Remove the `process` feature from `tokio` in \
                 `crates/conductr-core/Cargo.toml`.\n\
                 - [ ] `cargo check --workspace` passes.\n\n\
                 <!-- conductr-idle-fingerprint: {fp} -->"
            ),
            severity: FindingSeverity::Architecture,
            fingerprint: fp,
        });
    }
    findings
}

fn has_dep(table: &toml::Value, dep_name: &str) -> bool {
    let sections = ["dependencies", "dev-dependencies", "build-dependencies"];
    for section in &sections {
        if let Some(deps) = table.get(section).and_then(|v| v.as_table()) {
            if deps.contains_key(dep_name) {
                return true;
            }
        }
    }
    false
}

fn has_dep_with_feature(table: &toml::Value, dep_name: &str, feature: &str) -> bool {
    let sections = ["dependencies", "dev-dependencies", "build-dependencies"];
    for section in &sections {
        if let Some(deps) = table.get(section).and_then(|v| v.as_table()) {
            if let Some(dep_val) = deps.get(dep_name) {
                if let Some(features) = dep_val.get("features").and_then(|v| v.as_array()) {
                    if features.iter().any(|f| f.as_str() == Some(feature)) {
                        return true;
                    }
                }
            }
        }
    }
    false
}

// ── Clippy integration ────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct ClippyMessage {
    reason: String,
    message: Option<ClippyDiagnostic>,
}

#[derive(serde::Deserialize)]
struct ClippyDiagnostic {
    level: String,
    message: String,
    code: Option<ClippyCode>,
    rendered: Option<String>,
    spans: Vec<ClippySpan>,
}

#[derive(serde::Deserialize)]
struct ClippyCode {
    code: String,
}

#[derive(serde::Deserialize)]
struct ClippySpan {
    file_name: String,
    line_start: u32,
    is_primary: bool,
}

pub async fn run_clippy(repo_path: &Path, crate_name: &str) -> Result<Vec<Finding>> {
    let output = tokio::process::Command::new("cargo")
        .args(["clippy", "-p", crate_name, "--message-format=json"])
        .current_dir(repo_path)
        .output()
        .await?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(parse_clippy_output(&stdout, crate_name))
}

pub fn parse_clippy_output(output: &str, crate_name: &str) -> Vec<Finding> {
    let mut findings = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for line in output.lines() {
        let Ok(msg) = serde_json::from_str::<ClippyMessage>(line) else { continue };
        if msg.reason != "compiler-message" {
            continue;
        }
        let Some(diag) = msg.message else { continue };
        if diag.level != "warning" {
            continue;
        }

        let primary = diag.spans.iter().find(|s| s.is_primary);
        let location = primary
            .map(|s| format!("{}:{}", s.file_name, s.line_start))
            .unwrap_or_else(|| "unknown".to_string());

        let lint = diag.code.as_ref().map(|c| c.code.as_str()).unwrap_or("warning");
        let fp = format!("clippy/{crate_name}/{lint}/{location}");
        if !seen.insert(fp.clone()) {
            continue;
        }

        let rendered = diag.rendered.as_deref().unwrap_or(&diag.message);
        let title = format!("quality: `{crate_name}` clippy warning in {location}");
        let body = format!(
            "## Finding\n\n\
             Clippy warning in `{crate_name}` at `{location}`.\n\n\
             ```\n{rendered}\n```\n\n\
             ## Acceptance criteria\n\n\
             - [ ] Fix the clippy warning at `{location}`.\n\
             - [ ] `cargo clippy --workspace -- -D warnings` passes.\n\n\
             <!-- conductr-idle-fingerprint: {fp} -->"
        );
        findings.push(Finding {
            title,
            body,
            severity: FindingSeverity::Quality,
            fingerprint: fp,
        });
    }
    findings
}

// ── Coverage scan ─────────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct LlvmCovReport {
    data: Vec<LlvmCovData>,
}

#[derive(serde::Deserialize)]
struct LlvmCovData {
    files: Vec<LlvmCovFile>,
}

#[derive(serde::Deserialize)]
struct LlvmCovFile {
    filename: String,
    segments: Vec<Vec<serde_json::Value>>,
    summary: LlvmCovSummary,
}

#[derive(serde::Deserialize)]
struct LlvmCovSummary {
    lines: LlvmCovLines,
}

#[derive(serde::Deserialize)]
struct LlvmCovLines {
    count: u64,
    covered: u64,
    percent: f64,
}

/// Run `cargo llvm-cov --json -p <crate>` and parse findings.
///
/// Returns `Ok(vec![])` (with a logged warning) when `cargo llvm-cov` is not
/// installed rather than propagating an error so the rest of the idle pass
/// continues unaffected.
pub async fn run_coverage_scan(
    repo_path: &Path,
    crate_name: &str,
    threshold: f32,
    exclude: &[String],
) -> Result<Vec<Finding>> {
    let result = tokio::process::Command::new("cargo")
        .args(["llvm-cov", "--json", "-p", crate_name])
        .current_dir(repo_path)
        .output()
        .await;

    let output = match result {
        Ok(o) => o,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            tracing::warn!("cargo-llvm-cov not installed; skipping coverage phase");
            return Ok(Vec::new());
        }
        Err(e) => return Err(e.into()),
    };

    // Exit 1 just means tests failed or nothing to measure — still parse JSON.
    let stdout = String::from_utf8_lossy(&output.stdout);
    if stdout.trim().is_empty() {
        tracing::warn!("cargo llvm-cov produced no output for {crate_name}");
        return Ok(Vec::new());
    }

    Ok(parse_coverage_output(&stdout, crate_name, threshold, exclude))
}

pub fn parse_coverage_output(
    json: &str,
    crate_name: &str,
    threshold: f32,
    exclude: &[String],
) -> Vec<Finding> {
    let report: LlvmCovReport = match serde_json::from_str(json) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("failed to parse llvm-cov JSON: {e}");
            return Vec::new();
        }
    };

    let src_prefix = format!("/{crate_name}/src/");
    let mut findings = Vec::new();

    for data in &report.data {
        for file in &data.files {
            // Only files under <crate>/src/
            let Some(rel_pos) = file.filename.find(&src_prefix) else { continue };
            let rel_path = &file.filename[rel_pos + 1..]; // e.g. "conductr-foo/src/lib.rs"

            // Strip to just "src/..." for glob matching
            let src_rel = &rel_path[crate_name.len() + 1..]; // "src/lib.rs"

            if glob_matches_any(src_rel, exclude) {
                continue;
            }

            let pct = file.summary.lines.percent as f32 / 100.0;
            if pct >= threshold {
                continue;
            }

            let covered_pct = (pct * 100.0).round() as u64;
            let threshold_pct = (threshold * 100.0).round() as u64;
            let missing = file.summary.lines.count.saturating_sub(file.summary.lines.covered);

            let ranges = uncovered_ranges(&file.segments);
            let ranges_text = if ranges.is_empty() {
                String::from("_(no detailed range data)_")
            } else {
                ranges
                    .iter()
                    .map(|(s, e)| {
                        if s == e {
                            format!("- line {s}")
                        } else {
                            format!("- lines {s}–{e}")
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            };

            let fp = format!("coverage/{crate_name}/{src_rel}");
            findings.push(Finding {
                title: format!(
                    "coverage: `{crate_name}/{src_rel}`: {covered_pct}% below threshold \
                     ({missing} uncovered lines)"
                ),
                body: format!(
                    "## Finding\n\n\
                     `{src_rel}` in `{crate_name}` has **{covered_pct}%** line coverage, \
                     below the {threshold_pct}% threshold.\n\n\
                     **Uncovered lines:** {missing} of {total}\n\n\
                     ### Top uncovered ranges\n\n\
                     {ranges_text}\n\n\
                     ## Acceptance criteria\n\n\
                     - [ ] Add tests so line coverage of `{src_rel}` reaches ≥ {threshold_pct}%.\n\
                     - [ ] `cargo llvm-cov -p {crate_name}` reports ≥ {threshold_pct}% for this file.\n\n\
                     <!-- conductr-idle-fingerprint: {fp} -->",
                    total = file.summary.lines.count,
                ),
                severity: FindingSeverity::Coverage,
                fingerprint: fp,
            });
        }
    }

    findings
}

/// Extract the top 5 uncovered line ranges from llvm-cov segment data.
///
/// Each segment is `[line, col, count, has_count, is_region_entry, is_gap_region]`.
/// A region is uncovered when `has_count == true` and `count == 0`.
fn uncovered_ranges(segments: &[Vec<serde_json::Value>]) -> Vec<(u64, u64)> {
    let mut ranges: Vec<(u64, u64)> = Vec::new();
    let mut uncov_start: Option<u64> = None;
    let mut prev_line: u64 = 0;

    for seg in segments {
        if seg.len() < 4 {
            continue;
        }
        let line = seg[0].as_u64().unwrap_or(0);
        let count = seg[2].as_u64().unwrap_or(0);
        let has_count = seg[3].as_bool().unwrap_or(false);
        let is_uncovered = has_count && count == 0;

        if is_uncovered && uncov_start.is_none() {
            uncov_start = Some(line);
        } else if !is_uncovered {
            if let Some(start) = uncov_start.take() {
                let end = if line > 0 { line - 1 } else { prev_line };
                if end >= start {
                    ranges.push((start, end));
                }
            }
        }
        prev_line = line;
    }

    if let Some(start) = uncov_start {
        ranges.push((start, prev_line));
    }

    // Largest ranges first, then take top 5, then re-sort by start line.
    ranges.sort_by(|a, b| (b.1 - b.0).cmp(&(a.1 - a.0)));
    ranges.truncate(5);
    ranges.sort_by_key(|r| r.0);
    ranges
}

/// Match a relative path against a list of simple glob patterns.
/// Supports `*` (any sequence of non-separator chars) and `**` (any path segment).
fn glob_matches_any(path: &str, patterns: &[String]) -> bool {
    patterns.iter().any(|p| glob_match(p, path))
}

fn glob_match(pattern: &str, path: &str) -> bool {
    // Split pattern and path on '/' and match segment by segment.
    // "**" matches zero or more path segments.
    let pat_parts: Vec<&str> = pattern.split('/').collect();
    let path_parts: Vec<&str> = path.split('/').collect();
    glob_match_parts(&pat_parts, &path_parts)
}

fn glob_match_parts(pat: &[&str], path: &[&str]) -> bool {
    match (pat.first(), path.first()) {
        (None, None) => true,
        (None, _) | (_, None) => {
            // Allow trailing ** to match zero remaining segments
            pat.first() == Some(&"**") && pat.len() == 1
        }
        (Some(&"**"), _) => {
            // ** matches zero or more segments
            glob_match_parts(&pat[1..], path)
                || glob_match_parts(pat, &path[1..])
        }
        (Some(p), Some(s)) => {
            segment_match(p, s) && glob_match_parts(&pat[1..], &path[1..])
        }
    }
}

fn segment_match(pattern: &str, segment: &str) -> bool {
    // Match a single path segment against a pattern containing '*' wildcards.
    let mut pat_chars = pattern.chars().peekable();
    let seg_chars: Vec<char> = segment.chars().collect();
    segment_match_inner(&mut pat_chars.collect::<Vec<_>>(), &seg_chars)
}

fn segment_match_inner(pat: &[char], seg: &[char]) -> bool {
    match (pat.first(), seg.first()) {
        (None, None) => true,
        (None, _) => false,
        (Some('*'), _) => {
            // '*' matches zero or more chars in this segment
            segment_match_inner(&pat[1..], seg)
                || (!seg.is_empty() && segment_match_inner(pat, &seg[1..]))
        }
        (_, None) => false,
        (Some(p), Some(s)) => p == s && segment_match_inner(&pat[1..], &seg[1..]),
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

pub(crate) fn severity_label(s: &FindingSeverity) -> &'static str {
    match s {
        FindingSeverity::Architecture => "architecture",
        FindingSeverity::Quality => "quality",
        FindingSeverity::Coverage => "coverage",
        FindingSeverity::Security => "security",
    }
}

/// True when `Cargo.toml` exists at the repo root (non-Rust repos lack this).
pub fn has_cargo_toml(repo_path: &Path) -> bool {
    repo_path.join("Cargo.toml").exists()
}

/// Check that `.claude/base.md` exists and has recognisable structure.
///
/// When the file is absent or has no `## ` headings, emits one `Architecture`
/// finding whose body embeds the hexagonal (ports & adapters) template as the
/// recommended starting point.  Authors who want a different pattern write
/// their own base.md; absent that, hex is the standing convention.
pub fn check_base_md(repo_path: &Path) -> Vec<Finding> {
    let base_path = repo_path.join(".claude").join("base.md");

    let fp_suffix = match std::fs::read_to_string(&base_path) {
        Err(_) => "missing",
        Ok(content) if !content.lines().any(|l| l.starts_with("## ")) => "unstructured",
        Ok(_) => return vec![],
    };

    let fp = format!("arch/base-md/{fp_suffix}");
    let detail = if fp_suffix == "missing" {
        "`.claude/base.md` was not found".to_string()
    } else {
        format!(
            "`{}` exists but has no `## ` section headings",
            base_path.display()
        )
    };

    vec![Finding {
        title: "arch: `.claude/base.md` is absent or unstructured".to_string(),
        body: format!(
            "## Finding\n\n\
             {detail}\n\n\
             This file is the architectural ground truth for the idle audit and the architect \
             skill. Without it, the LLM-driven architecture audit runs against a generic \
             baseline instead of this repo's declared rules.\n\n\
             ## Proposed hexagonal default\n\n\
             Create `.claude/base.md` with at minimum **Pattern**, **Arms**, and **Rules** \
             sections. For a hexagonal (ports & adapters) repo the template is:\n\n\
             ```markdown\n\
             ## Pattern\n\
             \n\
             Hexagonal (ports & adapters). Driving adapters (CLI, skills) call use-case\n\
             crates; those depend only on core (types + ports); concrete connectors live\n\
             in a single adapters crate.\n\
             \n\
             ## Arms\n\
             \n\
             List your use-case crates here, e.g.:\n\
             - `<name>-orchestrate`\n\
             - `<name>-tasks`\n\
             - `<name>-setup`\n\
             \n\
             ## Rules\n\
             \n\
             1. Use-case crates may not depend on the adapters crate.\n\
             2. The binary is the only place adapters are constructed and wired into use cases.\n\
             3. Adapters never depend on use-case crates.\n\
             4. Core has no I/O.\n\
             5. Mocks belong in the adapters crate, not in per-crate test modules.\n\
             6. One trait per port.\n\
             ```\n\
             \n\
             Override the **Pattern** section with your repo's actual architectural pattern; \
             the architect skill reads it at runtime and audits against whatever rules the \
             file declares. Absent a custom base.md, hexagonal is the standing default.\n\n\
             ## Acceptance criteria\n\n\
             - [ ] Create `.claude/base.md` with **Pattern**, **Arms**, and **Rules** sections.\n\
             - [ ] Re-run `conductr architect review` — the LLM audit now uses your rules.\n\n\
             <!-- conductr-idle-fingerprint: {fp} -->"
        ),
        severity: FindingSeverity::Architecture,
        fingerprint: fp,
    }]
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use conductr_core::types::TmuxSession;

    // ── pick_module ───────────────────────────────────────────────────────────

    #[test]
    fn pick_module_empty_starts_at_first() {
        assert_eq!(pick_module(""), USE_CASE_CRATES[0]);
    }

    #[test]
    fn pick_module_advances_from_first() {
        let next = pick_module(USE_CASE_CRATES[0]);
        assert_eq!(next, USE_CASE_CRATES[1]);
    }

    #[test]
    fn pick_module_wraps_from_last() {
        let next = pick_module(USE_CASE_CRATES[USE_CASE_CRATES.len() - 1]);
        assert_eq!(next, USE_CASE_CRATES[0]);
    }

    #[test]
    fn pick_module_unknown_resets_to_first() {
        assert_eq!(pick_module("unknown-crate"), USE_CASE_CRATES[0]);
    }

    // ── Architecture rules ────────────────────────────────────────────────────

    #[test]
    fn rule1_passes_when_no_adapter_dep() {
        let dir = tempdir_with_cargo_toml(
            r#"[package]
name = "conductr-orchestrate"
version = "0.1.0"

[dependencies]
conductr-core = { path = "../conductr-core" }
"#,
            "conductr-orchestrate",
        );
        let findings = check_rule1(dir.path());
        assert!(findings.is_empty(), "expected no findings, got: {findings:?}");
    }

    #[test]
    fn rule1_fails_when_adapter_dep_present() {
        let dir = tempdir_with_cargo_toml(
            r#"[package]
name = "conductr-orchestrate"
version = "0.1.0"

[dependencies]
conductr-adapters = { path = "../conductr-adapters" }
"#,
            "conductr-orchestrate",
        );
        let findings = check_rule1(dir.path());
        assert_eq!(findings.len(), 1);
        assert!(findings[0].title.contains("conductr-orchestrate"));
        assert!(findings[0].title.contains("conductr-adapters"));
    }

    #[test]
    fn rule3_passes_when_no_usecase_dep() {
        let dir = tempdir_with_cargo_toml(
            r#"[package]
name = "conductr-adapters"
version = "0.1.0"

[dependencies]
conductr-core = { path = "../conductr-core" }
"#,
            "conductr-adapters",
        );
        let findings = check_rule3(dir.path());
        assert!(findings.is_empty(), "expected no findings, got: {findings:?}");
    }

    #[test]
    fn rule3_fails_when_usecase_dep_present() {
        let dir = tempdir_with_cargo_toml(
            r#"[package]
name = "conductr-adapters"
version = "0.1.0"

[dependencies]
conductr-orchestrate = { path = "../conductr-orchestrate" }
"#,
            "conductr-adapters",
        );
        let findings = check_rule3(dir.path());
        assert_eq!(findings.len(), 1);
        assert!(findings[0].title.contains("conductr-orchestrate"));
    }

    #[test]
    fn rule4_passes_when_no_io_dep() {
        let dir = tempdir_with_cargo_toml(
            r#"[package]
name = "conductr-core"
version = "0.1.0"

[dependencies]
serde = { version = "1", features = ["derive"] }
"#,
            "conductr-core",
        );
        let findings = check_rule4(dir.path());
        assert!(findings.is_empty(), "expected no findings, got: {findings:?}");
    }

    #[test]
    fn rule4_fails_when_reqwest_present() {
        let dir = tempdir_with_cargo_toml(
            r#"[package]
name = "conductr-core"
version = "0.1.0"

[dependencies]
reqwest = "0.12"
"#,
            "conductr-core",
        );
        let findings = check_rule4(dir.path());
        assert_eq!(findings.len(), 1);
        assert!(findings[0].fingerprint.contains("reqwest"));
    }

    #[test]
    fn rule4_fails_when_tokio_process_feature() {
        let dir = tempdir_with_cargo_toml(
            r#"[package]
name = "conductr-core"
version = "0.1.0"

[dependencies]
tokio = { version = "1", features = ["process", "rt"] }
"#,
            "conductr-core",
        );
        let findings = check_rule4(dir.path());
        assert_eq!(findings.len(), 1);
        assert!(findings[0].fingerprint.contains("tokio/process"));
    }

    // ── Clippy output parsing ─────────────────────────────────────────────────

    #[test]
    fn parse_clippy_empty_output() {
        let findings = parse_clippy_output("", "my-crate");
        assert!(findings.is_empty());
    }

    #[test]
    fn parse_clippy_warning() {
        let line = r#"{"reason":"compiler-message","package_id":"x","manifest_path":"","target":{"kind":["lib"],"name":"x"},"message":{"level":"warning","message":"unused variable","code":{"code":"unused_variables","explanation":null},"rendered":"warning: unused variable\n  --> src/lib.rs:5:9\n","spans":[{"file_name":"src/lib.rs","line_start":5,"line_end":5,"column_start":9,"column_end":10,"is_primary":true,"byte_start":0,"byte_end":0,"expansion":null,"label":null,"suggested_replacement":null,"suggestion_applicability":null,"text":[]}],"children":[]}}"#;
        let findings = parse_clippy_output(line, "my-crate");
        assert_eq!(findings.len(), 1);
        assert!(findings[0].title.contains("my-crate"));
        assert!(findings[0].fingerprint.starts_with("clippy/my-crate/"));
        assert_eq!(findings[0].severity, FindingSeverity::Quality);
    }

    #[test]
    fn parse_clippy_skips_errors() {
        let line = r#"{"reason":"compiler-message","package_id":"x","manifest_path":"","target":{"kind":["lib"],"name":"x"},"message":{"level":"error","message":"mismatched types","code":{"code":"E0308","explanation":null},"rendered":"","spans":[],"children":[]}}"#;
        let findings = parse_clippy_output(line, "my-crate");
        assert!(findings.is_empty());
    }

    // ── stale_agent_panes ─────────────────────────────────────────────────────

    fn make_tmux_session(name: &str) -> TmuxSession {
        TmuxSession {
            name: name.to_string(),
            created: chrono::Utc::now(),
            last_activity: chrono::Utc::now(),
            windows: 1,
            attached: false,
            cwd: None,
        }
    }

    #[test]
    fn stale_agent_panes_empty_when_below_in_flight() {
        let sessions = vec![make_tmux_session("agent1"), make_tmux_session("agent2")];
        assert!(stale_agent_panes(&sessions, 2).is_empty());
        assert!(stale_agent_panes(&sessions, 5).is_empty());
    }

    #[test]
    fn stale_agent_panes_returns_excess() {
        let sessions = vec![
            make_tmux_session("agent1"),
            make_tmux_session("agent2"),
            make_tmux_session("agent3"),
        ];
        // 1 in-flight → 2 stale
        let stale = stale_agent_panes(&sessions, 1);
        assert_eq!(stale.len(), 2);
    }

    #[test]
    fn stale_agent_panes_ignores_non_agent_sessions() {
        let sessions = vec![
            make_tmux_session("conductr"),
            make_tmux_session("qa1"),
            make_tmux_session("agent1"),
        ];
        // 0 in-flight → only agent1 is stale
        let stale = stale_agent_panes(&sessions, 0);
        assert_eq!(stale, vec!["agent1"]);
    }

    #[test]
    fn stale_agent_panes_zero_in_flight_frees_all() {
        let sessions = vec![make_tmux_session("agent2"), make_tmux_session("agent1")];
        let stale = stale_agent_panes(&sessions, 0);
        assert_eq!(stale.len(), 2);
    }

    // ── Coverage scan ─────────────────────────────────────────────────────────

    fn make_llvm_cov_json(filename: &str, count: u64, covered: u64) -> String {
        let percent = if count == 0 { 100.0 } else { covered as f64 / count as f64 * 100.0 };
        format!(
            r#"{{"data":[{{"files":[{{"filename":"{filename}","segments":[],"summary":{{"lines":{{"count":{count},"covered":{covered},"percent":{percent}}}}}}}]}}],"type":"llvm.coverage.json.export","version":"2.0.1"}}"#
        )
    }

    #[test]
    fn parse_coverage_empty_json() {
        let json = r#"{"data":[],"type":"llvm.coverage.json.export","version":"2.0.1"}"#;
        let findings = parse_coverage_output(json, "my-crate", 0.6, &[]);
        assert!(findings.is_empty());
    }

    #[test]
    fn parse_coverage_file_above_threshold_not_flagged() {
        let json = make_llvm_cov_json("/workspace/my-crate/src/lib.rs", 100, 80);
        let findings = parse_coverage_output(&json, "my-crate", 0.6, &[]);
        assert!(findings.is_empty(), "80% >= 60%, should not be flagged");
    }

    #[test]
    fn parse_coverage_file_below_threshold_flagged() {
        let json = make_llvm_cov_json("/workspace/my-crate/src/lib.rs", 100, 40);
        let findings = parse_coverage_output(&json, "my-crate", 0.6, &[]);
        assert_eq!(findings.len(), 1);
        let f = &findings[0];
        assert!(f.title.contains("my-crate/src/lib.rs"));
        assert!(f.title.contains("40%"));
        assert_eq!(f.severity, FindingSeverity::Coverage);
        assert!(f.fingerprint.starts_with("coverage/my-crate/src/lib.rs"));
    }

    #[test]
    fn parse_coverage_exclude_glob_skips_file() {
        let json = make_llvm_cov_json("/workspace/my-crate/src/main.rs", 50, 10);
        let exclude = vec!["src/main.rs".to_string()];
        let findings = parse_coverage_output(&json, "my-crate", 0.6, &exclude);
        assert!(findings.is_empty(), "src/main.rs should be excluded");
    }

    #[test]
    fn parse_coverage_wildcard_exclude() {
        let json = make_llvm_cov_json("/workspace/my-crate/src/bin/foo.rs", 50, 10);
        let exclude = vec!["src/bin/*.rs".to_string()];
        let findings = parse_coverage_output(&json, "my-crate", 0.6, &exclude);
        assert!(findings.is_empty(), "src/bin/*.rs should match src/bin/foo.rs");
    }

    #[test]
    fn parse_coverage_empty_output() {
        let findings = parse_coverage_output("", "my-crate", 0.6, &[]);
        assert!(findings.is_empty());
    }

    #[test]
    fn parse_coverage_invalid_json() {
        let findings = parse_coverage_output("{not valid json}", "my-crate", 0.6, &[]);
        assert!(findings.is_empty());
    }

    #[test]
    fn parse_coverage_skips_non_src_files() {
        // File outside src/ should be ignored
        let json = make_llvm_cov_json("/workspace/my-crate/tests/integration.rs", 100, 0);
        let findings = parse_coverage_output(&json, "my-crate", 0.6, &[]);
        assert!(findings.is_empty(), "tests/ files should not be flagged");
    }

    #[test]
    fn uncovered_ranges_empty_segments() {
        let ranges = uncovered_ranges(&[]);
        assert!(ranges.is_empty());
    }

    #[test]
    fn uncovered_ranges_identifies_gap() {
        // Segment format: [line, col, count, has_count, is_region_entry, is_gap_region]
        let segments = vec![
            vec![
                serde_json::Value::Number(1u64.into()),
                serde_json::Value::Number(1u64.into()),
                serde_json::Value::Number(5u64.into()),
                serde_json::Value::Bool(true),
                serde_json::Value::Bool(true),
                serde_json::Value::Bool(false),
            ],
            vec![
                serde_json::Value::Number(5u64.into()),
                serde_json::Value::Number(1u64.into()),
                serde_json::Value::Number(0u64.into()),
                serde_json::Value::Bool(true),
                serde_json::Value::Bool(false),
                serde_json::Value::Bool(false),
            ],
            vec![
                serde_json::Value::Number(9u64.into()),
                serde_json::Value::Number(1u64.into()),
                serde_json::Value::Number(3u64.into()),
                serde_json::Value::Bool(true),
                serde_json::Value::Bool(true),
                serde_json::Value::Bool(false),
            ],
        ];
        let ranges = uncovered_ranges(&segments);
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0], (5, 8));
    }

    #[test]
    fn glob_match_exact() {
        assert!(glob_match("src/lib.rs", "src/lib.rs"));
        assert!(!glob_match("src/lib.rs", "src/main.rs"));
    }

    #[test]
    fn glob_match_star_in_segment() {
        assert!(glob_match("src/bin/*.rs", "src/bin/foo.rs"));
        assert!(glob_match("src/bin/*.rs", "src/bin/bar.rs"));
        assert!(!glob_match("src/bin/*.rs", "src/lib.rs"));
    }

    #[test]
    fn glob_match_double_star() {
        assert!(glob_match("src/**/*.rs", "src/nested/deep/mod.rs"));
        assert!(glob_match("src/**", "src/lib.rs"));
        assert!(glob_match("src/**", "src/a/b/c.rs"));
    }

    // ── has_cargo_toml ────────────────────────────────────────────────────────

    #[test]
    fn has_cargo_toml_true_when_present() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"test\"\n",
        )
        .unwrap();
        assert!(has_cargo_toml(dir.path()));
    }

    #[test]
    fn has_cargo_toml_false_when_absent() {
        let dir = tempfile::TempDir::new().unwrap();
        assert!(!has_cargo_toml(dir.path()));
    }

    // ── check_base_md ─────────────────────────────────────────────────────────

    #[test]
    fn check_base_md_missing_emits_finding() {
        let dir = tempfile::TempDir::new().unwrap();
        let findings = check_base_md(dir.path());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, FindingSeverity::Architecture);
        assert!(findings[0].fingerprint.contains("missing"));
        assert!(findings[0].body.contains("hexagonal"));
    }

    #[test]
    fn check_base_md_unstructured_emits_finding() {
        let dir = tempfile::TempDir::new().unwrap();
        let claude_dir = dir.path().join(".claude");
        std::fs::create_dir_all(&claude_dir).unwrap();
        std::fs::write(claude_dir.join("base.md"), "Just a paragraph, no headings.\n").unwrap();
        let findings = check_base_md(dir.path());
        assert_eq!(findings.len(), 1);
        assert!(findings[0].fingerprint.contains("unstructured"));
    }

    #[test]
    fn check_base_md_structured_returns_empty() {
        let dir = tempfile::TempDir::new().unwrap();
        let claude_dir = dir.path().join(".claude");
        std::fs::create_dir_all(&claude_dir).unwrap();
        std::fs::write(
            claude_dir.join("base.md"),
            "# Architecture\n\n## Pattern\n\nHexagonal.\n\n## Rules\n\n1. foo\n",
        )
        .unwrap();
        let findings = check_base_md(dir.path());
        assert!(
            findings.is_empty(),
            "structured base.md should produce no findings"
        );
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    /// Create a temp directory with `crates/<name>/Cargo.toml` containing `content`.
    fn tempdir_with_cargo_toml(content: &str, crate_name: &str) -> tempfile::TempDir {
        let dir = tempfile::TempDir::new().unwrap();
        let crate_dir = dir.path().join("crates").join(crate_name);
        std::fs::create_dir_all(&crate_dir).unwrap();
        std::fs::write(crate_dir.join("Cargo.toml"), content).unwrap();
        dir
    }
}
