use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Stable wire error codes (§8 of dashboard-api.md).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    StaleCache,
    SourceUnavailable,
    NotFound,
    InvalidQuery,
    ProtocolMismatch,
    Unauthorized,
    Internal,
}

/// The `"error"` object inside a non-2xx body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiError {
    pub code: ErrorCode,
    pub message: String,
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stale_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Top-level error body `{"error": {...}}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiErrorBody {
    pub error: ApiError,
}

impl ApiErrorBody {
    pub fn not_found(message: impl Into<String>) -> Self {
        Self {
            error: ApiError {
                code: ErrorCode::NotFound,
                message: message.into(),
                retryable: false,
                context: None,
                stale_at: None,
            },
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            error: ApiError {
                code: ErrorCode::Internal,
                message: message.into(),
                retryable: false,
                context: None,
                stale_at: None,
            },
        }
    }

    pub fn source_unavailable(source: impl Into<String>, message: impl Into<String>) -> Self {
        let src = source.into();
        Self {
            error: ApiError {
                code: ErrorCode::SourceUnavailable,
                message: message.into(),
                retryable: true,
                context: Some(serde_json::json!({ "source": src })),
                stale_at: None,
            },
        }
    }
}

#[derive(Debug, Error)]
pub enum DashboardError {
    #[error("source unavailable: {name}")]
    SourceUnavailable { name: String },
    #[error("not found: {0}")]
    NotFound(String),
    #[error("internal error: {0}")]
    Internal(String),
}
