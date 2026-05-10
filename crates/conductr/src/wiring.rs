use anyhow::{Context, Result};
use conductr_adapters::{beads::Beads, notion::Notion};
use conductr_core::ports::IssueTracker;

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
