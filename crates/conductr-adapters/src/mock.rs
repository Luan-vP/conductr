//! In-memory test doubles for all port traits.
//!
//! These mocks live here (not per-crate) so every use-case crate shares the
//! same inspection helpers. Enable with feature `mock`.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, RwLock};
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use conductr_core::ports::{
    CalendarError, CalendarPort, InstanceError, InstanceProvider, IssueTracker, IssueTrackerError,
    LocalAgent, LocalAgentError, Mailbox, MailboxError, ScmHost, TmuxAgent, TmuxError,
};
use conductr_core::signals::{MailKind, MailMessage, MailRef};
use conductr_core::types::{
    CalendarEvent, CiStatus, ClosedPr, InstanceHandle, InstanceSpec, Issue, IssueNumber,
    IssueState, NewCalendarEvent, Pr, PrNumber, PrState, Provider, RepoSlug, Task, TaskStatus,
    TmuxSession, UpdateCalendarEvent,
};

// ── MockTmuxAgent ─────────────────────────────────────────────────────────────

#[derive(Debug, Default, Clone)]
struct MockSession {
    pane_content: String,
}

#[derive(Debug, Default)]
pub struct MockTmuxAgent {
    sessions: Arc<RwLock<HashMap<String, MockSession>>>,
    sent_lines: Arc<RwLock<HashMap<String, Vec<String>>>>,
    sent_keys: Arc<RwLock<HashMap<String, Vec<String>>>>,
}

impl MockTmuxAgent {
    pub fn new() -> Self {
        Self::default()
    }

    /// Pre-seed a session with a given pane content.
    pub fn with_session(self, name: impl Into<String>, pane_content: impl Into<String>) -> Self {
        self.sessions.write().unwrap().insert(
            name.into(),
            MockSession { pane_content: pane_content.into() },
        );
        self
    }

    /// Return all lines sent via `send_line` for a session.
    pub fn sent_lines(&self, session: &str) -> Vec<String> {
        self.sent_lines
            .read()
            .unwrap()
            .get(session)
            .cloned()
            .unwrap_or_default()
    }

    /// Return all keys sent via `send_key` for a session.
    pub fn sent_keys(&self, session: &str) -> Vec<String> {
        self.sent_keys
            .read()
            .unwrap()
            .get(session)
            .cloned()
            .unwrap_or_default()
    }
}

#[async_trait]
impl TmuxAgent for MockTmuxAgent {
    async fn list_sessions(&self) -> Result<Vec<TmuxSession>, TmuxError> {
        let now = Utc::now();
        Ok(self
            .sessions
            .read()
            .unwrap()
            .keys()
            .map(|name| TmuxSession {
                name: name.clone(),
                created: now,
                last_activity: now,
                windows: 1,
                attached: false,
                cwd: None,
            })
            .collect())
    }

    async fn capture_pane(&self, session: &str) -> Result<String, TmuxError> {
        self.sessions
            .read()
            .unwrap()
            .get(session)
            .map(|s| s.pane_content.clone())
            .ok_or_else(|| TmuxError::Parse(format!("session not found: {session}")))
    }

    async fn send_line(&self, session: &str, text: &str) -> Result<(), TmuxError> {
        self.sent_lines
            .write()
            .unwrap()
            .entry(session.to_string())
            .or_default()
            .push(text.to_string());
        Ok(())
    }

    async fn send_key(&self, session: &str, key: &str) -> Result<(), TmuxError> {
        self.sent_keys
            .write()
            .unwrap()
            .entry(session.to_string())
            .or_default()
            .push(key.to_string());
        Ok(())
    }

    async fn new_session(&self, name: &str, _cwd: &str) -> Result<(), TmuxError> {
        self.sessions
            .write()
            .unwrap()
            .entry(name.to_string())
            .or_insert_with(|| MockSession { pane_content: String::new() });
        Ok(())
    }
}

// ── MockIssueTracker ──────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct MockIssueTracker {
    tasks: Arc<RwLock<HashMap<String, Task>>>,
    counter: Arc<AtomicU64>,
    closed_ids: Arc<RwLock<Vec<String>>>,
}

impl MockIssueTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Pre-seed tasks.
    pub fn with_tasks(self, tasks: impl IntoIterator<Item = Task>) -> Self {
        let mut guard = self.tasks.write().unwrap();
        for t in tasks {
            guard.insert(t.id.clone(), t);
        }
        drop(guard);
        self
    }

    /// Snapshot of all tasks currently in the tracker.
    pub fn all_tasks(&self) -> Vec<Task> {
        self.tasks.read().unwrap().values().cloned().collect()
    }

    /// IDs that were closed via `close()`.
    pub fn closed_ids(&self) -> Vec<String> {
        self.closed_ids.read().unwrap().clone()
    }
}

#[async_trait]
impl IssueTracker for MockIssueTracker {
    async fn list(&self) -> Result<Vec<Task>, IssueTrackerError> {
        Ok(self.tasks.read().unwrap().values().cloned().collect())
    }

    async fn list_ready(&self) -> Result<Vec<Task>, IssueTrackerError> {
        Ok(self
            .tasks
            .read()
            .unwrap()
            .values()
            .filter(|t| {
                matches!(t.status, TaskStatus::NotStarted | TaskStatus::InProgress)
            })
            .cloned()
            .collect())
    }

    async fn create_full(
        &self,
        title: &str,
        priority: Option<u8>,
        body: Option<&str>,
        labels: &[&str],
    ) -> Result<Task, IssueTrackerError> {
        let id = format!("mock-{}", self.counter.fetch_add(1, Ordering::SeqCst));
        let task = Task {
            id: id.clone(),
            title: title.to_string(),
            status: TaskStatus::NotStarted,
            priority,
            depends_on: vec![],
            tags: labels.iter().map(|l| l.to_string()).collect(),
            body: body.map(str::to_string),
        };
        self.tasks.write().unwrap().insert(id, task.clone());
        Ok(task)
    }

    async fn close(&self, id: &str) -> Result<(), IssueTrackerError> {
        let mut tasks = self.tasks.write().unwrap();
        match tasks.get_mut(id) {
            Some(t) => {
                t.status = TaskStatus::Done;
                self.closed_ids.write().unwrap().push(id.to_string());
                Ok(())
            }
            None => Err(IssueTrackerError::NotFound(id.to_string())),
        }
    }

    async fn upsert_task(&self, task: &Task) -> Result<(), IssueTrackerError> {
        self.tasks.write().unwrap().insert(task.id.clone(), task.clone());
        Ok(())
    }
}

// ── MockScmHost ───────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct MockScmHost {
    issues: Arc<RwLock<Vec<Issue>>>,
    closed_issue_numbers: Arc<RwLock<BTreeSet<IssueNumber>>>,
    prs: Arc<RwLock<Vec<Pr>>>,
    comments: Arc<RwLock<HashMap<IssueNumber, Vec<String>>>>,
    posted_comments: Arc<RwLock<Vec<(IssueNumber, String)>>>,
    assignments: Arc<RwLock<Vec<(IssueNumber, String)>>>,
    merged_prs: Arc<RwLock<Vec<PrNumber>>>,
    created_issues: Arc<RwLock<Vec<(String, String, Vec<String>)>>>,
    issue_counter: Arc<AtomicU64>,
    closed_prs: Arc<RwLock<Vec<ClosedPr>>>,
    labels_added: Arc<RwLock<Vec<(IssueNumber, String)>>>,
    labels_removed: Arc<RwLock<Vec<(IssueNumber, String)>>>,
}

impl MockScmHost {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_issues(self, issues: impl IntoIterator<Item = Issue>) -> Self {
        *self.issues.write().unwrap() = issues.into_iter().collect();
        self
    }

    pub fn with_prs(self, prs: impl IntoIterator<Item = Pr>) -> Self {
        *self.prs.write().unwrap() = prs.into_iter().collect();
        self
    }

    pub fn with_closed_issues(self, numbers: impl IntoIterator<Item = IssueNumber>) -> Self {
        *self.closed_issue_numbers.write().unwrap() = numbers.into_iter().collect();
        self
    }

    /// Comments posted via `comment_issue`.
    pub fn posted_comments(&self) -> Vec<(IssueNumber, String)> {
        self.posted_comments.read().unwrap().clone()
    }

    /// Assignments made via `assign_issue`.
    pub fn assignments(&self) -> Vec<(IssueNumber, String)> {
        self.assignments.read().unwrap().clone()
    }

    /// PRs merged via `merge_pr_squash`.
    pub fn merged_prs(&self) -> Vec<PrNumber> {
        self.merged_prs.read().unwrap().clone()
    }

    /// Issues created via `create_issue`: `(title, body, labels)`.
    pub fn created_issues(&self) -> Vec<(String, String, Vec<String>)> {
        self.created_issues.read().unwrap().clone()
    }

    pub fn with_closed_prs(self, prs: impl IntoIterator<Item = ClosedPr>) -> Self {
        *self.closed_prs.write().unwrap() = prs.into_iter().collect();
        self
    }

    /// Labels added via `add_issue_label`: `(issue_number, label)`.
    pub fn labels_added(&self) -> Vec<(IssueNumber, String)> {
        self.labels_added.read().unwrap().clone()
    }

    /// Labels removed via `remove_issue_label`: `(issue_number, label)`.
    pub fn labels_removed(&self) -> Vec<(IssueNumber, String)> {
        self.labels_removed.read().unwrap().clone()
    }
}

#[async_trait]
impl ScmHost for MockScmHost {
    async fn list_open_issues(&self, _repo: &RepoSlug) -> anyhow::Result<Vec<Issue>> {
        Ok(self
            .issues
            .read()
            .unwrap()
            .iter()
            .filter(|i| i.state == IssueState::Open)
            .cloned()
            .collect())
    }

    async fn list_closed_issue_numbers(
        &self,
        _repo: &RepoSlug,
    ) -> anyhow::Result<BTreeSet<IssueNumber>> {
        Ok(self.closed_issue_numbers.read().unwrap().clone())
    }

    async fn list_open_prs(&self, _repo: &RepoSlug) -> anyhow::Result<Vec<Pr>> {
        Ok(self
            .prs
            .read()
            .unwrap()
            .iter()
            .filter(|p| p.state == PrState::Open)
            .cloned()
            .collect())
    }

    async fn list_issue_comments(
        &self,
        _repo: &RepoSlug,
        n: IssueNumber,
    ) -> anyhow::Result<Vec<String>> {
        Ok(self
            .comments
            .read()
            .unwrap()
            .get(&n)
            .cloned()
            .unwrap_or_default())
    }

    async fn comment_issue(
        &self,
        _repo: &RepoSlug,
        n: IssueNumber,
        body: &str,
    ) -> anyhow::Result<()> {
        self.posted_comments.write().unwrap().push((n, body.to_string()));
        Ok(())
    }

    async fn assign_issue(
        &self,
        _repo: &RepoSlug,
        n: IssueNumber,
        login: &str,
    ) -> anyhow::Result<()> {
        self.assignments.write().unwrap().push((n, login.to_string()));
        Ok(())
    }

    async fn merge_pr_squash(&self, _repo: &RepoSlug, n: PrNumber) -> anyhow::Result<()> {
        self.merged_prs.write().unwrap().push(n);
        Ok(())
    }

    async fn create_issue(
        &self,
        _repo: &RepoSlug,
        title: &str,
        body: &str,
        labels: &[&str],
    ) -> anyhow::Result<IssueNumber> {
        let number = self.issue_counter.fetch_add(1, Ordering::SeqCst) + 1;
        self.created_issues.write().unwrap().push((
            title.to_string(),
            body.to_string(),
            labels.iter().map(|l| l.to_string()).collect(),
        ));
        Ok(number)
    }

    async fn add_issue_label(
        &self,
        _repo: &RepoSlug,
        n: IssueNumber,
        label: &str,
    ) -> anyhow::Result<()> {
        self.labels_added.write().unwrap().push((n, label.to_string()));
        let mut issues = self.issues.write().unwrap();
        if let Some(issue) = issues.iter_mut().find(|i| i.number == n) {
            if !issue.labels.iter().any(|l| l == label) {
                issue.labels.push(label.to_string());
            }
        }
        Ok(())
    }

    async fn remove_issue_label(
        &self,
        _repo: &RepoSlug,
        n: IssueNumber,
        label: &str,
    ) -> anyhow::Result<()> {
        self.labels_removed.write().unwrap().push((n, label.to_string()));
        let mut issues = self.issues.write().unwrap();
        if let Some(issue) = issues.iter_mut().find(|i| i.number == n) {
            issue.labels.retain(|l| l != label);
        }
        Ok(())
    }

    async fn list_closed_prs(&self, _repo: &RepoSlug) -> anyhow::Result<Vec<ClosedPr>> {
        Ok(self.closed_prs.read().unwrap().clone())
    }
}

// ── MockInstanceProvider ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct MockInstanceProvider;

#[async_trait]
impl InstanceProvider for MockInstanceProvider {
    async fn spin_up(&self, spec: &InstanceSpec) -> Result<InstanceHandle, InstanceError> {
        Ok(InstanceHandle {
            id: format!("mock-{}", spec.name),
            provider: spec.provider,
            host: "127.0.0.1".to_string(),
            user: "mock".to_string(),
        })
    }

    async fn connect(&self, _handle: &InstanceHandle) -> Result<(), InstanceError> {
        Ok(())
    }

    async fn run(&self, _handle: &InstanceHandle, cmd: &str) -> Result<String, InstanceError> {
        Ok(format!("mock output for: {cmd}"))
    }

    async fn tear_down(&self, _handle: &InstanceHandle) -> Result<(), InstanceError> {
        Ok(())
    }
}

// ── MockMailbox ───────────────────────────────────────────────────────────────

#[derive(Debug, Default, Clone)]
pub struct MockMailbox {
    messages: Arc<RwLock<Vec<MailMessage>>>,
    counter: Arc<AtomicU64>,
}

impl MockMailbox {
    pub fn new() -> Self {
        Self::default()
    }

    /// All messages currently in the mailbox.
    pub fn all_messages(&self) -> Vec<MailMessage> {
        self.messages.read().unwrap().clone()
    }
}

#[async_trait]
impl Mailbox for MockMailbox {
    async fn send(&self, agent: &str, payload: MailKind) -> Result<MailRef, MailboxError> {
        let id = format!("mock-msg-{}", self.counter.fetch_add(1, Ordering::SeqCst));
        let msg = MailMessage {
            id: id.clone(),
            agent: agent.to_string(),
            sent_at: Utc::now(),
            payload,
        };
        self.messages.write().unwrap().push(msg);
        Ok(id)
    }

    async fn inbox(
        &self,
        kind_filter: Option<&str>,
        _since: Option<std::time::Duration>,
    ) -> Result<Vec<MailMessage>, MailboxError> {
        let all = self.messages.read().unwrap().clone();
        if let Some(filter) = kind_filter {
            Ok(all
                .into_iter()
                .filter(|m| kind_tag(&m.payload) == filter)
                .collect())
        } else {
            Ok(all)
        }
    }

    async fn thread(&self, thread_id: &MailRef) -> Result<Vec<MailMessage>, MailboxError> {
        Ok(self
            .messages
            .read()
            .unwrap()
            .iter()
            .filter(|m| &m.id == thread_id)
            .cloned()
            .collect())
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

// ── MockLocalAgent ────────────────────────────────────────────────────────────

/// In-memory `LocalAgent` that returns a fixed response regardless of prompt.
#[derive(Debug, Clone)]
pub struct MockLocalAgent {
    agent_name: &'static str,
    response: String,
}

impl MockLocalAgent {
    pub fn new(name: &'static str, response: impl Into<String>) -> Self {
        Self { agent_name: name, response: response.into() }
    }
}

#[async_trait]
impl LocalAgent for MockLocalAgent {
    async fn name(&self) -> &'static str {
        self.agent_name
    }

    async fn complete(&self, _prompt: &str) -> Result<String, LocalAgentError> {
        Ok(self.response.clone())
    }
}

// ── PiStub ────────────────────────────────────────────────────────────────────

/// Placeholder `LocalAgent` for the Pi provider.
/// Returns `NotInstalled` on every call. Real implementation tracked in #57.
#[derive(Debug, Default, Clone)]
pub struct PiStub;

#[async_trait]
impl LocalAgent for PiStub {
    async fn name(&self) -> &'static str {
        "pi"
    }

    async fn complete(&self, _prompt: &str) -> Result<String, LocalAgentError> {
        Err(LocalAgentError::NotInstalled)
    }
}

// ── MockCalendarPort ─────────────────────────────────────────────────────────

/// In-memory `CalendarPort` for use in tests.
#[derive(Debug, Default, Clone)]
pub struct MockCalendarPort {
    events: Arc<RwLock<Vec<CalendarEvent>>>,
    counter: Arc<AtomicU64>,
    created: Arc<RwLock<Vec<CalendarEvent>>>,
    deleted_ids: Arc<RwLock<Vec<String>>>,
}

impl MockCalendarPort {
    pub fn new() -> Self {
        Self::default()
    }

    /// Pre-seed events (e.g. window and blocker events).
    pub fn with_events(self, events: impl IntoIterator<Item = CalendarEvent>) -> Self {
        *self.events.write().unwrap() = events.into_iter().collect();
        self
    }

    /// Events created via `create_event`.
    pub fn created_events(&self) -> Vec<CalendarEvent> {
        self.created.read().unwrap().clone()
    }

    /// IDs deleted via `delete_event`.
    pub fn deleted_ids(&self) -> Vec<String> {
        self.deleted_ids.read().unwrap().clone()
    }

    /// All events currently in the store (seeded + created).
    pub fn all_events(&self) -> Vec<CalendarEvent> {
        self.events.read().unwrap().clone()
    }
}

#[async_trait]
impl CalendarPort for MockCalendarPort {
    async fn list_upcoming_events(
        &self,
        from: DateTime<Utc>,
    ) -> Result<Vec<CalendarEvent>, CalendarError> {
        Ok(self
            .events
            .read()
            .unwrap()
            .iter()
            .filter(|e| e.start >= from)
            .cloned()
            .collect())
    }

    async fn create_event(
        &self,
        event: NewCalendarEvent,
    ) -> Result<CalendarEvent, CalendarError> {
        let id = format!("mock-cal-{}", self.counter.fetch_add(1, Ordering::SeqCst));
        let ev = CalendarEvent {
            id: id.clone(),
            title: event.title,
            start: event.start,
            end: event.end,
            description: Some(event.description),
        };
        self.events.write().unwrap().push(ev.clone());
        self.created.write().unwrap().push(ev.clone());
        Ok(ev)
    }

    async fn update_event(
        &self,
        id: &str,
        event: UpdateCalendarEvent,
    ) -> Result<CalendarEvent, CalendarError> {
        let mut store = self.events.write().unwrap();
        let ev = store
            .iter_mut()
            .find(|e| e.id == id)
            .ok_or_else(|| CalendarError::NotFound(id.to_string()))?;
        if let Some(t) = event.title {
            ev.title = t;
        }
        if let Some(s) = event.start {
            ev.start = s;
        }
        if let Some(e) = event.end {
            ev.end = e;
        }
        if let Some(d) = event.description {
            ev.description = Some(d);
        }
        Ok(ev.clone())
    }

    async fn delete_event(&self, id: &str) -> Result<(), CalendarError> {
        let mut store = self.events.write().unwrap();
        let pos = store
            .iter()
            .position(|e| e.id == id)
            .ok_or_else(|| CalendarError::NotFound(id.to_string()))?;
        store.remove(pos);
        self.deleted_ids.write().unwrap().push(id.to_string());
        Ok(())
    }
}

// ── Builder helpers ───────────────────────────────────────────────────────────

/// Convenience constructor for a timed calendar event.
pub fn make_calendar_event(
    id: &str,
    title: &str,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    description: Option<&str>,
) -> CalendarEvent {
    CalendarEvent {
        id: id.to_string(),
        title: title.to_string(),
        start,
        end,
        description: description.map(str::to_string),
    }
}

/// Convenience constructor for a simple open issue.
pub fn make_issue(number: IssueNumber, title: &str, labels: &[&str]) -> Issue {
    Issue {
        number,
        title: title.to_string(),
        body: String::new(),
        labels: labels.iter().map(|s| s.to_string()).collect(),
        state: IssueState::Open,
    }
}

/// Convenience constructor for a simple open PR.
pub fn make_pr(number: PrNumber, title: &str, head_ref: &str) -> Pr {
    Pr {
        number,
        title: title.to_string(),
        body: String::new(),
        head_ref: head_ref.to_string(),
        state: PrState::Open,
        ci: CiStatus::Unknown,
        linked_issue: None,
    }
}

/// Convenience constructor for a task.
pub fn make_task(id: &str, title: &str, status: TaskStatus) -> Task {
    Task {
        id: id.to_string(),
        title: title.to_string(),
        status,
        priority: None,
        depends_on: vec![],
        tags: vec![],
        body: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_tmux_send_and_inspect() {
        let agent = MockTmuxAgent::new().with_session("s1", "some pane content");

        agent.send_line("s1", "hello world").await.unwrap();
        agent.send_line("s1", "second line").await.unwrap();
        agent.send_key("s1", "C-c").await.unwrap();

        assert_eq!(agent.sent_lines("s1"), vec!["hello world", "second line"]);
        assert_eq!(agent.sent_keys("s1"), vec!["C-c"]);
        assert_eq!(agent.capture_pane("s1").await.unwrap(), "some pane content");
    }

    #[tokio::test]
    async fn mock_tmux_list_sessions() {
        let agent = MockTmuxAgent::new()
            .with_session("alpha", "")
            .with_session("beta", "");
        let sessions = agent.list_sessions().await.unwrap();
        let mut names: Vec<_> = sessions.iter().map(|s| s.name.as_str()).collect();
        names.sort();
        assert_eq!(names, vec!["alpha", "beta"]);
    }

    #[tokio::test]
    async fn mock_issue_tracker_crud() {
        let tracker = MockIssueTracker::new();

        let task = tracker.create_full("Test task", Some(1), None, &["tag1"]).await.unwrap();
        assert_eq!(task.title, "Test task");
        assert_eq!(task.priority, Some(1));

        let all = tracker.list().await.unwrap();
        assert_eq!(all.len(), 1);

        tracker.close(&task.id).await.unwrap();
        let closed = tracker.closed_ids();
        assert_eq!(closed, vec![task.id.clone()]);
    }

    #[tokio::test]
    async fn mock_issue_tracker_upsert() {
        let tracker = MockIssueTracker::new();
        let task = make_task("t-1", "My task", TaskStatus::InProgress);
        tracker.upsert_task(&task).await.unwrap();
        let all = tracker.all_tasks();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, "t-1");
    }

    #[tokio::test]
    async fn mock_scm_host_comment_and_inspect() {
        let repo = RepoSlug::new("owner", "repo");
        let host = MockScmHost::new();

        host.comment_issue(&repo, 42, "lgtm").await.unwrap();
        host.assign_issue(&repo, 42, "alice").await.unwrap();
        host.merge_pr_squash(&repo, 7).await.unwrap();

        assert_eq!(host.posted_comments(), vec![(42, "lgtm".to_string())]);
        assert_eq!(host.assignments(), vec![(42, "alice".to_string())]);
        assert_eq!(host.merged_prs(), vec![7]);
    }

    #[tokio::test]
    async fn mock_scm_host_create_issue() {
        let repo = RepoSlug::new("owner", "repo");
        let host = MockScmHost::new();

        let n1 = host.create_issue(&repo, "Title 1", "Body 1", &["label-a"]).await.unwrap();
        let n2 = host.create_issue(&repo, "Title 2", "Body 2", &["label-b", "label-c"]).await.unwrap();

        assert_eq!(n1, 1);
        assert_eq!(n2, 2);
        let created = host.created_issues();
        assert_eq!(created.len(), 2);
        assert_eq!(created[0].0, "Title 1");
        assert_eq!(created[0].2, vec!["label-a"]);
        assert_eq!(created[1].2, vec!["label-b", "label-c"]);
    }

    #[tokio::test]
    async fn mock_scm_host_list_issues() {
        let repo = RepoSlug::new("owner", "repo");
        let host = MockScmHost::new().with_issues([
            make_issue(1, "First", &["bug"]),
            make_issue(2, "Second", &[]),
        ]);
        let issues = host.list_open_issues(&repo).await.unwrap();
        assert_eq!(issues.len(), 2);
    }

    #[tokio::test]
    async fn mock_local_agent_returns_fixed_response() {
        let agent = MockLocalAgent::new("test", "hello world");
        assert_eq!(agent.name().await, "test");
        let resp = agent.complete("any prompt").await.unwrap();
        assert_eq!(resp, "hello world");
    }

    #[tokio::test]
    async fn pi_stub_returns_not_installed() {
        let stub = PiStub;
        assert_eq!(stub.name().await, "pi");
        let err = stub.complete("hi").await.unwrap_err();
        assert!(matches!(err, LocalAgentError::NotInstalled));
    }

    #[tokio::test]
    async fn mock_instance_provider_lifecycle() {
        let provider = MockInstanceProvider;
        let spec = InstanceSpec {
            name: "test-node".to_string(),
            provider: Provider::Local,
            size: "small".to_string(),
            region: None,
            image: None,
            ssh_key: None,
        };
        let handle = provider.spin_up(&spec).await.unwrap();
        assert_eq!(handle.id, "mock-test-node");
        provider.connect(&handle).await.unwrap();
        let out = provider.run(&handle, "echo hi").await.unwrap();
        assert!(out.contains("echo hi"));
        provider.tear_down(&handle).await.unwrap();
    }
}
