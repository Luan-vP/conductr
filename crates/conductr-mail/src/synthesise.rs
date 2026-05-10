//! Synthesis substrate: post a `SynthesisRequest` to the mailbox and return
//! its id so a downstream agent can pick it up and produce a `SynthesisProposal`.

use conductr_core::ports::{Mailbox, MailboxError};
use conductr_core::types::{IssueNumber, MailKind, MailRef};

/// Post a `SynthesisRequest` for `issue` covering `pr_numbers`.
///
/// Returns the `MailRef` (message id) of the request so the caller can
/// correlate future `SynthesisProposal` messages.
pub async fn request_synthesis(
    mailbox: &impl Mailbox,
    agent: &str,
    issue: IssueNumber,
    pr_numbers: Vec<u64>,
) -> Result<MailRef, MailboxError> {
    mailbox
        .send(agent, MailKind::SynthesisRequest { issue, pr_numbers })
        .await
}
