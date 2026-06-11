use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use async_trait::async_trait;
use conductr_core::types::{RepoSlug, SafetyPreset};
use conductr_dashboard_core::model::{RepoEntry, RepoStatus};
use conductr_dashboard_core::SseEvent;
use serde::Deserialize;
use tokio::sync::broadcast;

use super::Aggregator;
use crate::state::SharedState;

/// Minimal shape of a `.conductr` project file used by the dashboard aggregator.
#[derive(Debug, Deserialize, Default)]
struct ConductrFile {
    #[serde(default)]
    cadence: HashMap<String, String>,
    #[serde(default)]
    orchestrate: OrchestrateSection,
}

#[derive(Debug, Deserialize, Default)]
struct OrchestrateSection {
    max_parallel_beats: Option<u32>,
    /// `UNHINGED | FERAL | FAST | STRICT | BUREAUCRATIC` — matches the wire enum.
    safety_preset: Option<SafetyPreset>,
}

pub struct ReposAggregator;

impl ReposAggregator {
    pub fn new() -> Self {
        Self
    }

    fn read_from_env() -> Vec<RepoEntry> {
        let raw = match std::env::var("CONDUCTR_REPOS") {
            Ok(v) if !v.is_empty() => v,
            _ => return Vec::new(),
        };

        let mut entries = Vec::new();
        for segment in raw.split(':') {
            let parts: Vec<&str> = segment.splitn(2, '=').collect();
            if parts.len() != 2 {
                continue;
            }
            let slug_str = parts[0];
            let path = parts[1];
            let slug_parts: Vec<&str> = slug_str.splitn(2, '/').collect();
            if slug_parts.len() != 2 {
                continue;
            }
            let slug = RepoSlug::new(slug_parts[0], slug_parts[1]);

            let (cadence, safety_preset, max_parallel_beats) =
                read_conductr_file(&format!("{path}/.conductr"));

            entries.push(RepoEntry {
                slug,
                tag: slug_str.replace('/', "-"),
                local_path: path.to_string(),
                status: RepoStatus::Active,
                cadence,
                maturity: None,
                safety_preset,
                max_parallel_beats,
            });
        }
        entries
    }
}

/// Parse a `.conductr` TOML file and extract the fields we care about.
/// Returns defaults on any read or parse error.
fn read_conductr_file(path: &str) -> (HashMap<String, String>, Option<SafetyPreset>, Option<u32>) {
    if !Path::new(path).exists() {
        return (Default::default(), None, None);
    }
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return (Default::default(), None, None),
    };
    let cfg: ConductrFile = match toml::from_str(&content) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("failed to parse {path}: {e}");
            return (Default::default(), None, None);
        }
    };
    (cfg.cadence, cfg.orchestrate.safety_preset, cfg.orchestrate.max_parallel_beats)
}

/// Update specific keys in a `.conductr` file without disturbing comments.
/// Returns the new file content.
pub fn patch_conductr_file(
    content: &str,
    safety: Option<&str>,
    beats: Option<u32>,
) -> String {
    let mut lines: Vec<String> = content.lines().map(String::from).collect();

    if let Some(preset) = safety {
        patch_toml_key(&mut lines, "orchestrate", "safety_preset", &format!("\"{preset}\""));
    }
    if let Some(b) = beats {
        patch_toml_key(&mut lines, "orchestrate", "max_parallel_beats", &b.to_string());
    }

    let mut out = lines.join("\n");
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Find `key = value` under `[section]` and replace its value in-place.
/// If the key is absent inside the section, it is appended after the section header.
/// If the section is absent, a new `[section]` with the key is appended at EOF.
fn patch_toml_key(lines: &mut Vec<String>, section: &str, key: &str, value: &str) {
    let section_header = format!("[{section}]");
    let key_prefix = format!("{key} ");

    // Find section start
    let section_idx = lines.iter().position(|l| l.trim() == section_header);

    if let Some(sec) = section_idx {
        // Find existing key within section (stops at next [section])
        let key_idx = lines[sec+1..].iter().position(|l| {
            let t = l.trim_start();
            t.starts_with(&key_prefix) || t.starts_with(&format!("{key}="))
        });
        if let Some(offset) = key_idx {
            let abs = sec + 1 + offset;
            lines[abs] = format!("{key} = {value}");
            return;
        }
        // Key not found in section; insert right after header
        lines.insert(sec + 1, format!("{key} = {value}"));
    } else {
        // Section not found; append at EOF
        lines.push(String::new());
        lines.push(section_header);
        lines.push(format!("{key} = {value}"));
    }
}

impl ReposAggregator {
    /// Re-read repos without going through the poll-loop.  Used by the config
    /// write handler so that the next `/state` response reflects the update.
    pub async fn refresh_blocking(&self, state: &SharedState) -> Result<()> {
        let repos = tokio::task::spawn_blocking(Self::read_from_env).await?;
        state.write().await.repos = repos;
        Ok(())
    }
}

#[async_trait]
impl Aggregator for ReposAggregator {
    async fn refresh(
        &self,
        state: &SharedState,
        _tx: &broadcast::Sender<SseEvent>,
    ) -> Result<()> {
        self.refresh_blocking(state).await
    }
}
