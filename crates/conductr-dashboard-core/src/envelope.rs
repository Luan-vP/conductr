use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Protocol version string, e.g. "1.0.0".
pub const PROTOCOL_VERSION: &str = "1.0.0";

/// Every REST response is wrapped in this envelope (§4 of dashboard-api.md).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope<T> {
    pub protocol: String,
    #[serde(rename = "impl")]
    pub impl_version: String,
    pub host: String,
    pub snapshot_at: DateTime<Utc>,
    pub data: T,
}

impl<T> Envelope<T> {
    pub fn new(impl_version: impl Into<String>, host: impl Into<String>, data: T) -> Self {
        Self {
            protocol: PROTOCOL_VERSION.into(),
            impl_version: impl_version.into(),
            host: host.into(),
            snapshot_at: Utc::now(),
            data,
        }
    }
}

/// Response from `GET /version` — no envelope, just versions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionResponse {
    pub protocol: String,
    #[serde(rename = "impl")]
    pub impl_version: String,
}
