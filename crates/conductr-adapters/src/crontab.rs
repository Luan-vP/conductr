use async_trait::async_trait;
use tokio::process::Command;

use conductr_core::ports::{CrontabAgent, CrontabError};

#[derive(Debug, Clone, Default)]
pub struct Crontab;

impl Crontab {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl CrontabAgent for Crontab {
    async fn list(&self) -> Result<String, CrontabError> {
        let out = Command::new("crontab")
            .arg("-l")
            .output()
            .await
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    CrontabError::NotInstalled
                } else {
                    CrontabError::Io(e.to_string())
                }
            })?;

        if out.status.success() {
            return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
        }

        let stderr = String::from_utf8_lossy(&out.stderr);
        if stderr.contains("no crontab") {
            return Err(CrontabError::NoCrontab);
        }
        Err(CrontabError::Io(stderr.into_owned()))
    }
}
