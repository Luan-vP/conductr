//! Ollama adapter implementing [`LocalAgent`].
//!
//! Talks to a running `ollama serve` process over HTTP.
//! Set `OLLAMA_HOST` to override the default `http://localhost:11434`.

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use conductr_core::ports::{LocalAgent, LocalAgentError};

const DEFAULT_HOST: &str = "http://localhost:11434";
const TIMEOUT_SECS: u64 = 300;

#[derive(Debug, Clone)]
pub struct OllamaAdapter {
    client: reqwest::Client,
    pub model: String,
    pub host: String,
}

#[derive(Serialize)]
struct GenerateRequest<'a> {
    model: &'a str,
    prompt: &'a str,
    stream: bool,
}

#[derive(Deserialize)]
struct GenerateResponse {
    response: String,
}

impl OllamaAdapter {
    pub fn new(model: impl Into<String>) -> Self {
        let host =
            std::env::var("OLLAMA_HOST").unwrap_or_else(|_| DEFAULT_HOST.to_string());
        Self::with_host(model, host)
    }

    pub fn with_host(model: impl Into<String>, host: impl Into<String>) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(TIMEOUT_SECS))
            .build()
            .expect("reqwest client builds");
        Self { client, model: model.into(), host: host.into() }
    }

    /// Verify connectivity by calling `GET /api/tags`.
    pub async fn ping(&self) -> Result<(), LocalAgentError> {
        let url = format!("{}/api/tags", self.host);
        self.client
            .get(&url)
            .send()
            .await
            .map_err(|_| LocalAgentError::NotRunning)?;
        Ok(())
    }
}

#[async_trait]
impl LocalAgent for OllamaAdapter {
    async fn complete(&self, prompt: &str) -> Result<String, LocalAgentError> {
        let url = format!("{}/api/generate", self.host);
        let body = GenerateRequest { model: &self.model, prompt, stream: false };

        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| LocalAgentError::Http(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let text = resp.text().await.unwrap_or_default();
            return Err(LocalAgentError::Http(format!("{status}: {text}")));
        }

        let gen: GenerateResponse =
            resp.json().await.map_err(|e| LocalAgentError::Parse(e.to_string()))?;
        Ok(gen.response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_host_from_env() {
        let prev = std::env::var("OLLAMA_HOST").ok();
        unsafe { std::env::remove_var("OLLAMA_HOST") };
        let a = OllamaAdapter::new("llama3");
        assert_eq!(a.host, DEFAULT_HOST);
        if let Some(v) = prev {
            unsafe { std::env::set_var("OLLAMA_HOST", v) };
        }
    }

    #[test]
    fn custom_host_overrides_default() {
        let a = OllamaAdapter::with_host("llama3", "http://myhost:11434");
        assert_eq!(a.host, "http://myhost:11434");
        assert_eq!(a.model, "llama3");
    }
}
