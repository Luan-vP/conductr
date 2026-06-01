//! Parser for the machine-wide `~/.conductr` registry.
//!
//! The registry lists every project this host is responsible for. Each
//! `[[projects]]` entry has at minimum `tag`, `repo`, `path`, and `status`.
//! A top-level `[defaults]` block seeds generated per-repo `.conductr` files.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

// ── Schema ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProjectStatus {
    Active,
    Pending,
}

impl ProjectStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProjectStatus::Active => "active",
            ProjectStatus::Pending => "pending",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct RegistryProject {
    pub tag: String,
    pub repo: String,
    pub path: PathBuf,
    pub status: ProjectStatus,
}

impl RegistryProject {
    /// Tmux session name for this project, per the `conductr-<tag>` convention.
    /// Shared by `setup spawn` (ensure-session) and `pod heal` (restart pass).
    pub fn session_name(&self) -> String {
        format!("conductr-{}", self.tag)
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct RegistryDefaults {
    pub human_assignee: Option<String>,
    pub local_provider: Option<String>,
    pub cadence_orchestrate: Option<String>,
    pub cadence_idle: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawRegistry {
    #[serde(default)]
    pub defaults: RegistryDefaults,
    #[serde(rename = "projects", default)]
    pub projects: Vec<RegistryProject>,
}

#[derive(Debug)]
pub struct Registry {
    pub defaults: RegistryDefaults,
    pub projects: Vec<RegistryProject>,
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Default path for the machine-wide registry (`~/.conductr`).
pub fn default_registry_path() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/root"));
    home.join(".conductr")
}

/// Load the registry from `path` (or `~/.conductr` if `None`).
/// Returns an error if the file is absent, malformed, or has duplicate tags.
pub fn load(path: Option<&Path>) -> Result<Registry> {
    let registry_path = match path {
        Some(p) => p.to_path_buf(),
        None => default_registry_path(),
    };

    let content = std::fs::read_to_string(&registry_path)
        .with_context(|| format!("reading registry {}", registry_path.display()))?;

    parse(&content).with_context(|| format!("parsing registry {}", registry_path.display()))
}

/// Parse a registry TOML string. Returns an error on duplicate tags.
pub fn parse(content: &str) -> Result<Registry> {
    let raw: RawRegistry = toml::from_str(content).context("invalid TOML in registry")?;

    let mut seen: HashSet<String> = HashSet::new();
    for project in &raw.projects {
        if !seen.insert(project.tag.clone()) {
            anyhow::bail!(
                "registry error: duplicate tag '{}' — each project must have a unique tag",
                project.tag
            );
        }
    }

    Ok(Registry {
        defaults: raw.defaults,
        projects: raw.projects,
    })
}

impl Registry {
    /// Return only the active projects.
    pub fn active(&self) -> impl Iterator<Item = &RegistryProject> {
        self.projects.iter().filter(|p| p.status == ProjectStatus::Active)
    }

    /// Return only the pending projects.
    pub fn pending(&self) -> impl Iterator<Item = &RegistryProject> {
        self.projects.iter().filter(|p| p.status == ProjectStatus::Pending)
    }

    /// Find a project by tag (case-sensitive).
    pub fn find_by_tag(&self, tag: &str) -> Option<&RegistryProject> {
        self.projects.iter().find(|p| p.tag == tag)
    }

    /// Find a project by its `owner/name` GitHub slug (case-sensitive). Used by
    /// `pod heal --repo`, which scopes work by slug rather than tag.
    pub fn find_by_repo(&self, repo: &str) -> Option<&RegistryProject> {
        self.projects.iter().find(|p| p.repo == repo)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE_REGISTRY: &str = r#"
[defaults]
human_assignee      = "Luan-vP"
local_provider      = "ollama"
cadence_orchestrate = "*/30 * * * *"
cadence_idle        = "17 * * * *"

[[projects]]
tag    = "conductr"
repo   = "Luan-vP/conductr"
path   = "/home/dev/developer/conductr"
status = "active"

[[projects]]
tag    = "myapp"
repo   = "Luan-vP/myapp"
path   = "/home/dev/developer/myapp"
status = "active"

[[projects]]
tag    = "future-project"
repo   = "Luan-vP/future-project"
path   = "/home/dev/developer/future-project"
status = "pending"
"#;

    #[test]
    fn parses_active_and_pending() {
        let reg = parse(FIXTURE_REGISTRY).unwrap();
        let active: Vec<_> = reg.active().collect();
        let pending: Vec<_> = reg.pending().collect();
        assert_eq!(active.len(), 2);
        assert_eq!(pending.len(), 1);
        assert_eq!(active[0].tag, "conductr");
        assert_eq!(active[1].tag, "myapp");
        assert_eq!(pending[0].tag, "future-project");
    }

    #[test]
    fn parses_defaults() {
        let reg = parse(FIXTURE_REGISTRY).unwrap();
        assert_eq!(reg.defaults.human_assignee.as_deref(), Some("Luan-vP"));
        assert_eq!(reg.defaults.local_provider.as_deref(), Some("ollama"));
        assert_eq!(reg.defaults.cadence_orchestrate.as_deref(), Some("*/30 * * * *"));
        assert_eq!(reg.defaults.cadence_idle.as_deref(), Some("17 * * * *"));
    }

    #[test]
    fn find_by_tag_returns_correct_project() {
        let reg = parse(FIXTURE_REGISTRY).unwrap();
        let p = reg.find_by_tag("myapp").unwrap();
        assert_eq!(p.repo, "Luan-vP/myapp");
        assert_eq!(p.status, ProjectStatus::Active);
    }

    #[test]
    fn find_by_tag_returns_none_for_missing() {
        let reg = parse(FIXTURE_REGISTRY).unwrap();
        assert!(reg.find_by_tag("nonexistent").is_none());
    }

    #[test]
    fn find_by_repo_returns_correct_project() {
        let reg = parse(FIXTURE_REGISTRY).unwrap();
        let p = reg.find_by_repo("Luan-vP/myapp").unwrap();
        assert_eq!(p.tag, "myapp");
        assert!(reg.find_by_repo("owner/nope").is_none());
    }

    #[test]
    fn session_name_uses_conductr_prefix() {
        let reg = parse(FIXTURE_REGISTRY).unwrap();
        assert_eq!(reg.find_by_tag("myapp").unwrap().session_name(), "conductr-myapp");
    }

    #[test]
    fn rejects_duplicate_tags() {
        let content = r#"
[[projects]]
tag    = "dup"
repo   = "owner/dup"
path   = "/tmp/dup"
status = "active"

[[projects]]
tag    = "dup"
repo   = "owner/dup2"
path   = "/tmp/dup2"
status = "pending"
"#;
        let err = parse(content).unwrap_err();
        assert!(err.to_string().contains("duplicate tag"), "{err}");
    }

    #[test]
    fn missing_path_detected() {
        // A registry path absent from disk is observably missing — `setup spawn`
        // keys its clone step off exactly this `Path::exists` check. Use a path
        // that cannot exist rather than a real-looking one, which is present on
        // a machine that actually hosts the project.
        let reg = parse(
            r#"
[[projects]]
tag    = "ghost"
repo   = "owner/ghost"
path   = "/nonexistent/owner/ghost"
status = "active"
"#,
        )
        .unwrap();
        let p = reg.find_by_tag("ghost").unwrap();
        assert!(!p.path.exists());
    }

    #[test]
    fn empty_registry_parses_ok() {
        let reg = parse("").unwrap();
        assert!(reg.projects.is_empty());
        assert!(reg.defaults.human_assignee.is_none());
    }

    #[test]
    fn defaults_applied_correctly() {
        let reg = parse(FIXTURE_REGISTRY).unwrap();
        let orch = reg.defaults.cadence_orchestrate.as_deref().unwrap_or("*/30 * * * *");
        let idle = reg.defaults.cadence_idle.as_deref().unwrap_or("17 * * * *");
        assert_eq!(orch, "*/30 * * * *");
        assert_eq!(idle, "17 * * * *");
    }
}
