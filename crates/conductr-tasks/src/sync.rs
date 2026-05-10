use conductr_core::ports::{IssueTracker, IssueTrackerError};
use conductr_core::types::{SyncReport, Task};

pub async fn list_tasks(
    src: &impl IssueTracker,
    ready_only: bool,
) -> Result<Vec<Task>, IssueTrackerError> {
    if ready_only {
        src.list_ready().await
    } else {
        src.list().await
    }
}

pub async fn create_task(
    src: &impl IssueTracker,
    title: &str,
    priority: Option<u8>,
    body: Option<&str>,
    labels: &[&str],
) -> Result<Task, IssueTrackerError> {
    src.create_full(title, priority, body, labels).await
}

/// Push every task from `src` into `dst`.
///
/// `dst_database` is an optional Notion-specific database identifier kept in
/// this signature so callers can pass it through without leaking
/// adapter-specific types. The `dst` adapter must already be configured with
/// the correct target (e.g. via `Notion::with_database`).
pub async fn sync_tasks(
    src: &impl IssueTracker,
    dst: &impl IssueTracker,
    _dst_database: Option<&str>,
) -> Result<SyncReport, IssueTrackerError> {
    let tasks = src.list().await?;
    let mut report = SyncReport::default();
    for t in tasks {
        match dst.upsert_task(&t).await {
            Ok(_) => report.pushed.push(t.id),
            Err(e) => report.failed.push((t.id, e.to_string())),
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use conductr_adapters::mock::{make_task, MockIssueTracker};
    use conductr_core::types::TaskStatus;

    #[tokio::test]
    async fn list_tasks_returns_all() {
        let tracker = MockIssueTracker::new().with_tasks([
            make_task("t-1", "Task One", TaskStatus::NotStarted),
            make_task("t-2", "Task Two", TaskStatus::InProgress),
        ]);
        let tasks = list_tasks(&tracker, false).await.unwrap();
        assert_eq!(tasks.len(), 2);
    }

    #[tokio::test]
    async fn list_tasks_ready_only_filters_done() {
        let tracker = MockIssueTracker::new().with_tasks([
            make_task("t-1", "Open Task", TaskStatus::NotStarted),
            make_task("t-2", "Done Task", TaskStatus::Done),
        ]);
        let tasks = list_tasks(&tracker, true).await.unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, "t-1");
    }

    #[tokio::test]
    async fn create_task_inserts_into_tracker() {
        let tracker = MockIssueTracker::new();
        let t = create_task(&tracker, "My Task", Some(2), Some("body text"), &["tag1"])
            .await
            .unwrap();
        assert_eq!(t.title, "My Task");
        assert_eq!(t.priority, Some(2));
        assert_eq!(tracker.all_tasks().len(), 1);
    }

    #[tokio::test]
    async fn sync_tasks_copies_all_to_dst() {
        let src = MockIssueTracker::new().with_tasks([
            make_task("t-1", "Task One", TaskStatus::NotStarted),
            make_task("t-2", "Task Two", TaskStatus::Done),
        ]);
        let dst = MockIssueTracker::new();
        let report = sync_tasks(&src, &dst, None).await.unwrap();
        assert_eq!(report.pushed.len(), 2);
        assert!(report.failed.is_empty());
        assert_eq!(dst.all_tasks().len(), 2);
    }
}
