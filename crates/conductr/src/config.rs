//! Reads sections of the `.conductr` project config file.

use std::path::Path;

use anyhow::{Context, Result};
use conductr_core::types::CiMode;
use serde::Deserialize;

#[derive(Debug, Deserialize, Default)]
pub struct LocalSection {
    /// Preferred local-agent provider ("ollama", "llamacpp", or "pi").
    pub provider: Option<String>,
    /// Default model name (ollama only; overridden by `--model` / `CONDUCTR_LOCAL_MODEL`).
    pub model: Option<String>,
}

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
}

impl Default for CiSection {
    fn default() -> Self {
        Self { commands: Vec::new(), timeout_secs: default_timeout_secs(), mode: CiMode::default() }
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

/// The `[orchestrate]` section of `.conductr`.
#[derive(Debug, Deserialize, Default)]
pub struct OrchestrateSection {
    /// Maximum number of parallel `agent<n>` slots.
    pub max_parallel_beats: Option<u32>,
    /// Maximum number of parallel `qa<n>` slots.
    pub max_parallel_qa: Option<u32>,
    /// Escalate to tmux at this complexity tier or above.
    pub tmux_complexity_min: Option<Complexity>,
}

#[derive(Debug, Deserialize, Default)]
struct RawConfig {
    #[serde(default)]
    pub local: LocalSection,
    #[serde(default)]
    pub ci: CiSection,
    #[serde(default)]
    pub band: BandSection,
    #[serde(default)]
    pub orchestrate: OrchestrateSection,
}

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

/// Read the `[orchestrate]` section from `.conductr` in `repo_path`.
/// Returns defaults when the file or section is absent.
pub fn read_orchestrate_section(repo_path: &Path) -> Result<OrchestrateSection> {
    read_raw(repo_path).map(|c| c.orchestrate)
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
    }

    #[test]
    fn ci_section_defaults_when_absent() {
        let raw = r#"project_tag = "test""#;
        let cfg: RawConfig = toml::from_str(raw).unwrap();
        assert!(cfg.ci.commands.is_empty());
        assert_eq!(cfg.ci.timeout_secs, 900);
        assert_eq!(cfg.ci.mode, CiMode::PreferLocal);
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
    fn orchestrate_section_parses_all_fields() {
        let raw = r#"
[orchestrate]
max_parallel_beats  = 3
max_parallel_qa     = 2
tmux_complexity_min = "L"
"#;
        let cfg: RawConfig = toml::from_str(raw).unwrap();
        assert_eq!(cfg.orchestrate.max_parallel_beats, Some(3));
        assert_eq!(cfg.orchestrate.max_parallel_qa, Some(2));
        assert_eq!(cfg.orchestrate.tmux_complexity_min, Some(Complexity::L));
    }

    #[test]
    fn orchestrate_section_defaults_when_absent() {
        let raw = r#"project_tag = "test""#;
        let cfg: RawConfig = toml::from_str(raw).unwrap();
        assert!(cfg.orchestrate.max_parallel_beats.is_none());
        assert!(cfg.orchestrate.max_parallel_qa.is_none());
        assert!(cfg.orchestrate.tmux_complexity_min.is_none());
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
}
