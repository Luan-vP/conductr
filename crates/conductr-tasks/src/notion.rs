//! Minimal Notion REST client: search, fetch a page/database, create a page,
//! update a page. Authenticates with an integration token (set
//! `NOTION_API_KEY` or pass to [`Notion::with_token`]).
//!
//! Reference: https://developers.notion.com/reference/intro

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

const NOTION_VERSION: &str = "2022-06-28";
const BASE_URL: &str = "https://api.notion.com/v1";

#[derive(Debug, Clone)]
pub struct Notion {
    pub client: reqwest::Client,
    pub token: String,
    pub version: String,
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
        Self { client, token: token.into(), version: NOTION_VERSION.to_string() }
    }

    fn req(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.client
            .request(method, format!("{BASE_URL}{path}"))
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

    /// Search across the workspace. `query` is a free-text query. `filter`
    /// can be `"page"` or `"database"` to scope.
    pub async fn search(&self, query: &str, filter: Option<&str>) -> Result<SearchResponse, NotionError> {
        #[derive(Serialize)]
        struct Body<'a> {
            query: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            filter: Option<Filter<'a>>,
            page_size: u32,
        }
        #[derive(Serialize)]
        struct Filter<'a> { property: &'a str, value: &'a str }

        let body = Body {
            query,
            filter: filter.map(|v| Filter { property: "object", value: v }),
            page_size: 25,
        };
        let rb = self.req(reqwest::Method::POST, "/search").json(&body);
        self.send_json(rb).await
    }

    /// Fetch a page by ID.
    pub async fn get_page(&self, id: &str) -> Result<Value, NotionError> {
        let rb = self.req(reqwest::Method::GET, &format!("/pages/{id}"));
        self.send_json(rb).await
    }

    /// Create a page in a database. `properties` is an object matching the
    /// database schema (see Notion docs); `children` is optional block content.
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

    /// Update properties on a page.
    pub async fn update_page(&self, id: &str, properties: Value) -> Result<Value, NotionError> {
        let body = serde_json::json!({ "properties": properties });
        let rb = self.req(reqwest::Method::PATCH, &format!("/pages/{id}")).json(&body);
        self.send_json(rb).await
    }

    /// Convenience: create a "task" page from a [`crate::Task`] in a
    /// database that has at minimum a Title property called `Name` and an
    /// optional Status select.
    pub async fn upsert_task(
        &self,
        database_id: &str,
        task: &crate::Task,
    ) -> Result<Value, NotionError> {
        let status_label = match task.status {
            crate::TaskStatus::NotStarted => "Not started",
            crate::TaskStatus::InProgress => "In progress",
            crate::TaskStatus::Blocked => "Blocked",
            crate::TaskStatus::Done => "Done",
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
            let multi: Vec<_> = task.tags.iter().map(|t| serde_json::json!({"name": t})).collect();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_env_errors_when_unset() {
        // SAFETY: we intentionally remove the var for the duration of this test.
        // SAFETY: tests in this crate don't otherwise read NOTION_API_KEY.
        let prev = std::env::var("NOTION_API_KEY").ok();
        // SAFETY: setting / unsetting env vars is unsafe in 2024 edition; we accept
        // the test-process race for the brevity of this assertion.
        unsafe { std::env::remove_var("NOTION_API_KEY"); }
        let r = Notion::from_env();
        assert!(matches!(r, Err(NotionError::MissingToken)));
        if let Some(p) = prev {
            unsafe { std::env::set_var("NOTION_API_KEY", p); }
        }
    }
}
