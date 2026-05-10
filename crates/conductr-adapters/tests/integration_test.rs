//! Integration tests for conductr-adapters with the full feature set.
//!
//! Tests that require real external tools (tmux, br, gh) are skipped when
//! those tools are absent. Mock-based tests always run.

#[cfg(feature = "mock")]
mod mock_tests {
    use conductr_adapters::mock::{
        make_issue, make_pr, make_task, MockIssueTracker, MockScmHost, MockTmuxAgent,
    };
    use conductr_core::ports::{IssueTracker, ScmHost, TmuxAgent};
    use conductr_core::types::{RepoSlug, TaskStatus};

    #[tokio::test]
    async fn tmux_agent_roundtrip() {
        let agent = MockTmuxAgent::new().with_session("work", "❯ claude");
        let sessions = agent.list_sessions().await.unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].name, "work");

        let pane = agent.capture_pane("work").await.unwrap();
        assert_eq!(pane, "❯ claude");

        agent.send_line("work", "/compact").await.unwrap();
        assert_eq!(agent.sent_lines("work"), vec!["/compact"]);
    }

    #[tokio::test]
    async fn issue_tracker_create_list_close() {
        let tracker = MockIssueTracker::new();
        let t = tracker
            .create_full("Fix the bug", Some(2), Some("details"), &["p2"])
            .await
            .unwrap();
        assert_eq!(t.title, "Fix the bug");

        let all = tracker.list().await.unwrap();
        assert_eq!(all.len(), 1);

        tracker.close(&t.id).await.unwrap();
        assert_eq!(tracker.closed_ids(), vec![t.id]);
    }

    #[tokio::test]
    async fn issue_tracker_list_ready_filters_done() {
        let tracker = MockIssueTracker::new().with_tasks([
            make_task("a", "Active", TaskStatus::NotStarted),
            make_task("b", "Done", TaskStatus::Done),
            make_task("c", "In progress", TaskStatus::InProgress),
        ]);
        let ready = tracker.list_ready().await.unwrap();
        assert_eq!(ready.len(), 2);
        assert!(ready.iter().all(|t| t.status != TaskStatus::Done));
    }

    #[tokio::test]
    async fn scm_host_comment_and_merge() {
        let repo = RepoSlug::new("acme", "widget");
        let host = MockScmHost::new()
            .with_issues([make_issue(1, "Add feature", &["enhancement"])])
            .with_prs([make_pr(10, "feat: add widget", "feature/widget")]);

        host.comment_issue(&repo, 1, "@claude please implement").await.unwrap();
        host.merge_pr_squash(&repo, 10).await.unwrap();

        assert_eq!(host.posted_comments().len(), 1);
        assert_eq!(host.merged_prs(), vec![10]);

        let open_issues = host.list_open_issues(&repo).await.unwrap();
        assert_eq!(open_issues.len(), 1);

        let open_prs = host.list_open_prs(&repo).await.unwrap();
        assert_eq!(open_prs.len(), 1);
    }
}

#[cfg(feature = "tmux")]
mod tmux_tests {
    use conductr_adapters::tmux::Tmux;

    #[test]
    fn tmux_struct_construction() {
        let t = Tmux::new();
        assert_eq!(t.binary, "tmux");
        let t2 = t.with_binary("tmux-alternative");
        assert_eq!(t2.binary, "tmux-alternative");
    }
}

#[cfg(feature = "beads")]
mod beads_tests {
    use conductr_adapters::beads::Beads;

    #[test]
    fn beads_struct_construction() {
        let b = Beads::new();
        assert_eq!(b.binary, "br");
        assert!(b.repo_dir.is_none());
    }
}

#[cfg(feature = "notion")]
mod notion_tests {
    use conductr_adapters::notion::Notion;
    use conductr_core::ports::{IssueTracker, IssueTrackerError};
    use conductr_core::types::{Task, TaskStatus};

    #[test]
    fn notion_requires_database_for_upsert() {
        let n = Notion::with_token("fake-token");
        assert!(n.database_id.is_none());
    }

    #[tokio::test]
    async fn notion_read_methods_return_unsupported() {
        let n = Notion::with_token("fake-token");
        assert!(matches!(n.list().await, Err(IssueTrackerError::Unsupported)));
        assert!(matches!(n.list_ready().await, Err(IssueTrackerError::Unsupported)));
        assert!(matches!(
            n.create_full("x", None, None, &[]).await,
            Err(IssueTrackerError::Unsupported)
        ));
        assert!(matches!(n.close("id").await, Err(IssueTrackerError::Unsupported)));
    }

    #[tokio::test]
    async fn notion_upsert_without_database_returns_config_error() {
        let n = Notion::with_token("fake-token");
        let task = Task {
            id: "t1".into(),
            title: "Test".into(),
            status: TaskStatus::NotStarted,
            priority: None,
            depends_on: vec![],
            tags: vec![],
            body: None,
        };
        assert!(matches!(
            n.upsert_task(&task).await,
            Err(IssueTrackerError::Configuration(_))
        ));
    }
}

#[cfg(feature = "gh-cli")]
mod gh_cli_tests {
    use conductr_adapters::gh_cli::{linked_issue_from, GhCli};

    #[test]
    fn gh_cli_struct_construction() {
        let _ = GhCli;
    }

    #[test]
    fn linked_issue_branch_parsing() {
        assert_eq!(linked_issue_from("claude/issue-42-foo", ""), Some(42));
        assert_eq!(linked_issue_from("feature/bar", "closes #5"), Some(5));
        assert_eq!(linked_issue_from("feature/bar", "see #99"), None);
    }
}
