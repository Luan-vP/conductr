use std::time::Duration;

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ProbeStatus {
    Present,
    Missing,
    Unreachable,
}

impl ProbeStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            ProbeStatus::Present => "present",
            ProbeStatus::Missing => "missing",
            ProbeStatus::Unreachable => "unreachable",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ProbeRow {
    pub provider: &'static str,
    pub status: ProbeStatus,
    pub detail: String,
}

const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

pub async fn run_all_probes() -> Vec<ProbeRow> {
    let (ollama, llamacpp, qwen3, pi) =
        tokio::join!(probe_ollama(), probe_llamacpp(), probe_qwen3_27b(), probe_pi());
    vec![ollama, llamacpp, qwen3, pi]
}

fn make_client() -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder().timeout(PROBE_TIMEOUT).build()
}

async fn probe_ollama() -> ProbeRow {
    let host = std::env::var("OLLAMA_HOST")
        .unwrap_or_else(|_| "http://localhost:11434".to_string());
    let url = format!("{}/api/tags", host.trim_end_matches('/'));

    let client = match make_client() {
        Ok(c) => c,
        Err(e) => {
            return ProbeRow {
                provider: "ollama",
                status: ProbeStatus::Unreachable,
                detail: format!("failed to build http client: {e}"),
            }
        }
    };

    match client.get(&url).send().await {
        Ok(resp) if resp.status().is_success() => ProbeRow {
            provider: "ollama",
            status: ProbeStatus::Present,
            detail: format!("running at {host}"),
        },
        Ok(resp) => ProbeRow {
            provider: "ollama",
            status: ProbeStatus::Unreachable,
            detail: format!("HTTP {}", resp.status()),
        },
        Err(e) if e.is_connect() => ProbeRow {
            provider: "ollama",
            status: ProbeStatus::Missing,
            detail: "not running (connection refused)".to_string(),
        },
        Err(e) => ProbeRow {
            provider: "ollama",
            status: ProbeStatus::Unreachable,
            detail: e.to_string(),
        },
    }
}

async fn probe_llamacpp() -> ProbeRow {
    let host = std::env::var("LLAMACPP_HOST")
        .unwrap_or_else(|_| "http://localhost:8080".to_string());
    let url = format!("{}/health", host.trim_end_matches('/'));

    let client = match make_client() {
        Ok(c) => c,
        Err(e) => {
            return ProbeRow {
                provider: "llamacpp",
                status: ProbeStatus::Unreachable,
                detail: format!("failed to build http client: {e}"),
            }
        }
    };

    match client.get(&url).send().await {
        Ok(resp) if resp.status().is_success() => ProbeRow {
            provider: "llamacpp",
            status: ProbeStatus::Present,
            detail: format!("running at {host}"),
        },
        Ok(resp) => ProbeRow {
            provider: "llamacpp",
            status: ProbeStatus::Unreachable,
            detail: format!("HTTP {}", resp.status()),
        },
        Err(e) if e.is_connect() => ProbeRow {
            provider: "llamacpp",
            status: ProbeStatus::Missing,
            detail: "not running (connection refused)".to_string(),
        },
        Err(e) => ProbeRow {
            provider: "llamacpp",
            status: ProbeStatus::Unreachable,
            detail: e.to_string(),
        },
    }
}

async fn probe_qwen3_27b() -> ProbeRow {
    let host = std::env::var("OLLAMA_HOST")
        .unwrap_or_else(|_| "http://localhost:11434".to_string());
    let url = format!("{}/api/tags", host.trim_end_matches('/'));

    let client = match make_client() {
        Ok(c) => c,
        Err(e) => {
            return ProbeRow {
                provider: "qwen3:27b",
                status: ProbeStatus::Unreachable,
                detail: format!("failed to build http client: {e}"),
            }
        }
    };

    match client.get(&url).send().await {
        Ok(resp) if resp.status().is_success() => match resp.json::<serde_json::Value>().await {
            Ok(body) => {
                let found = body["models"]
                    .as_array()
                    .map(|models| {
                        models.iter().any(|m| {
                            m["name"]
                                .as_str()
                                .map(|n| n.starts_with("qwen3:27b") || n.starts_with("qwen3-27b"))
                                .unwrap_or(false)
                        })
                    })
                    .unwrap_or(false);

                if found {
                    ProbeRow {
                        provider: "qwen3:27b",
                        status: ProbeStatus::Present,
                        detail: "model available in ollama".to_string(),
                    }
                } else {
                    ProbeRow {
                        provider: "qwen3:27b",
                        status: ProbeStatus::Missing,
                        detail: "not in ollama model list (run: ollama pull qwen3:27b)".to_string(),
                    }
                }
            }
            Err(e) => ProbeRow {
                provider: "qwen3:27b",
                status: ProbeStatus::Unreachable,
                detail: format!("could not parse ollama /api/tags response: {e}"),
            },
        },
        Ok(resp) => ProbeRow {
            provider: "qwen3:27b",
            status: ProbeStatus::Unreachable,
            detail: format!("ollama returned HTTP {}", resp.status()),
        },
        Err(e) if e.is_connect() => ProbeRow {
            provider: "qwen3:27b",
            status: ProbeStatus::Missing,
            detail: "ollama not running".to_string(),
        },
        Err(e) => ProbeRow {
            provider: "qwen3:27b",
            status: ProbeStatus::Unreachable,
            detail: e.to_string(),
        },
    }
}

async fn probe_pi() -> ProbeRow {
    // Placeholder — real probe to be filled in once Pi's API surface is confirmed.
    // The implementing PR should update this function and remove this comment.
    ProbeRow {
        provider: "pi-agent",
        status: ProbeStatus::Missing,
        detail: "probe not yet implemented — update when Pi API surface is confirmed".to_string(),
    }
}
