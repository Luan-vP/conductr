use anyhow::{Context, Result};
use conductr_adapters::{beads::Beads, notion::Notion};
use conductr_core::ports::{IssueTracker, LocalAgent};

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum TrackerKind {
    Beads,
    Notion,
}

impl TrackerKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Beads => "beads",
            Self::Notion => "notion",
        }
    }
}

impl Default for TrackerKind {
    fn default() -> Self {
        Self::Beads
    }
}

// ── LocalAgent wiring ─────────────────────────────────────────────────────────

/// The set of supported local-agent providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum LocalAgentKind {
    Ollama,
    #[value(name = "llamacpp")]
    LlamaCpp,
    Pi,
}

impl LocalAgentKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ollama => "ollama",
            Self::LlamaCpp => "llamacpp",
            Self::Pi => "pi",
        }
    }

    /// Parse from a string value (case-insensitive; accepts common aliases).
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "ollama" => Some(Self::Ollama),
            "llamacpp" | "llama-cpp" | "llama.cpp" | "llama_cpp" => Some(Self::LlamaCpp),
            "pi" => Some(Self::Pi),
            _ => None,
        }
    }
}

/// Construct a boxed `LocalAgent` for the given provider kind.
///
/// `model` is only used for Ollama; defaults to `"qwen3:27b"` when `None`.
/// The Pi variant returns a stub that immediately errors — the real adapter
/// is tracked in #57.
pub fn local_agent(kind: LocalAgentKind, model: Option<String>) -> Box<dyn LocalAgent> {
    match kind {
        LocalAgentKind::Ollama => {
            let m = model.unwrap_or_else(|| "qwen3:27b".to_string());
            Box::new(conductr_adapters::ollama::OllamaAdapter::new(m))
        }
        LocalAgentKind::LlamaCpp => Box::new(conductr_adapters::llamacpp::LlamaCpp::from_env()),
        LocalAgentKind::Pi => Box::new(conductr_adapters::mock::PiStub),
    }
}

// ── IssueTracker wiring ───────────────────────────────────────────────────────

pub fn issue_tracker(
    kind: TrackerKind,
    notion_database: Option<String>,
) -> Result<Box<dyn IssueTracker>> {
    match kind {
        TrackerKind::Beads => Ok(Box::new(Beads::new())),
        TrackerKind::Notion => {
            let db_id = notion_database
                .or_else(|| std::env::var("CONDUCTR_NOTION_DATABASE").ok())
                .context(
                    "--notion-database <id> or CONDUCTR_NOTION_DATABASE is required \
                     with --tracker notion",
                )?;
            let notion = Notion::from_env()
                .context("Notion auth failed: is NOTION_API_KEY set?")?
                .with_database(db_id);
            Ok(Box::new(notion))
        }
    }
}
