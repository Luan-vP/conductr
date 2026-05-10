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

#[derive(Debug, Deserialize, Default)]
struct RawConfig {
    #[serde(default)]
    pub local: LocalSection,
    #[serde(default)]
    pub ci: CiSection,
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
}
