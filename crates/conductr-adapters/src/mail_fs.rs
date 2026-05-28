//! Filesystem mailbox adapter — append-only JSONL files under
//! `.conductr/mail/<thread>.jsonl`.
//!
//! The default (inbox) thread is stored at `.conductr/mail/inbox.jsonl`.
//! Named threads are stored at `.conductr/mail/<thread_id>.jsonl`.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use chrono::Utc;

use conductr_core::ports::{Mailbox, MailboxError};
use conductr_core::types::{MailKind, MailMessage, MailRef};

#[derive(Debug)]
pub struct FsMailbox {
    dir: PathBuf,
    counter: AtomicU64,
}

impl FsMailbox {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into(), counter: AtomicU64::new(0) }
    }

    /// Default location: `.conductr/mail` inside the current directory.
    pub fn default_path() -> PathBuf {
        PathBuf::from(".conductr").join("mail")
    }

    fn inbox_path(&self) -> PathBuf {
        self.dir.join("inbox.jsonl")
    }

    fn thread_path(&self, thread_id: &str) -> PathBuf {
        self.dir.join(format!("{thread_id}.jsonl"))
    }

    fn ensure_dir(&self) -> Result<(), MailboxError> {
        std::fs::create_dir_all(&self.dir)
            .map_err(|e| MailboxError::Io(e.to_string()))
    }

    fn read_jsonl(&self, path: &Path) -> Result<Vec<MailMessage>, MailboxError> {
        if !path.exists() {
            return Ok(Vec::new());
        }
        let file = std::fs::File::open(path)
            .map_err(|e| MailboxError::Io(e.to_string()))?;
        let reader = std::io::BufReader::new(file);
        let mut messages = Vec::new();
        for line in reader.lines() {
            let line = line.map_err(|e| MailboxError::Io(e.to_string()))?;
            if line.trim().is_empty() {
                continue;
            }
            let msg: MailMessage = serde_json::from_str(&line)
                .map_err(|e| MailboxError::Parse(e.to_string()))?;
            messages.push(msg);
        }
        Ok(messages)
    }

    fn append_jsonl(&self, path: &Path, msg: &MailMessage) -> Result<(), MailboxError> {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|e| MailboxError::Io(e.to_string()))?;
        let line = serde_json::to_string(msg)
            .map_err(|e| MailboxError::Parse(e.to_string()))?;
        writeln!(file, "{line}")
            .map_err(|e| MailboxError::Io(e.to_string()))
    }

    fn next_id(&self) -> String {
        let seq = self.counter.fetch_add(1, Ordering::SeqCst);
        format!("fs-{}-{seq}", Utc::now().timestamp_millis())
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

#[async_trait]
impl Mailbox for FsMailbox {
    async fn send(&self, agent: &str, payload: MailKind) -> Result<MailRef, MailboxError> {
        self.ensure_dir()?;
        let id = self.next_id();
        let msg = MailMessage {
            id: id.clone(),
            agent: agent.to_string(),
            sent_at: Utc::now(),
            payload,
        };
        self.append_jsonl(&self.inbox_path(), &msg)?;
        Ok(id)
    }

    async fn inbox(
        &self,
        kind_filter: Option<&str>,
        since: Option<std::time::Duration>,
    ) -> Result<Vec<MailMessage>, MailboxError> {
        let all = self.read_jsonl(&self.inbox_path())?;
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
        self.read_jsonl(&self.thread_path(thread_id))
    }
}
