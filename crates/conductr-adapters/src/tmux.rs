//! Thin wrapper around the `tmux` CLI, implementing [`TmuxAgent`].
//!
//! We shell out rather than link against libtmux because tmux's C API is not
//! stable and we only need a handful of read/write commands.

use std::process::Stdio;

use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use tokio::process::Command;

use conductr_core::ports::{TmuxAgent, TmuxError};
use conductr_core::types::TmuxSession;

#[derive(Debug, Clone, Default)]
pub struct Tmux {
    pub binary: String,
}

impl Tmux {
    pub fn new() -> Self {
        Self { binary: "tmux".into() }
    }

    pub fn with_binary(mut self, b: impl Into<String>) -> Self {
        self.binary = b.into();
        self
    }

    async fn run(&self, args: &[&str]) -> Result<String, TmuxError> {
        let out = Command::new(&self.binary)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    TmuxError::NotInstalled
                } else {
                    TmuxError::Io(e)
                }
            })?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
            if stderr.contains("no server running") || stderr.contains("error connecting") {
                return Err(TmuxError::NoServer);
            }
            return Err(TmuxError::Exit {
                args: args.join(" "),
                status: out.status,
                stderr,
            });
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }
}

#[async_trait]
impl TmuxAgent for Tmux {
    /// `tmux list-sessions` parsed into structured form.
    async fn list_sessions(&self) -> Result<Vec<TmuxSession>, TmuxError> {
        // Use a tab separator; session names can't contain whitespace and
        // pane_current_path won't realistically contain a tab.
        let fmt = "#{session_name}\t#{session_created}\t#{session_activity}\t#{session_windows}\t#{session_attached}\t#{pane_current_path}";
        let raw = match self.run(&["list-sessions", "-F", fmt]).await {
            Ok(s) => s,
            Err(TmuxError::NoServer) => return Ok(vec![]),
            Err(e) => return Err(e),
        };
        let mut out = Vec::new();
        for line in raw.lines() {
            if line.is_empty() {
                continue;
            }
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() < 6 {
                return Err(TmuxError::Parse(format!(
                    "unexpected list-sessions line: {line:?}"
                )));
            }
            let created: i64 = parts[1]
                .parse()
                .map_err(|e| TmuxError::Parse(format!("created: {e}")))?;
            let activity: i64 = parts[2]
                .parse()
                .map_err(|e| TmuxError::Parse(format!("activity: {e}")))?;
            let windows: u32 = parts[3]
                .parse()
                .map_err(|e| TmuxError::Parse(format!("windows: {e}")))?;
            let attached: u32 = parts[4]
                .parse()
                .map_err(|e| TmuxError::Parse(format!("attached: {e}")))?;
            out.push(TmuxSession {
                name: parts[0].to_string(),
                created: Utc
                    .timestamp_opt(created, 0)
                    .single()
                    .unwrap_or_else(Utc::now),
                last_activity: Utc
                    .timestamp_opt(activity, 0)
                    .single()
                    .unwrap_or_else(Utc::now),
                windows,
                attached: attached > 0,
                cwd: if parts[5].is_empty() {
                    None
                } else {
                    Some(parts[5].to_string())
                },
            });
        }
        Ok(out)
    }

    /// `tmux capture-pane -p` of the active pane in `session`.
    async fn capture_pane(&self, session: &str) -> Result<String, TmuxError> {
        self.run(&["capture-pane", "-t", session, "-p"]).await
    }

    /// Send literal text followed by Enter to a session's active pane.
    async fn send_line(&self, session: &str, text: &str) -> Result<(), TmuxError> {
        self.run(&["send-keys", "-t", session, "-l", text]).await?;
        self.run(&["send-keys", "-t", session, "Enter"]).await.map(|_| ())
    }

    /// Send a tmux key name (e.g. `Enter`, `C-c`) to a session.
    async fn send_key(&self, session: &str, key: &str) -> Result<(), TmuxError> {
        self.run(&["send-keys", "-t", session, key]).await.map(|_| ())
    }

    /// Create a new detached session named `name`, starting in `cwd`.
    async fn new_session(&self, name: &str, cwd: &str) -> Result<(), TmuxError> {
        self.run(&["new-session", "-d", "-s", name, "-c", cwd]).await.map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tmux_default_binary() {
        let t = Tmux::new();
        assert_eq!(t.binary, "tmux");
    }

    #[test]
    fn tmux_with_binary() {
        let t = Tmux::new().with_binary("tmux-custom");
        assert_eq!(t.binary, "tmux-custom");
    }
}
