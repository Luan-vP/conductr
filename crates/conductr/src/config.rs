//! Reads the `[local]` section of the `.conductr` project config file.

use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Debug, Deserialize, Default)]
pub struct LocalSection {
    /// Preferred local-agent provider ("ollama", "llamacpp", or "pi").
    pub provider: Option<String>,
    /// Default model name (ollama only; overridden by `--model` / `CONDUCTR_LOCAL_MODEL`).
    pub model: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct RawConfig {
    #[serde(default)]
    pub local: LocalSection,
}

/// Read the `[local]` section from `.conductr` in `repo_path`.
/// Returns defaults when the file or section is absent.
pub fn read_local_section(repo_path: &Path) -> Result<LocalSection> {
    let path = repo_path.join(".conductr");
    if !path.exists() {
        return Ok(LocalSection::default());
    }
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("reading {}", path.display()))?;
    let cfg: RawConfig = toml::from_str(&raw)
        .with_context(|| format!("parsing {}", path.display()))?;
    Ok(cfg.local)
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
}
