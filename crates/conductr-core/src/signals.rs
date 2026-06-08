//! Agent coordination signals — the canonical protocol definitions for
//! inter-agent messaging, independent of any transport or storage adapter.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::types::IssueNumber;

/// Opaque identifier for an agent (e.g. "claude/issue-16-…" branch name or session name).
pub type AgentId = String;

/// Opaque reference to a [`Signal`] (its unique id string).
pub type MailRef = String;

/// The kind of payload carried by a [`Signal`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MailKind {
    /// An agent claiming ownership of a set of files for a given issue.
    ScopeClaim {
        issue: IssueNumber,
        files: Vec<String>,
        summary: String,
    },
    /// A request for a synthesis agent to merge two or more PRs for one issue.
    SynthesisRequest {
        issue: IssueNumber,
        pr_numbers: Vec<u64>,
    },
    /// A proposed merged solution produced by a synthesis agent.
    SynthesisProposal {
        issue: IssueNumber,
        request_id: MailRef,
        body: String,
    },
    /// An informational note from an agent (free-form).
    Note { text: String },
    /// Emitted by a FERAL-preset routine when a potential post-merge conflict
    /// is detected with a sibling branch. Structured so consumers can route,
    /// deduplicate, or alert on it without parsing free-text logs.
    Yell {
        /// The issue whose routine detected the conflict.
        issue: IssueNumber,
        /// Head ref (branch name) of the sibling branch in conflict.
        sibling_branch: String,
        /// Short human-readable reason (e.g. "sibling branch has failing CI").
        message: String,
    },
}

/// A single signal on the shared bulletin board.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MailMessage {
    pub id: MailRef,
    pub agent: AgentId,
    pub sent_at: DateTime<Utc>,
    pub payload: MailKind,
}
