use serde::{Deserialize, Serialize};

/// A single maturity check definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaturityCheck {
    pub id: String,
    pub level: MaturityLevel,
    pub label: String,
    /// Whether this check has an automated fix available.
    pub fixable: bool,
}

impl MaturityCheck {
    pub fn new(id: &str, level: MaturityLevel, label: &str, fixable: bool) -> Self {
        Self { id: id.into(), level, label: label.into(), fixable }
    }
}

/// The six maturity levels a repository can reach.
///
/// Wire format: `"L0"`–`"L5"`. The variant labels (e.g. `Bootstrap`,
/// `Architected`) are mnemonic and may shift over time; the on-wire
/// identity is the level number only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum MaturityLevel {
    #[serde(rename = "L0")]
    L0Bootstrap,
    #[serde(rename = "L1")]
    L1Tested,
    #[serde(rename = "L2")]
    L2GitFlow,
    #[serde(rename = "L3")]
    L3Architected,
    #[serde(rename = "L4")]
    L4Skilled,
    #[serde(rename = "L5")]
    L5Orchestrated,
}

impl MaturityLevel {
    pub fn label(self) -> &'static str {
        match self {
            Self::L0Bootstrap => "L0 Bootstrap",
            Self::L1Tested => "L1 Tested",
            Self::L2GitFlow => "L2 GitFlow",
            Self::L3Architected => "L3 Architected",
            Self::L4Skilled => "L4 Skilled",
            Self::L5Orchestrated => "L5 Orchestrated",
        }
    }
}

impl std::fmt::Display for MaturityLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// Result of running a single maturity check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaturityCheckResult {
    pub check: MaturityCheck,
    pub passed: bool,
    /// Optional human-readable detail (e.g. which file was missing).
    pub detail: Option<String>,
}

/// Full maturity report for a repository.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaturityReport {
    pub results: Vec<MaturityCheckResult>,
    /// The highest level where every check passes.
    pub level: MaturityLevel,
}

impl MaturityReport {
    /// Compute the achieved maturity level from check results.
    pub fn compute(results: Vec<MaturityCheckResult>) -> Self {
        use MaturityLevel::*;
        let levels = [L0Bootstrap, L1Tested, L2GitFlow, L3Architected, L4Skilled, L5Orchestrated];
        let mut achieved = L0Bootstrap;
        for &lvl in levels.iter().skip(1) {
            let all_pass = results
                .iter()
                .filter(|r| r.check.level == lvl)
                .all(|r| r.passed);
            if all_pass {
                achieved = lvl;
            } else {
                break;
            }
        }
        Self { results, level: achieved }
    }

    pub fn failed_checks(&self) -> impl Iterator<Item = &MaturityCheckResult> {
        self.results.iter().filter(|r| !r.passed)
    }
}
