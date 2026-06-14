//! Minimal Notion REST client implementing [`IssueTracker`].
//!
//! Authenticates with an integration token (set `NOTION_API_KEY` or pass to
//! [`Notion::with_token`]).
//!
//! Read methods return [`IssueTrackerError::Unsupported`]; only `upsert_task`
//! is required for the IssueTracker trait. Use [`Notion::with_database`] to
//! configure the target database before calling `upsert_task`.
//!
//! Reference: https://developers.notion.com/reference/intro

use std::fmt;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

use conductr_core::ports::{IssueTracker, IssueTrackerError};
use conductr_core::types::{Task, TaskStatus};

const NOTION_VERSION: &str = "2022-06-28";
const BASE_URL: &str = "https://api.notion.com/v1";

#[derive(Clone)]
pub struct Notion {
    pub client: reqwest::Client,
    token: String,
    pub version: String,
    pub database_id: Option<String>,
}

impl fmt::Debug for Notion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Notion")
            .field("token", &"***")
            .field("version", &self.version)
            .field("database_id", &self.database_id)
            .finish()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum NotionError {
    #[error("missing NOTION_API_KEY")]
    MissingToken,
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("api error: {status} {body}")]
    Api { status: u16, body: String },
    #[error("serde: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("database_id not configured — call with_database()")]
    NoDatabaseId,
}

impl Notion {
    pub fn from_env() -> Result<Self, NotionError> {
        let token = std::env::var("NOTION_API_KEY").map_err(|_| NotionError::MissingToken)?;
        Ok(Self::with_token(token))
    }

    pub fn with_token(token: impl Into<String>) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("reqwest client builds");
        Self {
            client,
            token: token.into(),
            version: NOTION_VERSION.to_string(),
            database_id: None,
        }
    }

    pub fn with_database(mut self, database_id: impl Into<String>) -> Self {
        self.database_id = Some(database_id.into());
        self
    }

    fn req(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        // Parse for validation; concatenate directly so BASE_URL's /v1 prefix
        // is preserved (url::Url::join with a leading '/' would strip it).
        let url = Url::parse(&format!("{BASE_URL}{path}")).expect("valid url");
        self.client
            .request(method, url)
            .bearer_auth(&self.token)
            .header("Notion-Version", &self.version)
            .header("Content-Type", "application/json")
    }

    async fn send_json<T: for<'de> Deserialize<'de>>(
        &self,
        rb: reqwest::RequestBuilder,
    ) -> Result<T, NotionError> {
        let resp = rb.send().await?;
        let status = resp.status();
        let bytes = resp.bytes().await?;
        if !status.is_success() {
            return Err(NotionError::Api {
                status: status.as_u16(),
                body: String::from_utf8_lossy(&bytes).into_owned(),
            });
        }
        Ok(serde_json::from_slice(&bytes)?)
    }

    /// Search across the workspace. `filter` can be `"page"` or `"database"`.
    pub async fn search(
        &self,
        query: &str,
        filter: Option<&str>,
    ) -> Result<SearchResponse, NotionError> {
        #[derive(Serialize)]
        struct Body<'a> {
            query: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            filter: Option<Filter<'a>>,
            page_size: u32,
        }
        #[derive(Serialize)]
        struct Filter<'a> {
            property: &'a str,
            value: &'a str,
        }

        let body = Body {
            query,
            filter: filter.map(|v| Filter { property: "object", value: v }),
            page_size: 25,
        };
        let rb = self.req(reqwest::Method::POST, "/search").json(&body);
        self.send_json(rb).await
    }

    pub async fn get_page(&self, id: &str) -> Result<Value, NotionError> {
        let rb = self.req(reqwest::Method::GET, &format!("/pages/{id}"));
        self.send_json(rb).await
    }

    pub async fn create_page_in_database(
        &self,
        database_id: &str,
        properties: Value,
        children: Option<Value>,
    ) -> Result<Value, NotionError> {
        let mut body = serde_json::json!({
            "parent": { "database_id": database_id },
            "properties": properties,
        });
        if let Some(c) = children {
            body["children"] = c;
        }
        let rb = self.req(reqwest::Method::POST, "/pages").json(&body);
        self.send_json(rb).await
    }

    pub async fn update_page(&self, id: &str, properties: Value) -> Result<Value, NotionError> {
        let body = serde_json::json!({ "properties": properties });
        let rb = self.req(reqwest::Method::PATCH, &format!("/pages/{id}")).json(&body);
        self.send_json(rb).await
    }

    /// Low-level: upsert a task into an explicit database ID.
    pub async fn upsert_task_in_db(
        &self,
        database_id: &str,
        task: &Task,
    ) -> Result<Value, NotionError> {
        let status_label = match task.status {
            TaskStatus::NotStarted => "Not started",
            TaskStatus::InProgress => "In progress",
            TaskStatus::Blocked => "Blocked",
            TaskStatus::Done => "Done",
        };
        let mut props = serde_json::json!({
            "Name": {
                "title": [ { "text": { "content": task.title } } ]
            },
            "Status": {
                "status": { "name": status_label }
            }
        });
        if !task.tags.is_empty() {
            let multi: Vec<_> =
                task.tags.iter().map(|t| serde_json::json!({"name": t})).collect();
            props["Tags"] = serde_json::json!({ "multi_select": multi });
        }
        self.create_page_in_database(database_id, props, None).await
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct SearchResponse {
    pub results: Vec<Value>,
    pub has_more: bool,
    pub next_cursor: Option<String>,
}

#[async_trait]
impl IssueTracker for Notion {
    async fn list(&self) -> Result<Vec<Task>, IssueTrackerError> {
        Err(IssueTrackerError::Unsupported)
    }

    async fn list_ready(&self) -> Result<Vec<Task>, IssueTrackerError> {
        Err(IssueTrackerError::Unsupported)
    }

    async fn create_full(
        &self,
        _title: &str,
        _priority: Option<u8>,
        _body: Option<&str>,
        _labels: &[&str],
    ) -> Result<Task, IssueTrackerError> {
        Err(IssueTrackerError::Unsupported)
    }

    async fn close(&self, _id: &str) -> Result<(), IssueTrackerError> {
        Err(IssueTrackerError::Unsupported)
    }

    async fn upsert_task(&self, task: &Task) -> Result<(), IssueTrackerError> {
        let db_id = self
            .database_id
            .as_deref()
            .ok_or_else(|| IssueTrackerError::Configuration("database_id not set".into()))?;
        self.upsert_task_in_db(db_id, task)
            .await
            .map(|_| ())
            .map_err(|e| IssueTrackerError::Backend(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_env_errors_when_unset() {
        let prev = std::env::var("NOTION_API_KEY").ok();
        // SAFETY: test-process only; acceptable race for this assertion.
        unsafe { std::env::remove_var("NOTION_API_KEY") };
        let r = Notion::from_env();
        assert!(matches!(r, Err(NotionError::MissingToken)));
        if let Some(p) = prev {
            unsafe { std::env::set_var("NOTION_API_KEY", p) };
        }
    }

    #[test]
    fn with_database_sets_id() {
        let n = Notion::with_token("tok").with_database("db-123");
        assert_eq!(n.database_id.as_deref(), Some("db-123"));
    }
}
