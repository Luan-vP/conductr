//! GitHub mailbox adapter — stores `MailMessage`s as comments on a sentinel
//! issue (the "mail issue") in the repository.
//!
//! Each comment body is a fenced JSON block containing the serialised
//! `MailMessage`. The sentinel issue number is provided at construction.

use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use chrono::Utc;
use tokio::process::Command;
use std::process::Stdio;

use conductr_core::ports::{Mailbox, MailboxError};
use conductr_core::signals::{MailKind, MailMessage, MailRef};

/// Mailbox backed by comments on a GitHub sentinel issue.
#[derive(Debug)]
pub struct GhMailbox {
    repo: String,
    sentinel_issue: u64,
    counter: AtomicU64,
}

impl GhMailbox {
    pub fn new(repo: impl Into<String>, sentinel_issue: u64) -> Self {
        Self {
            repo: repo.into(),
            sentinel_issue,
            counter: AtomicU64::new(0),
        }
    }

    fn next_id(&self) -> String {
        let seq = self.counter.fetch_add(1, Ordering::SeqCst);
        format!("gh-{}-{seq}", Utc::now().timestamp_millis())
    }

    async fn run_gh(&self, args: &[&str]) -> Result<String, MailboxError> {
        let out = Command::new("gh")
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| MailboxError::Io(e.to_string()))?;

        if !out.status.success() {
            return Err(MailboxError::Backend(
                String::from_utf8_lossy(&out.stderr).trim().to_string(),
            ));
        }
        String::from_utf8(out.stdout).map_err(|e| MailboxError::Parse(e.to_string()))
    }

    async fn list_comments(&self) -> Result<Vec<MailMessage>, MailboxError> {
        #[derive(serde::Deserialize)]
        struct View {
            comments: Vec<GhComment>,
        }
        #[derive(serde::Deserialize)]
        struct GhComment {
            body: String,
        }

        let out = self
            .run_gh(&[
                "issue",
                "view",
                &self.sentinel_issue.to_string(),
                "--repo",
                &self.repo,
                "--json",
                "comments",
            ])
            .await?;

        let view: View = serde_json::from_str(&out)
            .map_err(|e| MailboxError::Parse(e.to_string()))?;

        let mut messages = Vec::new();
        for comment in view.comments {
            if let Some(json) = extract_mail_json(&comment.body) {
                match serde_json::from_str::<MailMessage>(json) {
                    Ok(msg) => messages.push(msg),
                    Err(_) => {}
                }
            }
        }
        Ok(messages)
    }
}

fn kind_tag(kind: &MailKind) -> &'static str {
    match kind {
        MailKind::ScopeClaim { .. } => "scope_claim",
        MailKind::SynthesisRequest { .. } => "synthesis_request",
        MailKind::SynthesisProposal { .. } => "synthesis_proposal",
        MailKind::Note { .. } => "note",
        MailKind::Yell { .. } => "yell",
    }
}

// Extract JSON from a fenced code block like:
//
//     ```conductr-mail
//     { ... }
//     ```
fn extract_mail_json(body: &str) -> Option<&str> {
    let start = body.find("```conductr-mail\n")?;
    let json_start = start + "```conductr-mail\n".len();
    let end = body[json_start..].find("```")?;
    Some(body[json_start..json_start + end].trim())
}

#[async_trait]
impl Mailbox for GhMailbox {
    async fn send(&self, agent: &str, payload: MailKind) -> Result<MailRef, MailboxError> {
        let id = self.next_id();
        let msg = MailMessage {
            id: id.clone(),
            agent: agent.to_string(),
            sent_at: Utc::now(),
            payload,
        };
        let json = serde_json::to_string_pretty(&msg)
            .map_err(|e| MailboxError::Parse(e.to_string()))?;
        let body = format!("```conductr-mail\n{json}\n```");

        self.run_gh(&[
            "issue",
            "comment",
            &self.sentinel_issue.to_string(),
            "--repo",
            &self.repo,
            "--body",
            &body,
        ])
        .await?;

        Ok(id)
    }

    async fn inbox(
        &self,
        kind_filter: Option<&str>,
        since: Option<std::time::Duration>,
    ) -> Result<Vec<MailMessage>, MailboxError> {
        let all = self.list_comments().await?;
        let cutoff = since.map(|d| Utc::now() - chrono::Duration::from_std(d).unwrap_or(chrono::Duration::zero()));
        Ok(all
            .into_iter()
            .filter(|m| {
                if let Some(filter) = kind_filter {
                    if kind_tag(&m.payload) != filter {
                        return false;
                    }
                }
                if let Some(cutoff) = cutoff {
                    if m.sent_at < cutoff {
                        return false;
                    }
                }
                true
            })
            .collect())
    }

    async fn thread(&self, thread_id: &MailRef) -> Result<Vec<MailMessage>, MailboxError> {
        let all = self.list_comments().await?;
        Ok(all.into_iter().filter(|m| &m.id == thread_id).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_mail_json_works() {
        let body = "some text\n```conductr-mail\n{\"hello\":\"world\"}\n```\nmore text";
        assert_eq!(extract_mail_json(body), Some("{\"hello\":\"world\"}"));
    }

    #[test]
    fn extract_mail_json_missing_returns_none() {
        let body = "no fenced block here";
        assert_eq!(extract_mail_json(body), None);
    }
}
