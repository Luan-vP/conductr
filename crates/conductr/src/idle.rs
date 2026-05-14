//! `conductr idle` — architecture scan, module scan, and issue filing.
//!
//! Driving-adapter layer: lives in the binary crate, composes existing ports.

use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use anyhow::Result;
use conductr_core::ports::{LocalAgent, ScmHost};
use conductr_core::types::RepoSlug;
use serde::Deserialize;

/// Ordered list of use-case crates for round-robin scanning.
const USE_CASE_CRATES: &[&str] = &[
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

// ── Public types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum FindingSeverity {
    Architecture,
    Quality,
    Suggestion,
}

#[derive(Debug, Clone)]
pub struct Finding {
    pub title: String,
    pub body: String,
    pub severity: FindingSeverity,
    pub fingerprint: String,
}

pub struct IdleRunConfig {
    pub repo_path: PathBuf,
    pub repo_slug: RepoSlug,
    pub dry_run: bool,
    pub max_issues: usize,
    pub no_llm: bool,
}

// ── Entry point ───────────────────────────────────────────────────────────────

pub async fn run(
    scm: &dyn ScmHost,
    agent: Option<&dyn LocalAgent>,
    cfg: &IdleRunConfig,
) -> Result<()> {
    // Phase 1: read config
    let arch_cfg = crate::config::read_architecture_section(&cfg.repo_path)?;
    let idle_state = crate::config::read_idle_section(&cfg.repo_path)?;

    // Phase 2: architecture scan
    let mut findings: Vec<Finding> = Vec::new();
    if arch_cfg.style.as_deref() == Some("hexagonal") {
        findings.extend(check_rule1(&cfg.repo_path));
        findings.extend(check_rule3(&cfg.repo_path));
        findings.extend(check_rule4(&cfg.repo_path));
    }

    // Phase 3: module pick + scan
    let last_module = idle_state.last_module.as_deref().unwrap_or("");
    let current_module = pick_module(last_module);
    let module_findings = run_module_scan(&cfg.repo_path, current_module, agent, cfg.no_llm).await;
    findings.extend(module_findings);

    // Phase 4: file issues (or print for dry-run)
    let open_titles: HashSet<String> = if cfg.dry_run {
        HashSet::new()
    } else {
        scm.list_open_issues(&cfg.repo_slug)
            .await?
            .into_iter()
            .map(|i| i.title)
            .collect()
    };

    let to_file: Vec<&Finding> = findings
        .iter()
        .filter(|f| !open_titles.contains(&f.title))
        .take(cfg.max_issues)
        .collect();

    if cfg.dry_run {
        for f in &to_file {
            println!("[dry-run] {}: {}", severity_label(&f.severity), f.title);
            println!("{}", f.body);
            println!("---");
        }
    } else {
        ensure_idle_labels(&cfg.repo_slug).await;

        for f in &to_file {
            let sev_label = severity_label(&f.severity);
            scm.create_issue(
                &cfg.repo_slug,
                &f.title,
                &f.body,
                &["idle-finding", sev_label],
            )
            .await?;
        }

        // Phase 5: persist state
        let run_ts = chrono::Utc::now().to_rfc3339();
        crate::config::write_idle_state(&cfg.repo_path, current_module, &run_ts)?;
    }

    println!(
        "idle: {} finding(s) total, {} to file, scanned module={}",
        findings.len(),
        to_file.len(),
        current_module,
    );
    Ok(())
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

// ── Module scan ───────────────────────────────────────────────────────────────

async fn run_module_scan(
    repo_path: &Path,
    crate_name: &str,
    agent: Option<&dyn LocalAgent>,
    no_llm: bool,
) -> Vec<Finding> {
    let mut findings = Vec::new();

    // Deterministic: cargo clippy
    match run_clippy(repo_path, crate_name).await {
        Ok(f) => findings.extend(f),
        Err(e) => eprintln!("idle: clippy scan failed for {crate_name}: {e}"),
    }

    // LLM scan (optional)
    if !no_llm {
        if let Some(agent) = agent {
            let llm_findings = llm_scan(crate_name, repo_path, agent).await;
            findings.extend(llm_findings);
        }
    }

    findings
}

// ── Clippy integration ────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct ClippyMessage {
    reason: String,
    message: Option<ClippyDiagnostic>,
}

#[derive(Deserialize)]
struct ClippyDiagnostic {
    level: String,
    message: String,
    code: Option<ClippyCode>,
    rendered: Option<String>,
    spans: Vec<ClippySpan>,
}

#[derive(Deserialize)]
struct ClippyCode {
    code: String,
}

#[derive(Deserialize)]
struct ClippySpan {
    file_name: String,
    line_start: u32,
    is_primary: bool,
}

async fn run_clippy(repo_path: &Path, crate_name: &str) -> Result<Vec<Finding>> {
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
    // Deduplicate by fingerprint within this run
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

// ── LLM scan ──────────────────────────────────────────────────────────────────

const LLM_SOURCE_CAP: usize = 32 * 1024;

async fn llm_scan(
    crate_name: &str,
    repo_path: &Path,
    agent: &dyn LocalAgent,
) -> Vec<Finding> {
    let crate_path = repo_path.join("crates").join(crate_name);
    let source = read_crate_source(&crate_path, LLM_SOURCE_CAP);
    if source.is_empty() {
        return vec![];
    }

    let prompt = format!(
        "Review this Rust crate named `{crate_name}` and list up to 5 specific, \
         actionable improvements for refactoring, efficiency, or code quality.\n\n\
         Format each item as a numbered list starting with a short title followed by \
         a colon, then a description. Example:\n\
         1. Replace manual loop with iterator: The `collect` on line 42 can be \
         written more idiomatically with `.map(...).collect()`.\n\n\
         Source code:\n\n\
         ```rust\n{source}\n```"
    );

    match agent.complete(&prompt).await {
        Ok(response) => parse_llm_suggestions(crate_name, &response),
        Err(e) => {
            eprintln!("idle: LLM scan failed for {crate_name}: {e}");
            vec![]
        }
    }
}

fn read_crate_source(crate_path: &Path, max_bytes: usize) -> String {
    let src_path = crate_path.join("src");
    let mut buf = String::new();
    collect_rs_files(&src_path, &mut buf, max_bytes);
    buf
}

fn collect_rs_files(dir: &Path, buf: &mut String, max_bytes: usize) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        if buf.len() >= max_bytes {
            break;
        }
        if path.is_dir() {
            collect_rs_files(&path, buf, max_bytes);
        } else if path.extension() == Some(OsStr::new("rs")) {
            if let Ok(content) = std::fs::read_to_string(&path) {
                let remaining = max_bytes.saturating_sub(buf.len());
                if remaining == 0 {
                    break;
                }
                // Truncate at a valid UTF-8 character boundary.
                let mut take = content.len().min(remaining);
                while take > 0 && !content.is_char_boundary(take) {
                    take -= 1;
                }
                buf.push_str(&format!("\n// === {} ===\n", path.display()));
                buf.push_str(&content[..take]);
            }
        }
    }
}

pub fn parse_llm_suggestions(crate_name: &str, response: &str) -> Vec<Finding> {
    let mut findings = Vec::new();

    // Match lines starting with a number followed by period or parenthesis
    for line in response.lines() {
        let trimmed = line.trim();
        // e.g. "1. Title: description" or "1) Title"
        let rest = if let Some(r) = trimmed
            .strip_prefix(|c: char| c.is_ascii_digit())
            .and_then(|r| r.strip_prefix('.').or_else(|| r.strip_prefix(')')))
        {
            r.trim()
        } else {
            continue;
        };

        if rest.is_empty() {
            continue;
        }

        // Title is the part before the first colon, or the whole line.
        let title_raw = if let Some((t, _)) = rest.split_once(':') { t.trim() } else { rest };
        // Safe character-count truncation (no byte-boundary panic).
        let title: String = title_raw.chars().take(120).collect();

        let slug: String = title
            .to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '-' })
            .collect::<String>()
            .split('-')
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("-");
        let slug_short: String = slug.chars().take(60).collect();

        let fp = format!("llm/{crate_name}/{slug_short}");
        let body = format!(
            "## Suggestion\n\n\
             {rest}\n\n\
             ## Acceptance criteria\n\n\
             - [ ] Evaluate and implement (or explicitly reject with justification) \
             this suggestion in `{crate_name}`.\n\
             - [ ] `cargo test --workspace` passes.\n\n\
             <!-- conductr-idle-fingerprint: {fp} -->"
        );
        findings.push(Finding {
            title: format!("refactor: `{crate_name}`: {title}"),
            body,
            severity: FindingSeverity::Suggestion,
            fingerprint: fp,
        });

        if findings.len() >= 5 {
            break;
        }
    }
    findings
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn severity_label(s: &FindingSeverity) -> &'static str {
    match s {
        FindingSeverity::Architecture => "architecture",
        FindingSeverity::Quality => "quality",
        FindingSeverity::Suggestion => "refactor",
    }
}

/// Best-effort: create GitHub labels used by idle findings.
/// Silently ignores errors so a missing `gh` or missing auth doesn't abort the pass.
async fn ensure_idle_labels(repo: &RepoSlug) {
    let repo_str = repo.to_string();
    let labels = [
        ("idle-finding", "d4c5f9", "Auto-filed by conductr idle"),
        ("architecture", "e4e669", "Architecture rule violation"),
        ("quality", "bfd4f2", "Code quality finding"),
        ("refactor", "c2e0c6", "Refactor suggestion"),
    ];
    for (name, color, description) in &labels {
        let _ = tokio::process::Command::new("gh")
            .args([
                "label",
                "create",
                name,
                "--repo",
                &repo_str,
                "--color",
                color,
                "--description",
                description,
                "--force",
            ])
            .output()
            .await;
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

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

    // ── LLM suggestion parsing ────────────────────────────────────────────────

    #[test]
    fn parse_llm_suggestions_numbered_list() {
        let response = "Here are suggestions:\n\
                        1. Use iterators: Replace the loop with map/collect.\n\
                        2. Extract helper: The inner logic can be a separate function.\n\
                        3. Remove clone: The value is not used after this point.\n";
        let findings = parse_llm_suggestions("my-crate", response);
        assert_eq!(findings.len(), 3);
        assert!(findings[0].title.contains("my-crate"));
        assert!(findings[0].fingerprint.starts_with("llm/my-crate/"));
        assert_eq!(findings[0].severity, FindingSeverity::Suggestion);
    }

    #[test]
    fn parse_llm_suggestions_caps_at_five() {
        let response = (1..=8)
            .map(|i| format!("{i}. Suggestion {i}: Description {i}.\n"))
            .collect::<String>();
        let findings = parse_llm_suggestions("my-crate", &response);
        assert_eq!(findings.len(), 5);
    }

    #[test]
    fn parse_llm_suggestions_empty_response() {
        let findings = parse_llm_suggestions("my-crate", "");
        assert!(findings.is_empty());
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
