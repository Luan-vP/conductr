//! llama.cpp HTTP adapter implementing [`LocalAgent`].
//!
//! Talks to the llama.cpp HTTP server's OpenAI-compatible endpoint
//! (`/v1/chat/completions`). The HTTP path is preferred over spawning a
//! `llama-cli` subprocess because it is stateless, requires no per-request
//! model load, and works against any server — local or remote — without
//! shell-escaping concerns.
//!
//! Configure the server address via the `LLAMACPP_HOST` environment variable
//! (default: `http://localhost:8080`), or pass an explicit URL to [`LlamaCpp::new`].

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use conductr_core::ports::{LocalAgent, LocalAgentError};

const DEFAULT_HOST: &str = "http://localhost:8080";

pub struct LlamaCpp {
    client: reqwest::Client,
    /// Base URL of the llama.cpp HTTP server (no trailing slash).
    pub host: String,
}

impl LlamaCpp {
    /// Build from `LLAMACPP_HOST` (falls back to `http://localhost:8080`).
    pub fn from_env() -> Self {
        let host =
            std::env::var("LLAMACPP_HOST").unwrap_or_else(|_| DEFAULT_HOST.to_string());
        Self::new(host)
    }

    /// Build with an explicit host URL (e.g. `"http://192.168.1.5:8080"`).
    pub fn new(host: impl Into<String>) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .expect("reqwest client builds");
        Self { client, host: host.into() }
    }
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<Message<'a>>,
    stream: bool,
}

#[derive(Serialize)]
struct Message<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: AssistantMessage,
}

#[derive(Deserialize)]
struct AssistantMessage {
    content: String,
}

#[async_trait]
impl LocalAgent for LlamaCpp {
    async fn complete(&self, prompt: &str) -> Result<String, LocalAgentError> {
        let url = format!("{}/v1/chat/completions", self.host);
        let body = ChatRequest {
            // llama.cpp ignores the model field; include it for spec compliance.
            model: "local",
            messages: vec![Message { role: "user", content: prompt }],
            stream: false,
        };

        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| LocalAgentError::Http(e.to_string()))?;

        let status = resp.status();
        let bytes = resp.bytes().await.map_err(|e| LocalAgentError::Http(e.to_string()))?;

        if !status.is_success() {
            return Err(LocalAgentError::Api {
                status: status.as_u16(),
                body: String::from_utf8_lossy(&bytes).into_owned(),
            });
        }

        let parsed: ChatResponse = serde_json::from_slice(&bytes)
            .map_err(|e| LocalAgentError::Parse(e.to_string()))?;

        parsed
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .ok_or_else(|| LocalAgentError::Parse("no choices in response".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_env_uses_default_host() {
        // SAFETY: single-threaded test process; acceptable for env-var manipulation.
        unsafe { std::env::remove_var("LLAMACPP_HOST") };
        let adapter = LlamaCpp::from_env();
        assert_eq!(adapter.host, DEFAULT_HOST);
    }

    #[test]
    fn new_with_explicit_host() {
        let adapter = LlamaCpp::new("http://myserver:9090");
        assert_eq!(adapter.host, "http://myserver:9090");
    }

    #[test]
    fn from_env_reads_custom_host() {
        // SAFETY: single-threaded test process.
        unsafe { std::env::set_var("LLAMACPP_HOST", "http://custom:1234") };
        let adapter = LlamaCpp::from_env();
        assert_eq!(adapter.host, "http://custom:1234");
        unsafe { std::env::remove_var("LLAMACPP_HOST") };
    }

    /// Integration test: requires a real `llama-server` at `LLAMACPP_HOST`.
    ///
    /// Run with:
    /// ```sh
    /// cargo test -p conductr-adapters --features llamacpp -- --ignored
    /// ```
    #[tokio::test]
    #[ignore]
    async fn integration_complete() {
        let adapter = LlamaCpp::from_env();
        let reply = adapter
            .complete("Say the single word 'pong' and nothing else.")
            .await
            .expect("complete should succeed against a live llama-server");
        assert!(
            reply.to_lowercase().contains("pong"),
            "expected 'pong' in reply, got: {reply}"
        );
    }
}
