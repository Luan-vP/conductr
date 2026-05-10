//! Wrapper around the `br` CLI (beads_rust) — a local-first SQLite issue tracker.
//!
//! This deliberately avoids a Rust path-dependency on `beads_rust` so we don't
//! pick up its build.rs / SQLite-driver toolchain. Instead we shell out to
//! the installed `br` binary and parse its `--json` output.

use std::process::Stdio;

use serde::Deserialize;
use tokio::process::Command;

use crate::{Task, TaskStatus};

#[derive(Debug, Clone)]
pub struct Beads {
    pub binary: String,
    pub repo_dir: Option<std::path::PathBuf>,
}

impl Default for Beads {
    fn default() -> Self {
        Self { binary: "br".into(), repo_dir: None }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BeadsError {
    #[error("`br` not found on PATH (install via beads_rust install.sh)")]
    NotInstalled,
    #[error("`br` exited with status {0}: {1}")]
    Exit(std::process::ExitStatus, String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Deserialize)]
struct RawIssue {
    id: String,
    title: String,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    priority: Option<u8>,
    #[serde(default)]
    depends_on: Vec<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    body: Option<String>,
}

impl From<RawIssue> for Task {
    fn from(r: RawIssue) -> Self {
        let status = match r.status.as_deref() {
            Some("in-progress") | Some("in_progress") => TaskStatus::InProgress,
            Some("blocked") => TaskStatus::Blocked,
            Some("closed") | Some("done") => TaskStatus::Done,
            _ => TaskStatus::NotStarted,
        };
        Task {
            id: r.id,
            title: r.title,
            status,
            priority: r.priority,
            depends_on: r.depends_on,
            tags: r.tags,
            body: r.body,
        }
    }
}

impl Beads {
    pub fn new() -> Self { Self::default() }

    pub fn with_binary(mut self, binary: impl Into<String>) -> Self {
        self.binary = binary.into();
        self
    }

    pub fn with_repo_dir(mut self, p: impl Into<std::path::PathBuf>) -> Self {
        self.repo_dir = Some(p.into());
        self
    }

    async fn run(&self, args: &[&str]) -> Result<String, BeadsError> {
        let mut cmd = Command::new(&self.binary);
        if let Some(d) = &self.repo_dir {
            cmd.current_dir(d);
        }
        let out = cmd
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    BeadsError::NotInstalled
                } else {
                    BeadsError::Io(e)
                }
            })?;
        if !out.status.success() {
            return Err(BeadsError::Exit(
                out.status,
                String::from_utf8_lossy(&out.stderr).into_owned(),
            ));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    pub async fn init(&self) -> Result<(), BeadsError> {
        self.run(&["init"]).await.map(|_| ())
    }

    pub async fn list_ready(&self) -> Result<Vec<Task>, BeadsError> {
        let out = self.run(&["ready", "--json"]).await?;
        let raw: Vec<RawIssue> = serde_json::from_str(&out)?;
        Ok(raw.into_iter().map(Task::from).collect())
    }

    pub async fn list(&self) -> Result<Vec<Task>, BeadsError> {
        let out = self.run(&["list", "--json"]).await?;
        let raw: Vec<RawIssue> = serde_json::from_str(&out)?;
        Ok(raw.into_iter().map(Task::from).collect())
    }

    pub async fn create(&self, title: &str, priority: Option<u8>) -> Result<Task, BeadsError> {
        self.create_full(title, priority, None, &[]).await
    }

    /// Create with optional body and labels.
    pub async fn create_full(
        &self,
        title: &str,
        priority: Option<u8>,
        body: Option<&str>,
        labels: &[&str],
    ) -> Result<Task, BeadsError> {
        let prio = priority.map(|p| p.to_string());
        let labels_csv = if labels.is_empty() { None } else { Some(labels.join(",")) };
        let mut args: Vec<&str> = vec!["create", title, "--json"];
        if let Some(p) = &prio {
            args.push("-p");
            args.push(p);
        }
        if let Some(b) = body {
            args.push("-d");
            args.push(b);
        }
        if let Some(l) = &labels_csv {
            args.push("-l");
            args.push(l);
        }
        let out = self.run(&args).await?;
        let raw: RawIssue = serde_json::from_str(&out)?;
        Ok(raw.into())
    }

    pub async fn close(&self, id: &str) -> Result<(), BeadsError> {
        self.run(&["close", id]).await.map(|_| ())
    }

    pub async fn sync_flush(&self) -> Result<(), BeadsError> {
        self.run(&["sync", "--flush-only"]).await.map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_to_task_status_mapping() {
        let raw = RawIssue {
            id: "br-1".into(),
            title: "x".into(),
            status: Some("in-progress".into()),
            priority: Some(2),
            depends_on: vec![],
            tags: vec![],
            body: None,
        };
        let t: Task = raw.into();
        assert_eq!(t.status, TaskStatus::InProgress);
        assert_eq!(t.priority, Some(2));
    }
}
