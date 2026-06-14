//! Google Calendar REST API v3 adapter implementing [`CalendarPort`].
//!
//! Authentication: bearer token read from `GCAL_OAUTH_TOKEN`.
//! Calendar target: read from `GCAL_CALENDAR_ID` (defaults to `primary`).
//!
//! The token is a short-lived OAuth 2.0 access token. Refreshing it is out of
//! scope here — obtain a token via `gcloud auth print-access-token` or an
//! equivalent OAuth flow and export it before running `conductr sync`.

use std::fmt;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};

use conductr_core::ports::{CalendarError, CalendarPort};
use conductr_core::types::{CalendarEvent, NewCalendarEvent, UpdateCalendarEvent};

const GCAL_BASE: &str = "https://www.googleapis.com/calendar/v3";
const MAX_RESULTS_PER_PAGE: u32 = 250;
const MAX_PAGES: usize = 10;

// ── Public adapter ────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct GcalAdapter {
    client: reqwest::Client,
    token: String,
    calendar_id: String,
}

impl fmt::Debug for GcalAdapter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GcalAdapter")
            .field("token", &"***")
            .field("calendar_id", &self.calendar_id)
            .finish()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum GcalError {
    #[error("GCAL_OAUTH_TOKEN environment variable is not set")]
    MissingToken,
}

impl GcalAdapter {
    pub fn from_env() -> Result<Self, GcalError> {
        let token = std::env::var("GCAL_OAUTH_TOKEN").map_err(|_| GcalError::MissingToken)?;
        let calendar_id =
            std::env::var("GCAL_CALENDAR_ID").unwrap_or_else(|_| "primary".to_string());
        Ok(Self::new(token, calendar_id))
    }

    pub fn new(token: impl Into<String>, calendar_id: impl Into<String>) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("reqwest client builds");
        Self { client, token: token.into(), calendar_id: calendar_id.into() }
    }

    fn url(&self, path: &str) -> String {
        format!("{GCAL_BASE}{path}")
    }

    fn auth_header(&self) -> String {
        format!("Bearer {}", self.token)
    }
}

// ── CalendarPort impl ─────────────────────────────────────────────────────────

#[async_trait]
impl CalendarPort for GcalAdapter {
    async fn list_upcoming_events(
        &self,
        from: DateTime<Utc>,
    ) -> Result<Vec<CalendarEvent>, CalendarError> {
        let mut events: Vec<CalendarEvent> = vec![];
        let mut page_token: Option<String> = None;
        let base_url = self.url(&format!("/calendars/{}/events", self.calendar_id));

        for _ in 0..MAX_PAGES {
            let mut req = self
                .client
                .get(&base_url)
                .header("Authorization", self.auth_header())
                .query(&[
                    ("timeMin", from.to_rfc3339()),
                    ("singleEvents", "true".to_string()),
                    ("orderBy", "startTime".to_string()),
                    ("maxResults", MAX_RESULTS_PER_PAGE.to_string()),
                ]);

            if let Some(token) = &page_token {
                req = req.query(&[("pageToken", token.as_str())]);
            }

            let resp = req.send().await.map_err(|e| CalendarError::Http(e.to_string()))?;
            let status = resp.status();

            if status == reqwest::StatusCode::UNAUTHORIZED {
                return Err(CalendarError::Auth);
            }
            if !status.is_success() {
                let body = resp.text().await.unwrap_or_default();
                return Err(CalendarError::Api(format!("{status}: {body}")));
            }

            let list: GcalEventList =
                resp.json().await.map_err(|e| CalendarError::Parse(e.to_string()))?;

            for item in list.items.unwrap_or_default() {
                if let Some(ev) = gcal_item_to_event(item) {
                    events.push(ev);
                }
            }

            match list.next_page_token {
                Some(t) => page_token = Some(t),
                None => break,
            }
        }

        Ok(events)
    }

    async fn create_event(
        &self,
        event: NewCalendarEvent,
    ) -> Result<CalendarEvent, CalendarError> {
        let url = self.url(&format!("/calendars/{}/events", self.calendar_id));
        let body = GcalEventCreate {
            summary: event.title,
            description: Some(event.description),
            start: GcalTime::from_dt(event.start),
            end: GcalTime::from_dt(event.end),
        };

        let resp = self
            .client
            .post(&url)
            .header("Authorization", self.auth_header())
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| CalendarError::Http(e.to_string()))?;

        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(CalendarError::Auth);
        }
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(CalendarError::Api(format!("{status}: {text}")));
        }

        let item: GcalEventItem =
            resp.json().await.map_err(|e| CalendarError::Parse(e.to_string()))?;
        gcal_item_to_event(item).ok_or_else(|| CalendarError::Parse("missing fields".into()))
    }

    async fn update_event(
        &self,
        id: &str,
        event: UpdateCalendarEvent,
    ) -> Result<CalendarEvent, CalendarError> {
        // Fetch current event first so we can merge partial updates
        let get_url = self.url(&format!("/calendars/{}/events/{id}", self.calendar_id));
        let resp = self
            .client
            .get(&get_url)
            .header("Authorization", self.auth_header())
            .send()
            .await
            .map_err(|e| CalendarError::Http(e.to_string()))?;

        let status = resp.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(CalendarError::NotFound(id.to_string()));
        }
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(CalendarError::Auth);
        }
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(CalendarError::Api(format!("{status}: {text}")));
        }

        let mut current: GcalEventItem =
            resp.json().await.map_err(|e| CalendarError::Parse(e.to_string()))?;

        // Apply partial updates
        if let Some(title) = event.title {
            current.summary = Some(title);
        }
        if let Some(start) = event.start {
            current.start = GcalTime::from_dt(start);
        }
        if let Some(end) = event.end {
            current.end = GcalTime::from_dt(end);
        }
        if let Some(desc) = event.description {
            current.description = Some(desc);
        }

        let put_url = self.url(&format!("/calendars/{}/events/{id}", self.calendar_id));
        let resp = self
            .client
            .put(&put_url)
            .header("Authorization", self.auth_header())
            .header("Content-Type", "application/json")
            .json(&current)
            .send()
            .await
            .map_err(|e| CalendarError::Http(e.to_string()))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(CalendarError::Api(format!("{status}: {text}")));
        }

        let item: GcalEventItem =
            resp.json().await.map_err(|e| CalendarError::Parse(e.to_string()))?;
        gcal_item_to_event(item).ok_or_else(|| CalendarError::Parse("missing fields".into()))
    }

    async fn delete_event(&self, id: &str) -> Result<(), CalendarError> {
        let url = self.url(&format!("/calendars/{}/events/{id}", self.calendar_id));
        let resp = self
            .client
            .delete(&url)
            .header("Authorization", self.auth_header())
            .send()
            .await
            .map_err(|e| CalendarError::Http(e.to_string()))?;

        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(CalendarError::Auth);
        }
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(CalendarError::NotFound(id.to_string()));
        }
        if !status.is_success() && status != reqwest::StatusCode::NO_CONTENT {
            let text = resp.text().await.unwrap_or_default();
            return Err(CalendarError::Api(format!("{status}: {text}")));
        }
        Ok(())
    }
}

// ── GCal REST API types ───────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct GcalEventList {
    items: Option<Vec<GcalEventItem>>,
    #[serde(rename = "nextPageToken")]
    next_page_token: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct GcalEventItem {
    id: Option<String>,
    summary: Option<String>,
    description: Option<String>,
    start: GcalTime,
    end: GcalTime,
}

#[derive(Debug, Serialize, Deserialize)]
struct GcalEventCreate {
    summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    start: GcalTime,
    end: GcalTime,
}

#[derive(Debug, Serialize, Deserialize)]
struct GcalTime {
    #[serde(rename = "dateTime", skip_serializing_if = "Option::is_none")]
    date_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    date: Option<String>,
    #[serde(rename = "timeZone", skip_serializing_if = "Option::is_none")]
    time_zone: Option<String>,
}

impl GcalTime {
    fn from_dt(dt: DateTime<Utc>) -> Self {
        Self {
            date_time: Some(dt.to_rfc3339()),
            date: None,
            time_zone: Some("UTC".to_string()),
        }
    }

    fn to_datetime(&self) -> Option<DateTime<Utc>> {
        if let Some(dt_str) = &self.date_time {
            DateTime::parse_from_rfc3339(dt_str)
                .ok()
                .map(|dt| dt.with_timezone(&Utc))
        } else if let Some(date_str) = &self.date {
            chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d")
                .ok()
                .and_then(|d| d.and_hms_opt(0, 0, 0))
                .map(|ndt| Utc.from_utc_datetime(&ndt))
        } else {
            None
        }
    }
}

fn gcal_item_to_event(item: GcalEventItem) -> Option<CalendarEvent> {
    let id = item.id?;
    let title = item.summary.unwrap_or_default();
    let start = item.start.to_datetime()?;
    let end = item.end.to_datetime()?;
    Some(CalendarEvent { id, title, start, end, description: item.description })
}
