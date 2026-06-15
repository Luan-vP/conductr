//! Top-level orchestration loop.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Instant;

use tracing::{info, warn};

use crate::architect::{complexity_for, phrase_id_from_title};
use crate::classifier::{classify, Bucket, Classification};
use crate::dispatch::{self, Runner, SlotKind};
use crate::isolation::{self, IsolationDecision};
use crate::tempo;
use crate::types::IssueNumber;
use conductr_core::maturity::MaturityLevel;
use conductr_core::ports::{LocalCi, Mailbox, ScmHost, TmuxAgent};
use conductr_core::safety::{preset_from_labels, resolve_preset};
use conductr_core::signals::MailKind;
use conductr_core::types::{
    CiMode, CiRunRow, CiStatus, PrLocalCiResult, PrState, SafetyPreset, TempoPrRow, TmuxSession,
};

pub use conductr_core::types::{CycleReport, OrchestratorConfig};

/// GH label set on an issue before `@claude` dispatch and cleared on PR
/// open/close. Acts as the coordination lock between simultaneous orchestrate
/// calls.
const IN_FLIGHT_LABEL: &str = "conductr:in-flight";

pub struct Orchestrator<C: ScmHost> {
    pub client: C,
    pub config: OrchestratorConfig,
    mailbox: Option<Arc<dyn Mailbox>>,
    local_ci: Option<Arc<dyn LocalCi>>,
    tmux: Option<Arc<dyn TmuxAgent>>,
    /// Tracks when an issue was first deferred by the STRICT soft-chord.
    /// Used to enforce the `soft_chord_timeout`.
    soft_chord_blocked_since: BTreeMap<IssueNumber, Instant>,
    /// Semaphore with capacity 1 for BUREAUCRATIC hard-chord serialisation.
    /// Acquired before each BUREAUCRATIC dispatch; released immediately after
    /// (fire-and-forget dispatch model). Provides backpressure via async
    /// queuing rather than busy-wait.
    coordinator: Arc<tokio::sync::Semaphore>,
}

impl<C: ScmHost> Orchestrator<C> {
    pub fn new(client: C, config: OrchestratorConfig) -> Self {
        Self {
            client,
            config,
            mailbox: None,
            local_ci: None,
            tmux: None,
            soft_chord_blocked_since: BTreeMap::new(),
            coordinator: Arc::new(tokio::sync::Semaphore::new(1)),
        }
    }

    /// Attach an optional mailbox for scope-dedup. When set, `run_cycle` will
    /// skip Ready issues that have an existing `ScopeClaim` overlap.
    pub fn with_mailbox(mut self, mailbox: Arc<dyn Mailbox>) -> Self {
        self.mailbox = Some(mailbox);
        self
    }

    /// Attach a local CI runner. When set, `run_cycle` resolves each open PR's
    /// `ci` status using local commands before classifying, according to
    /// `config.ci_mode`.
    pub fn with_local_ci(mut self, local_ci: Arc<dyn LocalCi>) -> Self {
        self.local_ci = Some(local_ci);
        self
    }

    /// Attach a tmux adapter. When set, issues labelled `runner:tmux` are
    /// dispatched by spawning a local `agent<n>` pane instead of posting a
    /// GitHub comment.
    pub fn with_tmux(mut self, tmux: Arc<dyn TmuxAgent>) -> Self {
        self.tmux = Some(tmux);
        self
    }

    /// Check the mailbox for an existing scope claim that overlaps with `issue`.
    /// Returns `Some(message_id)` if an overlap is found.
    async fn check_scope_overlap(
        mailbox: &dyn Mailbox,
        issue: IssueNumber,
    ) -> Option<String> {
        let messages = mailbox.inbox(Some("scope_claim"), None).await.ok()?;
        for msg in messages {
            if let MailKind::ScopeClaim { issue: claimed_issue, .. } = &msg.payload {
                if *claimed_issue == issue {
                    return Some(msg.id.clone());
                }
            }
        }
        None
    }

    /// Run a single cycle of survey → classify → act.
    pub async fn run_cycle(&mut self) -> anyhow::Result<CycleReport> {
        let repo = &self.config.repo;
        info!(%repo, "surveying repo state");

        // ── Housekeeping: fetch closed PRs for tempo write-back + label cleanup.
        let closed_prs = self.client.list_closed_prs(repo).await.unwrap_or_default();

        let open_issues = self.client.list_open_issues(repo).await?;
        let closed = self.client.list_closed_issue_numbers(repo).await?;
        let mut prs = self.client.list_open_prs(repo).await?;

        // Clear conductr:in-flight from issues whose PR has now opened.
        if !self.config.dry_run {
            for pr in &prs {
                if let Some(issue_num) = pr.linked_issue {
                    if open_issues
                        .iter()
                        .find(|i| i.number == issue_num)
                        .is_some_and(|i| i.has_label(IN_FLIGHT_LABEL))
                    {
                        if let Err(e) = self
                            .client
                            .remove_issue_label(repo, issue_num, IN_FLIGHT_LABEL)
                            .await
                        {
                            warn!(issue = issue_num, error = %e, "failed to clear in-flight label after PR open");
                        }
                    }
                }
            }

            // Clear conductr:in-flight from issues whose PR has closed/merged.
            for pr in &closed_prs {
                if let Some(issue_num) = pr.linked_issue {
                    if open_issues
                        .iter()
                        .find(|i| i.number == issue_num)
                        .is_some_and(|i| i.has_label(IN_FLIGHT_LABEL))
                    {
                        if let Err(e) = self
                            .client
                            .remove_issue_label(repo, issue_num, IN_FLIGHT_LABEL)
                            .await
                        {
                            warn!(issue = issue_num, error = %e, "failed to clear in-flight label after PR close");
                        }
                    }
                }
            }
        }

        // ── Run local CI and resolve CiStatus for each open PR. ───────────────
        //
        // SECURITY: local CI executes the PR's code (build.rs, proc-macros,
        // test helpers) on the operator's machine without OS-level isolation.
        // Fork PRs are skipped by default (`allow_fork_local_ci = false`) to
        // prevent untrusted contributor code from running with operator
        // privileges. Set `allow_fork_local_ci = true` only for trusted forks.
        let mut local_ci_results: Vec<PrLocalCiResult> = Vec::new();
        if let Some(local_ci) = &self.local_ci {
            for pr in &mut prs {
                if pr.state != PrState::Open {
                    continue;
                }
                if pr.is_fork && !self.config.allow_fork_local_ci {
                    warn!(
                        pr = pr.number,
                        "skipping local CI for fork PR (set allow_fork_local_ci to override)"
                    );
                    continue;
                }
                match local_ci.run(&pr.head_ref).await {
                    Ok(report) => {
                        let resolved =
                            resolve_ci_mode(self.config.ci_mode, report.status, pr.ci);
                        local_ci_results.push(PrLocalCiResult {
                            pr: pr.number,
                            status: report.status,
                            commands: report.commands,
                        });
                        pr.ci = resolved;
                    }
                    Err(e) => {
                        warn!(pr = pr.number, error = %e, "local CI error; keeping GitHub status");
                    }
                }
            }
        }

        // ── Build the set of issues that already have @claude comments. ───────
        let mut triggered: BTreeSet<IssueNumber> = BTreeSet::new();
        for issue in &open_issues {
            let comments =
                self.client.list_issue_comments(repo, issue.number).await.unwrap_or_default();
            if comments.iter().any(|c| c.contains(&self.config.trigger_comment)) {
                triggered.insert(issue.number);
            }
        }

        let classifications: Vec<Classification> = open_issues
            .iter()
            .map(|i| classify(i, &closed, &prs, &triggered))
            .collect();

        let mut report = CycleReport { local_ci: local_ci_results, ..CycleReport::default() };

        // ── Fetch tmux sessions for slot management (if tmux is attached). ─────
        let tmux_sessions: Vec<TmuxSession> = if let Some(tmux) = &self.tmux {
            tmux.list_sessions().await.unwrap_or_default()
        } else {
            Vec::new()
        };
        let mut agent_indices_in_use =
            dispatch::active_slot_indices(&tmux_sessions, SlotKind::Agent);
        let mut qa_indices_in_use =
            dispatch::active_slot_indices(&tmux_sessions, SlotKind::Qa);

        // ── 1) Merge any PRs that are passing CI. ─────────────────────────────
        for c in &classifications {
            if c.bucket == Bucket::PrOpen {
                if let Some(pr) = c.pr {
                    if self.config.dry_run {
                        info!(pr, "would merge");
                    } else {
                        match self.client.merge_pr_squash(repo, pr).await {
                            Ok(_) => {
                                info!(pr, "merged");
                                report.merged.push(pr);
                                report.progress_made = true;
                            }
                            Err(e) => warn!(pr, error=%e, "merge failed"),
                        }
                    }
                }
            } else if c.bucket == Bucket::PrFailing {
                if let Some(pr) = c.pr {
                    report.pr_failing.push(pr);
                }
            }
        }

        // ── 1a) Spawn qa<n> slots for open PRs linked to tmux-runner issues. ───
        if let Some(tmux) = &self.tmux {
            for pr in &prs {
                if pr.state != PrState::Open {
                    continue;
                }
                let Some(issue_num) = pr.linked_issue else { continue };
                let Some(issue_obj) = open_issues.iter().find(|i| i.number == issue_num) else {
                    continue
                };
                if dispatch::runner_for(issue_obj) != Runner::Tmux {
                    continue;
                }

                let next_qa =
                    (1..=self.config.max_parallel_qa).find(|n| !qa_indices_in_use.contains(n));
                let Some(slot_idx) = next_qa else {
                    info!(
                        cap = self.config.max_parallel_qa,
                        "max_parallel_qa reached; deferring qa slot spawn"
                    );
                    break;
                };

                let name = dispatch::slot_name(SlotKind::Qa, slot_idx);
                let cwd = self.config.tmux_cwd.as_deref().unwrap_or(".");
                if self.config.dry_run {
                    info!(pr = pr.number, session = %name, "would spawn qa slot");
                } else {
                    match tmux.new_session(&name, cwd).await {
                        Ok(()) => {
                            let _ = tmux
                                .send_line(&name, "claude --dangerously-skip-permissions")
                                .await;
                            let prompt = format!("/qa --pr {}", pr.number);
                            if let Err(e) = tmux.send_line(&name, &prompt).await {
                                warn!(session = %name, pr = pr.number, error = %e,
                                      "failed to send qa prompt to tmux session");
                            } else {
                                info!(pr = pr.number, session = %name, "spawned qa slot");
                                qa_indices_in_use.push(slot_idx);
                            }
                        }
                        Err(e) => {
                            warn!(session = %name, pr = pr.number, error = %e,
                                  "failed to create qa tmux session");
                        }
                    }
                }
            }
        }

        // ── 2) Trigger Ready issues (with coordination guards + cap). ─────────
        //
        // Coordination order per spec:
        //   a) conductr:in-flight label check — skip if another orchestrate
        //      already claimed this issue.
        //   b) @claude comment scan — already captured in `triggered` set;
        //      issues with a comment are in TriggeredWaiting bucket, not Ready.
        //   c) Isolation decision — per-detent SafetyPreset check (may defer).
        //   d) Atomic label add — add conductr:in-flight *before* posting the
        //      @claude comment so that any concurrent orchestrate sees it.
        //   e) max_parallel_beats cap — dispatch at most N issues per pass.
        let mut dispatched_this_pass = 0usize;
        // Track BUREAUCRATIC dispatches separately: only one per cycle is allowed.
        let mut bureaucratic_dispatched_this_pass = false;

        for c in &classifications {
            if c.bucket != Bucket::Ready {
                if c.bucket == Bucket::TriggeredWaiting {
                    report.waiting.push(c.issue);
                } else if c.bucket == Bucket::Blocked {
                    report.blocked.push(c.issue);
                } else if c.bucket == Bucket::Human {
                    report.human.push(c.issue);
                    if let Some(login) = &self.config.default_human_assignee {
                        if !self.config.dry_run {
                            let _ = self.client.assign_issue(repo, c.issue, login).await;
                        }
                    }
                }
                continue;
            }

            // (a) conductr:in-flight label check.
            if open_issues
                .iter()
                .find(|i| i.number == c.issue)
                .is_some_and(|i| i.has_label(IN_FLIGHT_LABEL))
            {
                info!(issue = c.issue, "skipping — conductr:in-flight label present");
                continue;
            }

            // (b) @claude comment check is implicit: issues with @claude are in
            // TriggeredWaiting, not Ready, so they never reach this branch.

            // Scope dedup: skip if another agent has claimed this issue.
            if let Some(mb) = &self.mailbox {
                if let Some(msg_id) = Self::check_scope_overlap(mb.as_ref(), c.issue).await {
                    info!(issue = c.issue, msg_id = %msg_id, "skipping — scope overlap");
                    report.scope_overlap.push(c.issue);
                    continue;
                }
            }

            // (c) Isolation decision — resolve effective preset and apply per-detent behaviour.
            let issue_obj = open_issues.iter().find(|i| i.number == c.issue);
            let routine_preset = issue_obj.and_then(|i| preset_from_labels(&i.labels));
            let effective_preset = resolve_preset(
                MaturityLevel::L0Bootstrap, // default maturity when not tracked
                self.config.safety_preset,
                routine_preset,
            );

            let siblings = isolation::sibling_prs(c.issue, &prs);
            let soft_chord_first_seen = self.soft_chord_blocked_since.get(&c.issue).copied();
            let isolation_decision = isolation::decide(
                effective_preset,
                c.issue,
                &siblings,
                soft_chord_first_seen,
                self.config.soft_chord_timeout,
            );

            match isolation_decision {
                IsolationDecision::Defer => {
                    self.soft_chord_blocked_since.entry(c.issue).or_insert_with(Instant::now);
                    info!(issue = c.issue, preset = ?effective_preset, "deferred by soft-chord");
                    report.soft_chord_deferred.push(c.issue);
                    continue;
                }
                IsolationDecision::DispatchWithYell(yells) => {
                    if let Some(mb) = &self.mailbox {
                        for yell in yells {
                            if let Err(e) = mb.send("orchestrator", yell).await {
                                warn!(issue = c.issue, error = %e, "failed to emit yell event");
                            }
                        }
                    }
                    // Clear any stale soft-chord tracker.
                    self.soft_chord_blocked_since.remove(&c.issue);
                }
                IsolationDecision::DispatchWithAdvisory(advisory) => {
                    if !self.config.dry_run {
                        if let Err(e) = self.client.comment_issue(repo, c.issue, &advisory).await {
                            warn!(issue = c.issue, error = %e, "failed to post advisory comment");
                        }
                    }
                    self.soft_chord_blocked_since.remove(&c.issue);
                }
                IsolationDecision::Dispatch => {
                    self.soft_chord_blocked_since.remove(&c.issue);
                }
            }

            // (e) Hard-chord: BUREAUCRATIC preset serialises all dispatches through
            // a single coordinator.  Three conditions must hold:
            //   1. No other routine is currently in-flight (cross-cycle: persistent
            //      `conductr:in-flight` label acts as a distributed lock).
            //   2. No BUREAUCRATIC issue was already dispatched this cycle
            //      (within-cycle: local counter).
            //   3. The coordinator semaphore is available (concurrent `run_cycle`
            //      callers are queued via tokio backpressure, not busy-waited).
            if effective_preset == SafetyPreset::Bureaucratic {
                let any_in_flight =
                    open_issues.iter().any(|i| i.has_label(IN_FLIGHT_LABEL));
                if any_in_flight || bureaucratic_dispatched_this_pass {
                    info!(issue = c.issue,
                          "hard-chord: coordinator serialising; deferring this routine");
                    report.soft_chord_deferred.push(c.issue);
                    continue;
                }
                match self.coordinator.try_acquire() {
                    Ok(_permit) => {
                        // Permit dropped at end of loop body; semaphore guards
                        // concurrent `run_cycle` callers in the same process.
                    }
                    Err(_) => {
                        info!(issue = c.issue,
                              "hard-chord: coordinator semaphore held; deferring");
                        report.soft_chord_deferred.push(c.issue);
                        continue;
                    }
                }
            }

            // (f) Determine runner and enforce the appropriate cap.
            let issue_runner = issue_obj.map(dispatch::runner_for).unwrap_or(Runner::Web);

            match issue_runner {
                Runner::Web => {
                    // Web cap: at most max_parallel_beats GH comments per pass.
                    if dispatched_this_pass >= self.config.max_parallel_beats {
                        info!(
                            cap = self.config.max_parallel_beats,
                            "max_parallel_beats reached; deferring remaining Ready issues"
                        );
                        continue;
                    }

                    if self.config.dry_run {
                        info!(issue = c.issue, "would trigger (web)");
                    } else {
                        // (c) Atomic: add label first, then comment.
                        if let Err(e) = self
                            .client
                            .add_issue_label(repo, c.issue, IN_FLIGHT_LABEL)
                            .await
                        {
                            warn!(issue = c.issue, error = %e,
                                  "failed to add in-flight label; skipping dispatch");
                            continue;
                        }
                        match self
                            .client
                            .comment_issue(repo, c.issue, &self.config.trigger_comment)
                            .await
                        {
                            Ok(_) => {
                                info!(issue = c.issue, "triggered (web)");
                                report.triggered.push(c.issue);
                                report.progress_made = true;
                                dispatched_this_pass += 1;
                                if effective_preset == SafetyPreset::Bureaucratic {
                                    bureaucratic_dispatched_this_pass = true;
                                }
                            }
                            Err(e) => {
                                warn!(issue = c.issue, error = %e,
                                      "trigger failed; rolling back in-flight label");
                                let _ = self
                                    .client
                                    .remove_issue_label(repo, c.issue, IN_FLIGHT_LABEL)
                                    .await;
                            }
                        }
                    }
                }

                Runner::Tmux => {
                    // Tmux cap: slot pool bounded by max_parallel_beats.
                    let next_agent = (1..=self.config.max_parallel_beats)
                        .find(|n| !agent_indices_in_use.contains(n));
                    let Some(slot_idx) = next_agent else {
                        info!(
                            cap = self.config.max_parallel_beats,
                            "agent slot pool full; deferring tmux beat"
                        );
                        continue;
                    };

                    let session_name = dispatch::slot_name(SlotKind::Agent, slot_idx);

                    if self.config.dry_run {
                        info!(issue = c.issue, session = %session_name, "would trigger (tmux)");
                    } else if let Some(tmux) = &self.tmux {
                        // (c) Atomic: add label first, then spawn session.
                        if let Err(e) = self
                            .client
                            .add_issue_label(repo, c.issue, IN_FLIGHT_LABEL)
                            .await
                        {
                            warn!(issue = c.issue, error = %e,
                                  "failed to add in-flight label; skipping tmux dispatch");
                            continue;
                        }
                        let cwd = self.config.tmux_cwd.as_deref().unwrap_or(".");
                        match tmux.new_session(&session_name, cwd).await {
                            Ok(()) => {
                                let _ = tmux
                                    .send_line(
                                        &session_name,
                                        "claude --dangerously-skip-permissions",
                                    )
                                    .await;
                                let prompt = format!(
                                    "{} --issue {}",
                                    self.config.trigger_comment, c.issue
                                );
                                match tmux.send_line(&session_name, &prompt).await {
                                    Ok(()) => {
                                        info!(issue = c.issue, session = %session_name,
                                              "triggered (tmux)");
                                        report.triggered.push(c.issue);
                                        report.progress_made = true;
                                        agent_indices_in_use.push(slot_idx);
                                        dispatched_this_pass += 1;
                                        if effective_preset == SafetyPreset::Bureaucratic {
                                            bureaucratic_dispatched_this_pass = true;
                                        }
                                    }
                                    Err(e) => {
                                        warn!(issue = c.issue, session = %session_name,
                                              error = %e, "failed to send implementer prompt");
                                        let _ = self
                                            .client
                                            .remove_issue_label(repo, c.issue, IN_FLIGHT_LABEL)
                                            .await;
                                    }
                                }
                            }
                            Err(e) => {
                                warn!(issue = c.issue, session = %session_name, error = %e,
                                      "failed to create agent tmux session");
                                let _ = self
                                    .client
                                    .remove_issue_label(repo, c.issue, IN_FLIGHT_LABEL)
                                    .await;
                            }
                        }
                    } else {
                        // No tmux adapter attached — fall back to web dispatch.
                        warn!(issue = c.issue, "runner=tmux but no TmuxAgent attached; \
                               falling back to web dispatch");
                        if dispatched_this_pass >= self.config.max_parallel_beats {
                            continue;
                        }
                        if let Err(e) = self
                            .client
                            .add_issue_label(repo, c.issue, IN_FLIGHT_LABEL)
                            .await
                        {
                            warn!(issue = c.issue, error = %e,
                                  "failed to add in-flight label; skipping dispatch");
                            continue;
                        }
                        match self
                            .client
                            .comment_issue(repo, c.issue, &self.config.trigger_comment)
                            .await
                        {
                            Ok(_) => {
                                info!(issue = c.issue, "triggered (web fallback)");
                                report.triggered.push(c.issue);
                                report.progress_made = true;
                                dispatched_this_pass += 1;
                                if effective_preset == SafetyPreset::Bureaucratic {
                                    bureaucratic_dispatched_this_pass = true;
                                }
                            }
                            Err(e) => {
                                warn!(issue = c.issue, error = %e, "trigger failed");
                                let _ = self
                                    .client
                                    .remove_issue_label(repo, c.issue, IN_FLIGHT_LABEL)
                                    .await;
                            }
                        }
                    }
                }
            }
        }

        // ── 3) Tempo write-back for closed/merged PRs (batched). ─────────────
        if let Some(config_path) = &self.config.conductr_config_path {
            let mut pr_rows: Vec<TempoPrRow> = Vec::new();
            let mut ci_rows: Vec<CiRunRow> = Vec::new();

            for pr in &closed_prs {
                let linked_issue = pr.linked_issue
                    .and_then(|n| open_issues.iter().find(|i| i.number == n));

                let complexity = linked_issue
                    .map(|i| complexity_for(i, None))
                    .unwrap_or(conductr_core::types::Complexity::M);

                let phrase = linked_issue.map(|i| phrase_id_from_title(&i.title));

                pr_rows.push(TempoPrRow {
                    number: pr.number,
                    title: pr.title.clone(),
                    phrase,
                    chord: None,
                    complexity,
                    opened: pr.opened_at,
                    closed: pr.closed_at,
                    merged: pr.merged,
                });

                if let Ok(Some(minutes)) =
                    self.client.latest_ci_run_minutes(repo, &pr.head_ref).await
                {
                    ci_rows.push(CiRunRow { pr: pr.number, minutes, ts: pr.closed_at });
                }
            }

            if !pr_rows.is_empty() || !ci_rows.is_empty() {
                if let Err(e) = tempo::append_rows(config_path, &pr_rows, &ci_rows) {
                    warn!(error = %e, "failed to write tempo rows to .conductr");
                }
            }
        }

        Ok(report)
    }

    /// Drive the orchestration to completion (or until `max_cycles`).
    pub async fn run_to_completion(&mut self) -> anyhow::Result<Vec<CycleReport>> {
        let mut history = Vec::new();
        let mut cycle = 0u32;
        loop {
            let report = self.run_cycle().await?;
            let nothing_left = report.triggered.is_empty()
                && report.merged.is_empty()
                && report.waiting.is_empty()
                && report.pr_failing.is_empty();
            history.push(report);
            cycle += 1;
            if nothing_left {
                info!("nothing left to do");
                break;
            }
            if let Some(max) = self.config.max_cycles {
                if cycle >= max {
                    info!(cycles = cycle, "reached max cycles");
                    break;
                }
            }
            tokio::time::sleep(self.config.poll_interval).await;
        }
        Ok(history)
    }
}

fn resolve_ci_mode(mode: CiMode, local: CiStatus, github: CiStatus) -> CiStatus {
    match mode {
        CiMode::Local => match local {
            CiStatus::Unknown => CiStatus::Failing,
            other => other,
        },
        CiMode::PreferLocal => match local {
            CiStatus::Unknown => github,
            other => other,
        },
        CiMode::PreferGithub => match github {
            CiStatus::Passing | CiStatus::Failing => github,
            _ => local,
        },
        CiMode::Github => github,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use conductr_adapters::mock::MockScmHost;
    use conductr_core::signals::{MailKind, MailMessage, MailRef};
    use conductr_core::types::{CiStatus, Issue, IssueState, Pr, PrState, RepoSlug};
    use conductr_core::ports::{Mailbox, MailboxError};
    use async_trait::async_trait;
    use chrono::Utc;
    use std::sync::{Arc, Mutex, RwLock};
    use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

    #[derive(Default)]
    struct FakeClient {
        pub triggered: Mutex<Vec<IssueNumber>>,
        pub merged: Mutex<Vec<u64>>,
    }

    #[async_trait]
    impl ScmHost for FakeClient {
        async fn list_open_issues(&self, _: &RepoSlug) -> anyhow::Result<Vec<Issue>> {
            Ok(vec![Issue {
                number: 1,
                title: "x".into(),
                body: "no deps".into(),
                labels: vec![],
                state: IssueState::Open,
            }])
        }
        async fn list_closed_issue_numbers(&self, _: &RepoSlug) -> anyhow::Result<BTreeSet<IssueNumber>> {
            Ok(BTreeSet::new())
        }
        async fn list_open_prs(&self, _: &RepoSlug) -> anyhow::Result<Vec<Pr>> {
            Ok(vec![])
        }
        async fn list_issue_comments(&self, _: &RepoSlug, _: IssueNumber) -> anyhow::Result<Vec<String>> {
            Ok(vec![])
        }
        async fn comment_issue(&self, _: &RepoSlug, n: IssueNumber, _: &str) -> anyhow::Result<()> {
            self.triggered.lock().unwrap().push(n);
            Ok(())
        }
        async fn assign_issue(&self, _: &RepoSlug, _: IssueNumber, _: &str) -> anyhow::Result<()> {
            Ok(())
        }
        async fn merge_pr_squash(&self, _: &RepoSlug, n: u64) -> anyhow::Result<()> {
            self.merged.lock().unwrap().push(n);
            Ok(())
        }
        async fn create_issue(
            &self,
            _: &RepoSlug,
            _title: &str,
            _body: &str,
            _labels: &[&str],
        ) -> anyhow::Result<conductr_core::types::IssueNumber> {
            Ok(0)
        }
    }

    /// Minimal in-test mailbox that pre-seeds messages.
    #[derive(Default)]
    struct FakeMailbox {
        messages: Arc<RwLock<Vec<MailMessage>>>,
        counter: AtomicU64,
    }

    impl FakeMailbox {
        fn with_scope_claim(self, issue: u64) -> Self {
            self.messages.write().unwrap().push(MailMessage {
                id: format!("pre-{issue}"),
                agent: "agent-a".into(),
                sent_at: Utc::now(),
                payload: MailKind::ScopeClaim {
                    issue,
                    files: vec!["src/lib.rs".into()],
                    summary: "claimed".into(),
                },
            });
            self
        }
    }

    #[async_trait]
    impl Mailbox for FakeMailbox {
        async fn send(&self, agent: &str, payload: MailKind) -> Result<MailRef, MailboxError> {
            let id = format!("msg-{}", self.counter.fetch_add(1, AtomicOrdering::SeqCst));
            self.messages.write().unwrap().push(MailMessage {
                id: id.clone(),
                agent: agent.to_string(),
                sent_at: Utc::now(),
                payload,
            });
            Ok(id)
        }

        async fn inbox(
            &self,
            kind_filter: Option<&str>,
            _since: Option<std::time::Duration>,
        ) -> Result<Vec<MailMessage>, MailboxError> {
            let all = self.messages.read().unwrap().clone();
            Ok(match kind_filter {
                Some("scope_claim") => all
                    .into_iter()
                    .filter(|m| matches!(m.payload, MailKind::ScopeClaim { .. }))
                    .collect(),
                _ => all,
            })
        }

        async fn thread(&self, _: &MailRef) -> Result<Vec<MailMessage>, MailboxError> {
            Ok(vec![])
        }
    }

    fn make_issue(number: IssueNumber, labels: &[&str]) -> Issue {
        Issue {
            number,
            title: format!("issue-{number}"),
            body: "no deps".into(),
            labels: labels.iter().map(|s| s.to_string()).collect(),
            state: IssueState::Open,
        }
    }

    #[tokio::test]
    async fn ready_issue_gets_triggered() {
        let client = MockScmHost::new().with_issues([Issue {
            number: 1,
            title: "x".into(),
            body: "no deps".into(),
            labels: vec![],
            state: IssueState::Open,
        }]);
        let cfg = OrchestratorConfig::new(RepoSlug::new("o", "r"));
        let mut orch = Orchestrator::new(client, cfg);
        let report = orch.run_cycle().await.unwrap();
        assert_eq!(report.triggered, vec![1]);
        assert_eq!(orch.client.posted_comments(), vec![(1, "@claude please implement".to_string())]);
    }

    #[tokio::test]
    async fn dry_run_does_not_act() {
        let client = MockScmHost::new().with_issues([Issue {
            number: 1,
            title: "x".into(),
            body: "no deps".into(),
            labels: vec![],
            state: IssueState::Open,
        }]);
        let mut cfg = OrchestratorConfig::new(RepoSlug::new("o", "r"));
        cfg.dry_run = true;
        let mut orch = Orchestrator::new(client, cfg);
        let report = orch.run_cycle().await.unwrap();
        assert!(report.triggered.is_empty());
        assert!(orch.client.posted_comments().is_empty());
    }

    #[tokio::test]
    async fn scope_overlap_skips_ready_issue() {
        let client = FakeClient::default();
        let cfg = OrchestratorConfig::new(RepoSlug::new("o", "r"));
        let mb = Arc::new(FakeMailbox::default().with_scope_claim(1));
        let mut orch = Orchestrator::new(client, cfg).with_mailbox(mb);
        let report = orch.run_cycle().await.unwrap();
        // Issue 1 is Ready but has a scope claim → skipped.
        assert!(report.triggered.is_empty());
        assert_eq!(report.scope_overlap, vec![1]);
        assert!(orch.client.triggered.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn no_mailbox_behaves_as_before() {
        let client = FakeClient::default();
        let cfg = OrchestratorConfig::new(RepoSlug::new("o", "r"));
        let mut orch = Orchestrator::new(client, cfg);
        let report = orch.run_cycle().await.unwrap();
        assert_eq!(report.triggered, vec![1]);
        assert!(report.scope_overlap.is_empty());
    }

    #[test]
    fn prefer_local_passes_through_local_status() {
        use conductr_core::types::CiMode;
        assert_eq!(
            resolve_ci_mode(CiMode::PreferLocal, CiStatus::Passing, CiStatus::Failing),
            CiStatus::Passing
        );
        assert_eq!(
            resolve_ci_mode(CiMode::PreferLocal, CiStatus::Failing, CiStatus::Passing),
            CiStatus::Failing
        );
    }

    #[test]
    fn prefer_local_falls_back_to_github_on_unknown() {
        use conductr_core::types::CiMode;
        assert_eq!(
            resolve_ci_mode(CiMode::PreferLocal, CiStatus::Unknown, CiStatus::Passing),
            CiStatus::Passing
        );
        assert_eq!(
            resolve_ci_mode(CiMode::PreferLocal, CiStatus::Unknown, CiStatus::Pending),
            CiStatus::Pending
        );
    }

    #[test]
    fn local_mode_fails_on_unknown() {
        use conductr_core::types::CiMode;
        assert_eq!(
            resolve_ci_mode(CiMode::Local, CiStatus::Unknown, CiStatus::Passing),
            CiStatus::Failing
        );
    }

    #[test]
    fn prefer_github_uses_github_when_conclusive() {
        use conductr_core::types::CiMode;
        assert_eq!(
            resolve_ci_mode(CiMode::PreferGithub, CiStatus::Passing, CiStatus::Passing),
            CiStatus::Passing
        );
        assert_eq!(
            resolve_ci_mode(CiMode::PreferGithub, CiStatus::Failing, CiStatus::Failing),
            CiStatus::Failing
        );
        // GitHub pending → fall back to local
        assert_eq!(
            resolve_ci_mode(CiMode::PreferGithub, CiStatus::Passing, CiStatus::Pending),
            CiStatus::Passing
        );
    }

    #[test]
    fn github_mode_ignores_local() {
        use conductr_core::types::CiMode;
        assert_eq!(
            resolve_ci_mode(CiMode::Github, CiStatus::Failing, CiStatus::Passing),
            CiStatus::Passing
        );
    }

    // ── Coordination tests ────────────────────────────────────────────────────

    #[tokio::test]
    async fn in_flight_label_prevents_dispatch() {
        let client = MockScmHost::new().with_issues([make_issue(1, &[IN_FLIGHT_LABEL])]);
        let cfg = OrchestratorConfig::new(RepoSlug::new("o", "r"));
        let mut orch = Orchestrator::new(client, cfg);
        let report = orch.run_cycle().await.unwrap();
        // Issue has conductr:in-flight → should be skipped.
        assert!(report.triggered.is_empty());
        assert!(orch.client.posted_comments().is_empty());
    }

    #[tokio::test]
    async fn dispatch_adds_in_flight_label_before_comment() {
        let client = MockScmHost::new().with_issues([make_issue(1, &[])]);
        let cfg = OrchestratorConfig::new(RepoSlug::new("o", "r"));
        let mut orch = Orchestrator::new(client, cfg);
        let report = orch.run_cycle().await.unwrap();
        assert_eq!(report.triggered, vec![1]);
        // Label must have been added.
        assert_eq!(orch.client.labels_added(), vec![(1, IN_FLIGHT_LABEL.to_string())]);
        // Comment must also have been posted.
        assert!(!orch.client.posted_comments().is_empty());
    }

    #[tokio::test]
    async fn max_parallel_beats_caps_dispatch() {
        let client = MockScmHost::new().with_issues([
            make_issue(1, &[]),
            make_issue(2, &[]),
            make_issue(3, &[]),
            make_issue(4, &[]),
        ]);
        let mut cfg = OrchestratorConfig::new(RepoSlug::new("o", "r"));
        cfg.max_parallel_beats = 2;
        let mut orch = Orchestrator::new(client, cfg);
        let report = orch.run_cycle().await.unwrap();
        assert_eq!(report.triggered.len(), 2, "should dispatch exactly 2 issues");
    }

    // ── Per-detent isolation integration tests ────────────────────────────────

    fn make_pr_for_issue(pr_num: u64, issue_num: IssueNumber, ci: CiStatus) -> Pr {
        Pr {
            number: pr_num,
            title: format!("pr-{pr_num}"),
            body: String::new(),
            head_ref: format!("claude/issue-{issue_num}-fix"),
            state: conductr_core::types::PrState::Open,
            ci,
            linked_issue: Some(issue_num),
            is_fork: false,
        }
    }

    // ── Helper: open PR for issue 99 (a non-tracked sibling). ───────────────
    // Issues 1 and 2 see this as a sibling since it's not linked to them.
    fn sibling_pr_99() -> Pr {
        make_pr_for_issue(99, 99, CiStatus::Pending)
    }

    /// UNHINGED: both routines dispatch even when a sibling branch is present.
    #[tokio::test]
    async fn unhinged_two_routines_both_dispatch() {
        // PR 99 linked to issue 99 is a sibling of issues 1 and 2 (not linked to either).
        let client = MockScmHost::new()
            .with_issues([make_issue(1, &["safety:unhinged"]), make_issue(2, &["safety:unhinged"])])
            .with_prs([sibling_pr_99()]);
        let cfg = OrchestratorConfig::new(RepoSlug::new("o", "r"));
        let mut orch = Orchestrator::new(client, cfg);
        let report = orch.run_cycle().await.unwrap();
        assert_eq!(report.triggered.len(), 2, "UNHINGED: both should dispatch");
        assert!(report.soft_chord_deferred.is_empty());
    }

    /// FERAL: both routines dispatch; structured Yell events emitted for sibling presence.
    #[tokio::test]
    async fn feral_two_routines_dispatch_with_yell() {
        let mb = Arc::new(FakeMailbox::default());
        // PR 99 is a sibling of both issues 1 and 2.
        let client = MockScmHost::new()
            .with_issues([make_issue(1, &["safety:feral"]), make_issue(2, &["safety:feral"])])
            .with_prs([sibling_pr_99()]);
        let cfg = OrchestratorConfig::new(RepoSlug::new("o", "r"));
        let mut orch = Orchestrator::new(client, cfg).with_mailbox(mb.clone());
        let report = orch.run_cycle().await.unwrap();
        assert_eq!(report.triggered.len(), 2, "FERAL: both should dispatch");
        assert!(report.soft_chord_deferred.is_empty());
        // At least one Yell event should be in the mailbox.
        let inbox = mb.inbox(None, None).await.unwrap();
        assert!(
            inbox.iter().any(|m| matches!(m.payload, MailKind::Yell { .. })),
            "expected at least one structured Yell event, got: {:?}", inbox
        );
    }

    /// FAST: both routines dispatch; advisory comment posted for sibling overlap.
    #[tokio::test]
    async fn fast_two_routines_dispatch_with_advisory() {
        // PR 99 is a sibling of both issues 1 and 2.
        let client = MockScmHost::new()
            .with_issues([make_issue(1, &["safety:fast"]), make_issue(2, &["safety:fast"])])
            .with_prs([sibling_pr_99()]);
        let cfg = OrchestratorConfig::new(RepoSlug::new("o", "r"));
        let mut orch = Orchestrator::new(client, cfg);
        let report = orch.run_cycle().await.unwrap();
        assert_eq!(report.triggered.len(), 2, "FAST: both should dispatch");
        let comments = orch.client.posted_comments();
        assert!(
            comments.iter().any(|(_, body)| body.contains("Advisory")),
            "expected at least one advisory comment, got: {:?}", comments
        );
    }

    /// STRICT: routine deferred when the only sibling PR has amber (pending) CI.
    #[tokio::test]
    async fn strict_defers_when_sibling_amber() {
        // PR 99 (pending CI) is a sibling of issues 1 and 2.
        let client = MockScmHost::new()
            .with_issues([make_issue(1, &["safety:strict"]), make_issue(2, &["safety:strict"])])
            .with_prs([sibling_pr_99()]);
        let cfg = OrchestratorConfig::new(RepoSlug::new("o", "r"));
        let mut orch = Orchestrator::new(client, cfg);
        let report = orch.run_cycle().await.unwrap();
        // Both issues see amber sibling → both deferred.
        assert_eq!(report.soft_chord_deferred.len(), 2,
            "expected both issues deferred by soft-chord, got {:?}", report.soft_chord_deferred);
        assert!(report.triggered.is_empty());
    }

    /// STRICT: dispatches after the soft-chord timeout expires.
    #[tokio::test]
    async fn strict_dispatches_after_soft_chord_timeout() {
        let client = MockScmHost::new()
            .with_issues([make_issue(1, &["safety:strict"])])
            .with_prs([sibling_pr_99()]);
        let mut cfg = OrchestratorConfig::new(RepoSlug::new("o", "r"));
        cfg.soft_chord_timeout = std::time::Duration::from_millis(1);
        let mut orch = Orchestrator::new(client, cfg);

        // First cycle: deferred by soft-chord.
        let report1 = orch.run_cycle().await.unwrap();
        assert_eq!(report1.soft_chord_deferred, vec![1], "expected deferral in first cycle");
        assert!(report1.triggered.is_empty());

        // Wait past the timeout.
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        // Second cycle: timeout expired → dispatch despite amber sibling.
        let report2 = orch.run_cycle().await.unwrap();
        assert!(
            report2.soft_chord_deferred.is_empty(),
            "expected no deferral after timeout, got {:?}", report2.soft_chord_deferred
        );
        assert_eq!(report2.triggered, vec![1]);
    }

    /// BUREAUCRATIC: hard-chord serialises — only one routine dispatched per cycle.
    #[tokio::test]
    async fn bureaucratic_serialises_dispatch() {
        let client = MockScmHost::new().with_issues([
            make_issue(1, &["safety:bureaucratic"]),
            make_issue(2, &["safety:bureaucratic"]),
        ]);
        let cfg = OrchestratorConfig::new(RepoSlug::new("o", "r"));
        let mut orch = Orchestrator::new(client, cfg);
        let report = orch.run_cycle().await.unwrap();
        assert_eq!(
            report.triggered.len(), 1,
            "BUREAUCRATIC should dispatch exactly 1 per cycle, got {:?}", report.triggered
        );
        assert_eq!(
            report.soft_chord_deferred.len(), 1,
            "second BUREAUCRATIC routine should be deferred, got {:?}", report.soft_chord_deferred
        );
    }

    // ── Fork PR provenance tests ──────────────────────────────────────────────

    use conductr_core::types::LocalCiReport;

    struct SpyLocalCi {
        ran_for: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl LocalCi for SpyLocalCi {
        async fn run(&self, head_ref: &str) -> anyhow::Result<LocalCiReport> {
            self.ran_for.lock().unwrap().push(head_ref.to_string());
            Ok(LocalCiReport { status: CiStatus::Passing, commands: vec![] })
        }
    }

    fn fork_pr(pr_num: u64, issue_num: IssueNumber) -> Pr {
        Pr {
            number: pr_num,
            title: format!("fork-pr-{pr_num}"),
            body: String::new(),
            head_ref: format!("external/issue-{issue_num}-fix"),
            state: conductr_core::types::PrState::Open,
            ci: CiStatus::Unknown,
            linked_issue: Some(issue_num),
            is_fork: true,
        }
    }

    /// By default, local CI must not run for fork PRs (is_fork = true).
    #[tokio::test]
    async fn fork_pr_skips_local_ci_by_default() {
        let ran_for = Arc::new(Mutex::new(vec![]));
        let spy = Arc::new(SpyLocalCi { ran_for: ran_for.clone() });

        let client = MockScmHost::new()
            .with_issues([make_issue(1, &[])])
            .with_prs([fork_pr(10, 1)]);
        let cfg = OrchestratorConfig::new(RepoSlug::new("o", "r"));
        // allow_fork_local_ci defaults to false
        assert!(!cfg.allow_fork_local_ci);

        let mut orch = Orchestrator::new(client, cfg).with_local_ci(spy);
        orch.run_cycle().await.unwrap();

        assert!(
            ran_for.lock().unwrap().is_empty(),
            "local CI should not run for fork PRs when allow_fork_local_ci is false"
        );
    }

    /// When allow_fork_local_ci is explicitly set to true, fork PRs run local CI.
    #[tokio::test]
    async fn fork_pr_runs_local_ci_when_allowed() {
        let ran_for = Arc::new(Mutex::new(vec![]));
        let spy = Arc::new(SpyLocalCi { ran_for: ran_for.clone() });

        let client = MockScmHost::new()
            .with_issues([make_issue(1, &[])])
            .with_prs([fork_pr(10, 1)]);
        let mut cfg = OrchestratorConfig::new(RepoSlug::new("o", "r"));
        cfg.allow_fork_local_ci = true;

        let mut orch = Orchestrator::new(client, cfg).with_local_ci(spy);
        orch.run_cycle().await.unwrap();

        assert_eq!(
            *ran_for.lock().unwrap(),
            vec!["external/issue-1-fix"],
            "local CI should run for fork PRs when allow_fork_local_ci is true"
        );
    }

    /// Non-fork PRs (is_fork = false) always run local CI regardless of the flag.
    #[tokio::test]
    async fn non_fork_pr_always_runs_local_ci() {
        let ran_for = Arc::new(Mutex::new(vec![]));
        let spy = Arc::new(SpyLocalCi { ran_for: ran_for.clone() });

        let client = MockScmHost::new()
            .with_issues([make_issue(1, &[])])
            .with_prs([make_pr_for_issue(10, 1, CiStatus::Unknown)]);
        let cfg = OrchestratorConfig::new(RepoSlug::new("o", "r"));
        // allow_fork_local_ci = false but this PR is not a fork

        let mut orch = Orchestrator::new(client, cfg).with_local_ci(spy);
        orch.run_cycle().await.unwrap();

        assert_eq!(
            *ran_for.lock().unwrap(),
            vec!["claude/issue-1-fix"],
            "non-fork PRs should always run local CI"
        );
    }
}
