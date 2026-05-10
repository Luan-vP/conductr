//! Local CI adapter: runs project-defined commands in an isolated git worktree.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::process::Command;

use conductr_core::ports::LocalCi;
use conductr_core::types::{CiStatus, CommandRun, LocalCiConfig, LocalCiReport};

pub struct LocalCiAdapter {
    pub repo_root: PathBuf,
    pub config: LocalCiConfig,
}

impl LocalCiAdapter {
    pub fn new(repo_root: PathBuf, config: LocalCiConfig) -> Self {
        Self { repo_root, config }
    }
}

#[async_trait]
impl LocalCi for LocalCiAdapter {
    async fn run(&self, head_ref: &str) -> anyhow::Result<LocalCiReport> {
        // Sanitise the branch name for use in a path component.
        let safe_ref = head_ref.replace(['/', '\\', ' '], "-");
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let worktree_path = std::env::temp_dir().join(format!("conductr-ci-{safe_ref}-{ts}"));

        // Fetch the remote branch into FETCH_HEAD.
        let fetch_result = run_git(
            &["fetch", "--depth=1", "origin", head_ref],
            &self.repo_root,
        )
        .await;

        if let Err(e) = fetch_result {
            return Ok(LocalCiReport {
                status: CiStatus::Unknown,
                commands: vec![CommandRun {
                    cmd: format!("git fetch origin {head_ref}"),
                    exit: -1,
                    stdout_tail: String::new(),
                    stderr_tail: e.to_string(),
                    duration_secs: 0.0,
                }],
            });
        }

        // Create an isolated worktree at FETCH_HEAD.
        let wt_result = run_git(
            &["worktree", "add", "--detach", worktree_path.to_str().unwrap_or(""), "FETCH_HEAD"],
            &self.repo_root,
        )
        .await;

        if let Err(e) = wt_result {
            return Ok(LocalCiReport {
                status: CiStatus::Unknown,
                commands: vec![CommandRun {
                    cmd: format!("git worktree add {} FETCH_HEAD", worktree_path.display()),
                    exit: -1,
                    stdout_tail: String::new(),
                    stderr_tail: e.to_string(),
                    duration_secs: 0.0,
                }],
            });
        }

        // RAII guard removes the worktree even on panic/early return.
        let _guard = WorktreeGuard {
            repo_root: self.repo_root.clone(),
            worktree_path: worktree_path.clone(),
        };

        let timeout = Duration::from_secs(self.config.timeout_secs);
        let mut runs: Vec<CommandRun> = Vec::new();

        for cmd_str in &self.config.commands {
            let start = Instant::now();
            match run_command_in_worktree(cmd_str, &worktree_path, timeout).await {
                Ok((exit, stdout, stderr)) => {
                    let duration_secs = start.elapsed().as_secs_f64();
                    runs.push(CommandRun {
                        cmd: cmd_str.clone(),
                        exit,
                        stdout_tail: stdout,
                        stderr_tail: stderr,
                        duration_secs,
                    });
                    if exit != 0 {
                        return Ok(LocalCiReport { status: CiStatus::Failing, commands: runs });
                    }
                }
                Err(e) => {
                    let duration_secs = start.elapsed().as_secs_f64();
                    runs.push(CommandRun {
                        cmd: cmd_str.clone(),
                        exit: -1,
                        stdout_tail: String::new(),
                        stderr_tail: e.to_string(),
                        duration_secs,
                    });
                    return Ok(LocalCiReport { status: CiStatus::Unknown, commands: runs });
                }
            }
        }

        Ok(LocalCiReport { status: CiStatus::Passing, commands: runs })
    }
}

async fn run_git(args: &[&str], cwd: &std::path::Path) -> anyhow::Result<()> {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .await?;
    if !out.status.success() {
        anyhow::bail!(
            "`git {}` failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

async fn run_command_in_worktree(
    cmd: &str,
    cwd: &std::path::Path,
    timeout: Duration,
) -> anyhow::Result<(i32, String, String)> {
    let future = Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .output();

    match tokio::time::timeout(timeout, future).await {
        Ok(Ok(out)) => {
            let exit = out.status.code().unwrap_or(-1);
            let stdout = tail_lines(&String::from_utf8_lossy(&out.stdout), 80);
            let stderr = tail_lines(&String::from_utf8_lossy(&out.stderr), 80);
            Ok((exit, stdout, stderr))
        }
        Ok(Err(e)) => Err(e.into()),
        Err(_) => anyhow::bail!("command timed out after {}s", timeout.as_secs()),
    }
}

fn tail_lines(s: &str, n: usize) -> String {
    let lines: Vec<&str> = s.lines().collect();
    if lines.len() <= n { s.to_string() } else { lines[lines.len() - n..].join("\n") }
}

struct WorktreeGuard {
    repo_root: PathBuf,
    worktree_path: PathBuf,
}

impl Drop for WorktreeGuard {
    fn drop(&mut self) {
        let _ = std::process::Command::new("git")
            .args(["worktree", "remove", "--force"])
            .arg(&self.worktree_path)
            .current_dir(&self.repo_root)
            .output();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_lines_short() {
        let s = "a\nb\nc";
        assert_eq!(tail_lines(s, 10), "a\nb\nc");
    }

    #[test]
    fn tail_lines_truncates() {
        let lines: Vec<String> = (0..100).map(|i| i.to_string()).collect();
        let s = lines.join("\n");
        let result = tail_lines(&s, 80);
        let result_lines: Vec<&str> = result.lines().collect();
        assert_eq!(result_lines.len(), 80);
        assert_eq!(result_lines[0], "20");
        assert_eq!(result_lines[79], "99");
    }
}
