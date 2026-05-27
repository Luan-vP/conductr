//! Reads sections of the `.conductr` project config file.

use std::path::Path;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use conductr_core::types::CiMode;
use conductr_core::SafetyPreset;
use serde::{Deserialize, Serialize};

// ── [local] ───────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Default)]
pub struct LocalSection {
    /// Preferred local-agent provider ("ollama", "llamacpp", or "pi").
    pub provider: Option<String>,
    /// Default model name (ollama only; overridden by `--model` / `CONDUCTR_LOCAL_MODEL`).
    pub model: Option<String>,
}

// ── [ci] ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CiSection {
    /// Ordered command list. First non-zero exit = Failing. All zero = Passing.
    #[serde(default)]
    pub commands: Vec<String>,
    /// Per-command timeout in seconds.
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    /// How to resolve local vs GitHub CI status.
    #[serde(default)]
    pub mode: CiMode,
    /// Per-PR CI run records appended by the orchestrate pass (`[[ci.runs]]`).
    #[serde(default)]
    pub runs: Vec<CiRun>,
}

impl Default for CiSection {
    fn default() -> Self {
        Self {
            commands: Vec::new(),
            timeout_secs: default_timeout_secs(),
            mode: CiMode::default(),
            runs: Vec::new(),
        }
    }
}

fn default_timeout_secs() -> u64 { 900 }

/// Anthropic model voicing for a role slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VoiceModel {
    Opus,
    Sonnet,
    Haiku,
}

/// Default model voicings per role for tmux-pane execution.
/// Unknown role keys are rejected (`deny_unknown_fields`).
#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct BandDefaults {
    pub architect: Option<VoiceModel>,
    pub implementer: Option<VoiceModel>,
    pub reviewer: Option<VoiceModel>,
    pub tester: Option<VoiceModel>,
    pub security: Option<VoiceModel>,
    #[serde(rename = "doc-writer")]
    pub doc_writer: Option<VoiceModel>,
    #[serde(rename = "idle-sweeper")]
    pub idle_sweeper: Option<VoiceModel>,
}

/// The `[band]` section of `.conductr`.
#[derive(Debug, Deserialize, Default)]
pub struct BandSection {
    pub human_assignee: Option<String>,
    #[serde(default)]
    pub defaults: BandDefaults,
}

/// Complexity tier used to decide whether to escalate to a tmux pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum Complexity {
    S,
    M,
    L,
    #[serde(rename = "XL")]
    Xl,
}

/// A single CI run record written after each orchestrate pass.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CiRun {
    pub pr: u64,
    pub minutes: f64,
    pub ts: DateTime<Utc>,
}

// ── [[tempo.prs]] ─────────────────────────────────────────────────────────────

/// Size complexity label for a PR.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "UPPERCASE")]
pub enum PrComplexity {
    Xs,
    S,
    #[default]
    M,
    L,
}

/// A single per-PR record in `[[tempo.prs]]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TempoPr {
    pub number: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phrase: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chord: Option<String>,
    #[serde(default)]
    pub complexity: PrComplexity,
    pub opened: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closed: Option<DateTime<Utc>>,
    pub merged: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct TempoSection {
    #[serde(default)]
    pub prs: Vec<TempoPr>,
}

// ── [orchestrate] ─────────────────────────────────────────────────────────────

/// The `[orchestrate]` section of `.conductr`.
/// Combines #91's chord-size + pacing fields with #96's band/QA cap fields.
#[derive(Debug, Deserialize)]
pub struct OrchestrateSection {
    /// Maximum number of parallel `agent<n>` slots (chord size cap).
    #[serde(default = "default_max_parallel_beats")]
    pub max_parallel_beats: u32,
    /// Maximum number of parallel `qa<n>` slots.
    #[serde(default)]
    pub max_parallel_qa: Option<u32>,
    /// Escalate to tmux at this complexity tier or above.
    #[serde(default)]
    pub tmux_complexity_min: Option<Complexity>,
    /// Reserved for future pacing rules; v1 always uses 0 (no overlap constraint).
    #[serde(default)]
    pub phrase_overlap: u32,
}

fn default_max_parallel_beats() -> u32 { 3 }

impl Default for OrchestrateSection {
    fn default() -> Self {
        Self {
            max_parallel_beats: default_max_parallel_beats(),
            max_parallel_qa: None,
            tmux_complexity_min: None,
            phrase_overlap: 0,
        }
    }
}

// ── [safety] ─────────────────────────────────────────────────────────────────

/// Per-routine preset overrides stored under `[safety.overrides]`.
#[derive(Debug, Deserialize, Default)]
pub struct SafetyOverridesSection {
    pub architect: Option<SafetyPreset>,
    pub implementer: Option<SafetyPreset>,
    pub reviewer: Option<SafetyPreset>,
    pub tester: Option<SafetyPreset>,
    pub security: Option<SafetyPreset>,
    #[serde(rename = "doc-writer")]
    pub doc_writer: Option<SafetyPreset>,
    #[serde(rename = "idle-sweeper")]
    pub idle_sweeper: Option<SafetyPreset>,
}

/// The `[safety]` section of `.conductr`.
#[derive(Debug, Deserialize, Default)]
pub struct SafetySection {
    /// User-pinned preset. When absent, the maturity-derived default applies.
    pub preset: Option<SafetyPreset>,
    /// Per-routine preset overrides (`[safety.overrides]`).
    #[serde(default)]
    pub overrides: SafetyOverridesSection,
}

// ── RawConfig ─────────────────────────────────────────────────────────────────


#[derive(Debug, Deserialize, Default)]
pub struct ArchitectureSection {
    pub style: Option<String>,
    pub reference: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct IdleSection {
    pub last_module: Option<String>,
    pub last_run: Option<String>,
    /// Line-coverage fraction below which a file is flagged (0.0–1.0).
    #[serde(default = "default_coverage_threshold")]
    pub coverage_threshold: f32,
    /// Glob patterns (relative to crate root) for files to skip in coverage scan.
    #[serde(default)]
    pub coverage_exclude: Vec<String>,
}

fn default_coverage_threshold() -> f32 { 0.6 }

impl Default for IdleSection {
    fn default() -> Self {
        Self {
            last_module: None,
            last_run: None,
            coverage_threshold: default_coverage_threshold(),
            coverage_exclude: Vec::new(),
        }
    }
}

#[derive(Debug, Deserialize, Default)]
struct RawConfig {
    pub project_tag: Option<String>,
    pub repo: Option<String>,
    #[serde(default)]
    pub local: LocalSection,
    #[serde(default)]
    pub ci: CiSection,
    #[serde(default)]
    pub band: BandSection,
    #[serde(default)]
    pub orchestrate: OrchestrateSection,
    #[serde(default)]
    pub tempo: TempoSection,
    #[serde(default)]
    pub architecture: ArchitectureSection,
    #[serde(default)]
    pub idle: IdleSection,
    #[serde(default)]
    pub safety: SafetySection,
}

/// Read `project_tag` from `.conductr` in `repo_path`.
/// Returns `None` when the file or field is absent.
pub fn read_project_tag(repo_path: &Path) -> Result<Option<String>> {
    read_raw(repo_path).map(|c| c.project_tag)
}

/// Validate that `tag` contains only `[a-z0-9-]` and is non-empty.
pub fn validate_tag(tag: &str) -> Result<()> {
    if tag.is_empty() {
        anyhow::bail!("project tag must not be empty");
    }
    if !tag.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
        anyhow::bail!(
            "project tag '{}' contains invalid characters (allowed: [a-z0-9-])",
            tag
        );
    }
    Ok(())
}

/// Ensure a `.conductr` file exists in `repo_path`.
///
/// If the file already exists, returns the `project_tag` from it (error if the
/// field is missing).  If the file does not exist, derives the tag from the git
/// remote `origin`, writes a minimal `.conductr`, and returns the derived tag.
///
/// Failure mode: if no git remote `origin` is configured, returns an error
/// asking the caller to create `.conductr` manually.
pub fn ensure_dot_conductr(repo_path: &Path) -> Result<String> {
    let path = repo_path.join(".conductr");
    if path.exists() {
        return read_raw(repo_path)?
            .project_tag
            .ok_or_else(|| anyhow::anyhow!(".conductr exists but has no `project_tag` field"));
    }

    let (tag, repo_slug) = derive_tag_and_repo_from_git(repo_path)?;
    validate_tag(&tag)?;

    let content = format!(
        "# Project config for {} — created automatically by `conductr begin`.\n\
         # Edit `project_tag` to change the namespace slug (legal chars: [a-z0-9-]).\n\
         \n\
         project_tag = \"{}\"\n\
         repo        = \"{}\"\n",
        tag, tag, repo_slug,
    );

    std::fs::write(&path, content)
        .with_context(|| format!("writing {}", path.display()))?;

    println!("created {} (project_tag = \"{}\")", path.display(), tag);
    Ok(tag)
}

fn derive_tag_and_repo_from_git(repo_path: &Path) -> Result<(String, String)> {
    let output = std::process::Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(repo_path)
        .output()
        .context("running git remote get-url origin")?;

    if !output.status.success() {
        anyhow::bail!(
            "no git remote 'origin' found; cannot derive project tag automatically.\n\
             Create a .conductr file manually with `project_tag = \"<name>\"`"
        );
    }

    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let (owner_repo, tag) = parse_url(&url);
    Ok((tag, owner_repo))
}

/// Parse a git remote URL into `(owner/repo, tag)`.
/// Handles both HTTPS (`https://github.com/owner/repo.git`) and
/// SSH (`git@github.com:owner/repo.git`) forms.
fn parse_url(url: &str) -> (String, String) {
    let without_git = url.trim_end_matches('/').trim_end_matches(".git");

    // Split on '/'. Both URL forms end with "owner/repo" after their prefix:
    //   HTTPS: "https://github.com/owner/repo"  → ["https:", "", "github.com", "owner", "repo"]
    //   SSH:   "git@github.com:owner/repo"       → ["git@github.com:owner",               "repo"]
    let parts: Vec<&str> = without_git.split('/').collect();
    let n = parts.len();

    let repo_name = parts.last().copied().unwrap_or(without_git);

    let owner_repo = if n >= 2 {
        let owner_part = parts[n - 2];
        // SSH form has "git@host:owner" as the second-to-last segment; strip the host.
        let owner = owner_part.rsplit(':').next().unwrap_or(owner_part);
        format!("{}/{}", owner, repo_name)
    } else {
        repo_name.to_string()
    };

    let tag = repo_name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string();

    (owner_repo, tag)
}

// ── Public readers ────────────────────────────────────────────────────────────

/// Read the `[local]` section from `.conductr` in `repo_path`.
/// Returns defaults when the file or section is absent.
pub fn read_local_section(repo_path: &Path) -> Result<LocalSection> {
    read_raw(repo_path).map(|c| c.local)
}

/// Read the `[ci]` section from `.conductr` in `repo_path`.
/// Returns defaults when the file or section is absent.
pub fn read_ci_section(repo_path: &Path) -> Result<CiSection> {
    read_raw(repo_path).map(|c| c.ci)
}

/// Read the `[band]` section from `.conductr` in `repo_path`.
/// Returns defaults when the file or section is absent.
pub fn read_band_section(repo_path: &Path) -> Result<BandSection> {
    read_raw(repo_path).map(|c| c.band)
}

/// Read the `[tempo]` section (including `[[tempo.prs]]`) from `.conductr`.
/// Returns an empty section when the file or section is absent.
pub fn read_tempo_section(repo_path: &Path) -> Result<TempoSection> {
    read_raw(repo_path).map(|c| c.tempo)
}

/// Read the `[orchestrate]` section from `.conductr` in `repo_path`.
/// Returns defaults when the file or section is absent.
pub fn read_orchestrate_section(repo_path: &Path) -> Result<OrchestrateSection> {
    read_raw(repo_path).map(|c| c.orchestrate)
}

/// Read the `[architecture]` section from `.conductr` in `repo_path`.
/// Returns defaults when the file or section is absent.
pub fn read_architecture_section(repo_path: &Path) -> Result<ArchitectureSection> {
    read_raw(repo_path).map(|c| c.architecture)
}

/// Read the `[idle]` section from `.conductr` in `repo_path`.
/// Returns defaults when the file or section is absent.
pub fn read_idle_section(repo_path: &Path) -> Result<IdleSection> {
    read_raw(repo_path).map(|c| c.idle)
}

/// Read the top-level `repo` key from `.conductr` in `repo_path`.
/// Returns `None` when the file or key is absent.
pub fn read_project_repo(repo_path: &Path) -> Result<Option<String>> {
    read_raw(repo_path).map(|c| c.repo)
}

/// Read the `[safety]` section (including `[safety.overrides]`) from `.conductr`.
/// Returns defaults when the file or section is absent.
pub fn read_safety_section(repo_path: &Path) -> Result<SafetySection> {
    read_raw(repo_path).map(|c| c.safety)
}

/// Write `[safety] preset = "<preset>"` to `.conductr`, preserving all other content.
///
/// If no `[safety]` section exists it is appended; if the key already exists it is
/// updated in-place.
pub fn write_safety_preset(repo_path: &Path, preset: SafetyPreset) -> Result<()> {
    let dot_conductr = repo_path.join(".conductr");
    let content = if dot_conductr.exists() {
        std::fs::read_to_string(&dot_conductr)
            .with_context(|| format!("reading {}", dot_conductr.display()))?
    } else {
        String::new()
    };
    let updated = patch_safety_preset(&content, preset.as_str());
    std::fs::write(&dot_conductr, updated)
        .with_context(|| format!("writing {}", dot_conductr.display()))
}

/// Write a per-routine preset override to `[safety.overrides]` in `.conductr`.
///
/// If no `[safety.overrides]` section exists it is inserted (after `[safety]` if
/// present, otherwise appended); if the role key already exists it is updated.
pub fn write_safety_override(
    repo_path: &Path,
    role: conductr_core::SafetyRole,
    preset: SafetyPreset,
) -> Result<()> {
    let dot_conductr = repo_path.join(".conductr");
    let content = if dot_conductr.exists() {
        std::fs::read_to_string(&dot_conductr)
            .with_context(|| format!("reading {}", dot_conductr.display()))?
    } else {
        String::new()
    };
    let updated = patch_safety_override(&content, role.as_str(), Some(preset.as_str()));
    std::fs::write(&dot_conductr, updated)
        .with_context(|| format!("writing {}", dot_conductr.display()))
}

/// Remove a per-routine preset override from `[safety.overrides]` in `.conductr`.
///
/// If the key is not present this is a no-op.
pub fn clear_safety_override(
    repo_path: &Path,
    role: conductr_core::SafetyRole,
) -> Result<()> {
    let dot_conductr = repo_path.join(".conductr");
    let content = if dot_conductr.exists() {
        std::fs::read_to_string(&dot_conductr)
            .with_context(|| format!("reading {}", dot_conductr.display()))?
    } else {
        return Ok(());
    };
    let updated = patch_safety_override(&content, role.as_str(), None);
    std::fs::write(&dot_conductr, updated)
        .with_context(|| format!("writing {}", dot_conductr.display()))
}

/// Update `[idle].last_module` and `[idle].last_run` in `.conductr` while
/// preserving all other content (comments, other sections).
pub fn write_idle_state(repo_path: &Path, last_module: &str, last_run: &str) -> Result<()> {
    let dot_conductr = repo_path.join(".conductr");
    let content = if dot_conductr.exists() {
        std::fs::read_to_string(&dot_conductr)
            .with_context(|| format!("reading {}", dot_conductr.display()))?
    } else {
        String::new()
    };
    let updated = patch_idle_section(&content, last_module, last_run);
    std::fs::write(&dot_conductr, updated)
        .with_context(|| format!("writing {}", dot_conductr.display()))
}

fn patch_idle_section(content: &str, last_module: &str, last_run: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let trailing_newline = content.ends_with('\n');

    let mut idle_start: Option<usize> = None;
    let mut next_section: Option<usize> = None;

    for (i, &line) in lines.iter().enumerate() {
        let t = line.trim();
        if t == "[idle]" {
            idle_start = Some(i);
        } else if idle_start.is_some()
            && t.starts_with('[')
            && !t.starts_with("[[")
            && t.ends_with(']')
        {
            next_section = Some(i);
            break;
        }
    }

    let lm_line = format!("last_module = \"{last_module}\"");
    let lr_line = format!("last_run    = \"{last_run}\"");

    if let Some(start) = idle_start {
        let end = next_section.unwrap_or(lines.len());

        let mut result: Vec<String> = Vec::with_capacity(lines.len() + 2);
        let mut found_lm = false;
        let mut found_lr = false;

        for (i, &line) in lines.iter().enumerate() {
            if i >= start + 1 && i < end {
                let t = line.trim();
                if t.starts_with("last_module") {
                    result.push(lm_line.clone());
                    found_lm = true;
                } else if t.starts_with("last_run") {
                    result.push(lr_line.clone());
                    found_lr = true;
                } else {
                    result.push(line.to_string());
                }
            } else {
                result.push(line.to_string());
            }
        }

        let insert_base = start + 1;
        if !found_lm {
            result.insert(insert_base, lm_line);
        }
        if !found_lr {
            let pos = insert_base + usize::from(!found_lm);
            result.insert(pos, lr_line);
        }

        let mut out = result.join("\n");
        if trailing_newline {
            out.push('\n');
        }
        out
    } else {
        let mut out = content.to_string();
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str("\n[idle]\n");
        out.push_str(&lm_line);
        out.push('\n');
        out.push_str(&lr_line);
        out.push('\n');
        out
    }
}

/// Patch `[safety] preset = "<value>"` in the raw TOML string, preserving
/// comments and other sections.
fn patch_safety_preset(content: &str, value: &str) -> String {
    let preset_line = format!("preset = \"{value}\"");
    patch_section_key(content, "[safety]", "preset", Some(&preset_line))
}

/// Patch (or remove) a role key in `[safety.overrides]`.
///
/// `value = Some(...)` → upsert the key.
/// `value = None`      → delete the key (clear override).
fn patch_safety_override(content: &str, role: &str, value: Option<&str>) -> String {
    let new_line = value.map(|v| format!("{role:<12} = \"{v}\""));
    patch_section_key(content, "[safety.overrides]", role, new_line.as_deref())
}

/// Generic helper: upsert or delete `key` inside `section_header`.
///
/// - If `new_line` is `Some(s)`, the key is set to `s` (insert if absent, replace if present).
/// - If `new_line` is `None`, the key line is removed (if present).
/// - If the section does not exist and `new_line` is `Some`, the section is appended.
///
/// Preserves trailing newline, comments, and all other sections.
fn patch_section_key(
    content: &str,
    section_header: &str,
    key: &str,
    new_line: Option<&str>,
) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let trailing_newline = content.ends_with('\n');

    // Find the target section and where the next section starts.
    let mut section_start: Option<usize> = None;
    let mut next_section: Option<usize> = None;

    for (i, &line) in lines.iter().enumerate() {
        let t = line.trim();
        if t == section_header {
            section_start = Some(i);
        } else if section_start.is_some()
            && t.starts_with('[')
            && !t.starts_with("[[")
            && t.ends_with(']')
            && t != section_header
        {
            next_section = Some(i);
            break;
        }
    }

    if let Some(start) = section_start {
        let end = next_section.unwrap_or(lines.len());

        let mut result: Vec<String> = Vec::with_capacity(lines.len() + 1);
        let mut key_found = false;

        for (i, &line) in lines.iter().enumerate() {
            if i >= start + 1 && i < end {
                let t = line.trim();
                if t.starts_with(key) && t[key.len()..].trim_start().starts_with('=') {
                    // Existing key — replace or delete.
                    if let Some(replacement) = new_line {
                        result.push(replacement.to_string());
                    }
                    // None → omit (delete)
                    key_found = true;
                } else {
                    result.push(line.to_string());
                }
            } else {
                result.push(line.to_string());
            }
        }

        // Insert if key was absent and we have a value.
        if !key_found {
            if let Some(replacement) = new_line {
                let insert_pos = start + 1;
                result.insert(insert_pos, replacement.to_string());
            }
        }

        let mut out = result.join("\n");
        if trailing_newline {
            out.push('\n');
        }
        out
    } else {
        // Section doesn't exist yet.
        if new_line.is_none() {
            return content.to_string();
        }
        let mut out = content.to_string();
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push('\n');
        out.push_str(section_header);
        out.push('\n');
        out.push_str(new_line.unwrap());
        out.push('\n');
        out
    }
}

fn read_raw(repo_path: &Path) -> Result<RawConfig> {
    let path = repo_path.join(".conductr");
    if !path.exists() {
        return Ok(RawConfig::default());
    }
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("reading {}", path.display()))?;
    toml::from_str(&raw).with_context(|| format!("parsing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_defaults_when_no_file() {
        let section = read_local_section(std::path::Path::new("/nonexistent/path")).unwrap();
        assert!(section.provider.is_none());
        assert!(section.model.is_none());
    }

    #[test]
    fn parses_toml_with_local_section() {
        let raw = r#"
project_tag = "test"
[local]
provider = "ollama"
model = "llama3"
"#;
        let cfg: RawConfig = toml::from_str(raw).unwrap();
        assert_eq!(cfg.local.provider.as_deref(), Some("ollama"));
        assert_eq!(cfg.local.model.as_deref(), Some("llama3"));
    }

    #[test]
    fn parses_toml_without_local_section() {
        let raw = r#"project_tag = "test""#;
        let cfg: RawConfig = toml::from_str(raw).unwrap();
        assert!(cfg.local.provider.is_none());
    }

    #[test]
    fn parses_ci_section() {
        let raw = r#"
[ci]
commands = ["cargo test", "cargo clippy"]
timeout_secs = 300
mode = "prefer-local"
"#;
        let cfg: RawConfig = toml::from_str(raw).unwrap();
        assert_eq!(cfg.ci.commands, ["cargo test", "cargo clippy"]);
        assert_eq!(cfg.ci.timeout_secs, 300);
        assert_eq!(cfg.ci.mode, CiMode::PreferLocal);
        assert!(cfg.ci.runs.is_empty());
    }

    #[test]
    fn ci_section_defaults_when_absent() {
        let raw = r#"project_tag = "test""#;
        let cfg: RawConfig = toml::from_str(raw).unwrap();
        assert!(cfg.ci.commands.is_empty());
        assert_eq!(cfg.ci.timeout_secs, 900);
        assert_eq!(cfg.ci.mode, CiMode::PreferLocal);
        assert!(cfg.ci.runs.is_empty());
    }

    #[test]
    fn ci_mode_github_parses() {
        let raw = r#"
[ci]
mode = "github"
"#;
        let cfg: RawConfig = toml::from_str(raw).unwrap();
        assert_eq!(cfg.ci.mode, CiMode::Github);
    }

    // ── [band] + [band.defaults] ──────────────────────────────────────────────

    #[test]
    fn band_section_parses_human_assignee_and_defaults() {
        let raw = r#"
[band]
human_assignee = "Luan-vP"

[band.defaults]
architect    = "opus"
implementer  = "sonnet"
reviewer     = "sonnet"
tester       = "haiku"
security     = "haiku"
doc-writer   = "sonnet"
idle-sweeper = "opus"
"#;
        let cfg: RawConfig = toml::from_str(raw).unwrap();
        assert_eq!(cfg.band.human_assignee.as_deref(), Some("Luan-vP"));
        assert_eq!(cfg.band.defaults.architect, Some(VoiceModel::Opus));
        assert_eq!(cfg.band.defaults.implementer, Some(VoiceModel::Sonnet));
        assert_eq!(cfg.band.defaults.reviewer, Some(VoiceModel::Sonnet));
        assert_eq!(cfg.band.defaults.tester, Some(VoiceModel::Haiku));
        assert_eq!(cfg.band.defaults.security, Some(VoiceModel::Haiku));
        assert_eq!(cfg.band.defaults.doc_writer, Some(VoiceModel::Sonnet));
        assert_eq!(cfg.band.defaults.idle_sweeper, Some(VoiceModel::Opus));
    }

    #[test]
    fn band_defaults_absent_gives_nones() {
        let raw = r#"project_tag = "test""#;
        let cfg: RawConfig = toml::from_str(raw).unwrap();
        assert!(cfg.band.human_assignee.is_none());
        assert!(cfg.band.defaults.architect.is_none());
    }

    #[test]
    fn band_defaults_partial_is_allowed() {
        let raw = r#"
[band.defaults]
architect = "sonnet"
"#;
        let cfg: RawConfig = toml::from_str(raw).unwrap();
        assert_eq!(cfg.band.defaults.architect, Some(VoiceModel::Sonnet));
        assert!(cfg.band.defaults.implementer.is_none());
    }

    #[test]
    fn band_defaults_rejects_unknown_role() {
        let raw = r#"
[band.defaults]
architect = "opus"
unknown-role = "sonnet"
"#;
        let result: Result<RawConfig, _> = toml::from_str(raw);
        assert!(result.is_err(), "unknown role should be rejected");
    }

    #[test]
    fn band_defaults_rejects_invalid_model() {
        let raw = r#"
[band.defaults]
architect = "gpt-4"
"#;
        let result: Result<RawConfig, _> = toml::from_str(raw);
        assert!(result.is_err(), "invalid model name should be rejected");
    }

    // ── [orchestrate] ─────────────────────────────────────────────────────────

    #[test]
    fn parses_ci_runs() {
        let raw = r#"
[ci]
mode = "prefer-local"

[[ci.runs]]
pr = 21
minutes = 4.2
ts = "2026-05-01T14:35:00Z"

[[ci.runs]]
pr = 22
minutes = 7.1
ts = "2026-05-02T09:10:00Z"
"#;
        let cfg: RawConfig = toml::from_str(raw).unwrap();
        assert_eq!(cfg.ci.runs.len(), 2);
        assert_eq!(cfg.ci.runs[0].pr, 21);
        assert!((cfg.ci.runs[0].minutes - 4.2).abs() < f64::EPSILON);
        assert_eq!(cfg.ci.runs[1].pr, 22);
    }

    #[test]
    fn parses_tempo_prs() {
        let raw = r#"
[[tempo.prs]]
number     = 21
phrase     = "begin"
chord      = "begin-impl-1"
complexity = "M"
opened     = "2026-05-01T09:12:00Z"
closed     = "2026-05-01T14:38:00Z"
merged     = true

[[tempo.prs]]
number     = 22
complexity = "S"
opened     = "2026-05-02T08:00:00Z"
merged     = false
"#;
        let cfg: RawConfig = toml::from_str(raw).unwrap();
        assert_eq!(cfg.tempo.prs.len(), 2);
        let pr = &cfg.tempo.prs[0];
        assert_eq!(pr.number, 21);
        assert_eq!(pr.phrase.as_deref(), Some("begin"));
        assert_eq!(pr.chord.as_deref(), Some("begin-impl-1"));
        assert_eq!(pr.complexity, PrComplexity::M);
        assert!(pr.merged);
        assert!(pr.closed.is_some());

        let pr2 = &cfg.tempo.prs[1];
        assert_eq!(pr2.number, 22);
        assert_eq!(pr2.complexity, PrComplexity::S);
        assert!(!pr2.merged);
        assert!(pr2.phrase.is_none());
        assert!(pr2.closed.is_none());
    }

    #[test]
    fn tempo_section_defaults_when_absent() {
        let raw = r#"project_tag = "test""#;
        let cfg: RawConfig = toml::from_str(raw).unwrap();
        assert!(cfg.tempo.prs.is_empty());
    }

    // ── [orchestrate] ─────────────────────────────────────────────────────────

    #[test]
    fn orchestrate_section_parses_all_fields() {
        let raw = r#"
[orchestrate]
max_parallel_beats  = 5
max_parallel_qa     = 2
tmux_complexity_min = "L"
phrase_overlap      = 1
"#;
        let cfg: RawConfig = toml::from_str(raw).unwrap();
        assert_eq!(cfg.orchestrate.max_parallel_beats, 5);
        assert_eq!(cfg.orchestrate.max_parallel_qa, Some(2));
        assert_eq!(cfg.orchestrate.tmux_complexity_min, Some(Complexity::L));
        assert_eq!(cfg.orchestrate.phrase_overlap, 1);
    }

    #[test]
    fn orchestrate_section_defaults_when_absent() {
        let raw = r#"project_tag = "test""#;
        let cfg: RawConfig = toml::from_str(raw).unwrap();
        assert_eq!(cfg.orchestrate.max_parallel_beats, 3);
        assert!(cfg.orchestrate.max_parallel_qa.is_none());
        assert!(cfg.orchestrate.tmux_complexity_min.is_none());
        assert_eq!(cfg.orchestrate.phrase_overlap, 0);
    }

    #[test]
    fn orchestrate_complexity_xl_parses() {
        let raw = r#"
[orchestrate]
tmux_complexity_min = "XL"
"#;
        let cfg: RawConfig = toml::from_str(raw).unwrap();
        assert_eq!(cfg.orchestrate.tmux_complexity_min, Some(Complexity::Xl));
    }

    #[test]
    fn orchestrate_complexity_invalid_is_rejected() {
        let raw = r#"
[orchestrate]
tmux_complexity_min = "huge"
"#;
        let result: Result<RawConfig, _> = toml::from_str(raw);
        assert!(result.is_err(), "invalid complexity value should be rejected");
    }

    #[test]
    fn complexity_defaults_to_m() {
        let raw = r#"
[[tempo.prs]]
number = 99
opened = "2026-05-01T00:00:00Z"
merged = false
"#;
        let cfg: RawConfig = toml::from_str(raw).unwrap();
        assert_eq!(cfg.tempo.prs[0].complexity, PrComplexity::M);
    }

    // ── namespacing (project_tag / repo / parse_url) ──────────────────────────

    #[test]
    fn parses_project_tag() {
        let raw = r#"project_tag = "my-project""#;
        let cfg: RawConfig = toml::from_str(raw).unwrap();
        assert_eq!(cfg.project_tag.as_deref(), Some("my-project"));
    }

    #[test]
    fn parses_repo_field() {
        let raw = r#"
project_tag = "foo"
repo        = "acme/foo"
"#;
        let cfg: RawConfig = toml::from_str(raw).unwrap();
        assert_eq!(cfg.project_tag.as_deref(), Some("foo"));
        assert_eq!(cfg.repo.as_deref(), Some("acme/foo"));
    }

    #[test]
    fn validate_tag_accepts_valid() {
        assert!(validate_tag("foo").is_ok());
        assert!(validate_tag("my-project").is_ok());
        assert!(validate_tag("conductr").is_ok());
        assert!(validate_tag("project123").is_ok());
        assert!(validate_tag("a").is_ok());
    }

    #[test]
    fn validate_tag_rejects_uppercase() {
        assert!(validate_tag("MyProject").is_err());
    }

    #[test]
    fn validate_tag_rejects_slash() {
        assert!(validate_tag("repo/product").is_err());
    }

    #[test]
    fn validate_tag_rejects_empty() {
        assert!(validate_tag("").is_err());
    }

    #[test]
    fn parse_url_https() {
        let (owner_repo, tag) = parse_url("https://github.com/acme/my-project.git");
        assert_eq!(tag, "my-project");
        assert!(owner_repo.contains("my-project"));
    }

    #[test]
    fn parse_url_ssh() {
        let (owner_repo, tag) = parse_url("git@github.com:acme/my-project.git");
        assert_eq!(tag, "my-project");
        assert!(owner_repo.contains("my-project"));
    }

    #[test]
    fn parse_url_uppercase_sanitised() {
        let (_owner_repo, tag) = parse_url("https://github.com/acme/My-Repo.git");
        assert_eq!(tag, "my-repo");
    }

    #[test]
    fn parse_url_conductr_repo() {
        let (_owner_repo, tag) = parse_url("https://github.com/Luan-vP/conductr.git");
        assert_eq!(tag, "conductr");
    }

    // ── [architecture] + [idle] ──────────────────────────────────────────────

    #[test]
    fn parses_architecture_section() {
        let raw = r#"
[architecture]
style     = "hexagonal"
reference = ".claude/base.md"
"#;
        let cfg: RawConfig = toml::from_str(raw).unwrap();
        assert_eq!(cfg.architecture.style.as_deref(), Some("hexagonal"));
        assert_eq!(cfg.architecture.reference.as_deref(), Some(".claude/base.md"));
    }

    #[test]
    fn parses_idle_section() {
        let raw = r#"
[idle]
last_module = "conductr-orchestrate"
last_run    = "2026-05-11T08:00:00Z"
"#;
        let cfg: RawConfig = toml::from_str(raw).unwrap();
        assert_eq!(cfg.idle.last_module.as_deref(), Some("conductr-orchestrate"));
        assert_eq!(cfg.idle.last_run.as_deref(), Some("2026-05-11T08:00:00Z"));
    }

    #[test]
    fn patch_idle_section_inserts_when_missing() {
        let content = "project_tag = \"foo\"\n";
        let out = patch_idle_section(content, "conductr-pod", "2026-01-01T00:00:00Z");
        assert!(out.contains("[idle]"));
        assert!(out.contains("last_module = \"conductr-pod\""));
        assert!(out.contains("last_run    = \"2026-01-01T00:00:00Z\""));
        // Original content preserved
        assert!(out.contains("project_tag = \"foo\""));
    }

    #[test]
    fn patch_idle_section_updates_existing() {
        let content = "[idle]\nlast_module = \"\"\nlast_run    = \"\"\n";
        let out = patch_idle_section(content, "conductr-tasks", "2026-05-11T09:00:00Z");
        assert!(out.contains("last_module = \"conductr-tasks\""));
        assert!(out.contains("last_run    = \"2026-05-11T09:00:00Z\""));
    }

    #[test]
    fn patch_idle_section_preserves_surrounding_sections() {
        let content = "[cadence]\norchestrate = \"*/30 * * * *\"\n[idle]\nlast_module = \"\"\nlast_run = \"\"\n[band]\nagents = []\n";
        let out = patch_idle_section(content, "conductr-mail", "2026-05-11T10:00:00Z");
        assert!(out.contains("[cadence]"));
        assert!(out.contains("[band]"));
        assert!(out.contains("last_module = \"conductr-mail\""));
    }

    // ── [safety] ──────────────────────────────────────────────────────────────

    #[test]
    fn parses_safety_section_with_preset() {
        let raw = r#"
[safety]
preset = "strict"
"#;
        let cfg: RawConfig = toml::from_str(raw).unwrap();
        assert_eq!(cfg.safety.preset, Some(SafetyPreset::Strict));
        assert!(cfg.safety.overrides.architect.is_none());
    }

    #[test]
    fn parses_safety_overrides() {
        let raw = r#"
[safety]
preset = "fast"

[safety.overrides]
architect   = "strict"
implementer = "feral"
"#;
        let cfg: RawConfig = toml::from_str(raw).unwrap();
        assert_eq!(cfg.safety.preset, Some(SafetyPreset::Fast));
        assert_eq!(cfg.safety.overrides.architect, Some(SafetyPreset::Strict));
        assert_eq!(cfg.safety.overrides.implementer, Some(SafetyPreset::Feral));
        assert!(cfg.safety.overrides.reviewer.is_none());
    }

    #[test]
    fn safety_section_defaults_when_absent() {
        let raw = r#"project_tag = "test""#;
        let cfg: RawConfig = toml::from_str(raw).unwrap();
        assert!(cfg.safety.preset.is_none());
        assert!(cfg.safety.overrides.architect.is_none());
    }

    #[test]
    fn safety_overrides_doc_writer_and_idle_sweeper() {
        let raw = r#"
[safety.overrides]
doc-writer   = "bureaucratic"
idle-sweeper = "unhinged"
"#;
        let cfg: RawConfig = toml::from_str(raw).unwrap();
        assert_eq!(cfg.safety.overrides.doc_writer, Some(SafetyPreset::Bureaucratic));
        assert_eq!(cfg.safety.overrides.idle_sweeper, Some(SafetyPreset::Unhinged));
    }

    #[test]
    fn patch_safety_preset_inserts_when_missing() {
        let content = "project_tag = \"foo\"\n";
        let out = patch_safety_preset(content, "strict");
        assert!(out.contains("[safety]"), "should insert [safety] section");
        assert!(out.contains("preset = \"strict\""));
        assert!(out.contains("project_tag = \"foo\""), "original content preserved");
    }

    #[test]
    fn patch_safety_preset_updates_existing() {
        let content = "[safety]\npreset = \"feral\"\n";
        let out = patch_safety_preset(content, "fast");
        assert!(out.contains("preset = \"fast\""));
        assert!(!out.contains("preset = \"feral\""));
    }

    #[test]
    fn patch_safety_preset_preserves_overrides_section() {
        let content = "[safety]\npreset = \"fast\"\n\n[safety.overrides]\narchitect = \"strict\"\n";
        let out = patch_safety_preset(content, "strict");
        assert!(out.contains("[safety.overrides]"));
        assert!(out.contains("architect = \"strict\""));
        assert!(out.contains("preset = \"strict\""));
    }

    #[test]
    fn patch_safety_preset_preserves_surrounding_sections() {
        let content = "[band]\nhuman_assignee = \"Luan-vP\"\n[safety]\npreset = \"fast\"\n[idle]\nlast_run = \"\"\n";
        let out = patch_safety_preset(content, "bureaucratic");
        assert!(out.contains("[band]"));
        assert!(out.contains("[idle]"));
        assert!(out.contains("preset = \"bureaucratic\""));
    }

    #[test]
    fn patch_safety_override_inserts_when_no_section() {
        let content = "project_tag = \"foo\"\n";
        let out = patch_safety_override(content, "architect", Some("strict"));
        assert!(out.contains("[safety.overrides]"));
        assert!(out.contains("architect"));
        assert!(out.contains("\"strict\""));
    }

    #[test]
    fn patch_safety_override_updates_existing_key() {
        let content = "[safety.overrides]\narchitect    = \"fast\"\n";
        let out = patch_safety_override(content, "architect", Some("strict"));
        assert!(out.contains("\"strict\""));
        assert!(!out.contains("\"fast\""));
    }

    #[test]
    fn patch_safety_override_deletes_key() {
        let content = "[safety.overrides]\narchitect    = \"strict\"\nimplementer  = \"fast\"\n";
        let out = patch_safety_override(content, "architect", None);
        assert!(!out.contains("architect"), "cleared key must be removed");
        assert!(out.contains("implementer"), "other keys preserved");
    }

    #[test]
    fn patch_safety_override_clear_noop_when_absent() {
        let content = "project_tag = \"foo\"\n";
        let out = patch_safety_override(content, "architect", None);
        assert_eq!(out, content, "no-op when section and key absent");
    }

    #[test]
    fn safety_round_trip_read_write() {
        use tempfile::TempDir;
        let dir = TempDir::new().unwrap();
        let path = dir.path();

        // Start: no safety section
        std::fs::write(path.join(".conductr"), "project_tag = \"foo\"\n").unwrap();

        write_safety_preset(path, SafetyPreset::Strict).unwrap();
        let section = read_safety_section(path).unwrap();
        assert_eq!(section.preset, Some(SafetyPreset::Strict));

        write_safety_override(path, conductr_core::SafetyRole::Architect, SafetyPreset::Bureaucratic).unwrap();
        let section = read_safety_section(path).unwrap();
        assert_eq!(section.overrides.architect, Some(SafetyPreset::Bureaucratic));

        clear_safety_override(path, conductr_core::SafetyRole::Architect).unwrap();
        let section = read_safety_section(path).unwrap();
        assert!(section.overrides.architect.is_none(), "override should be cleared");
        assert_eq!(section.preset, Some(SafetyPreset::Strict), "global preset preserved");
    }
}
