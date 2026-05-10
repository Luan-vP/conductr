//! Security sweep — surfaces security work the project owes itself.
//!
//! Initial implementation: shells out to `cargo audit --json` and converts
//! each vulnerability into a [`Finding`]. If `cargo-audit` is not installed,
//! emits a single low-severity finding suggesting installation.

use std::process::Stdio;

use async_trait::async_trait;
use serde::Deserialize;
use tokio::process::Command;

use crate::sweep::{Finding, Severity, Sweep, SweepContext};

#[derive(Debug, Default)]
pub struct SecuritySweep;

#[async_trait]
impl Sweep for SecuritySweep {
    fn name(&self) -> &'static str { "security" }

    async fn run(&self, ctx: &SweepContext) -> anyhow::Result<Vec<Finding>> {
        let result = Command::new("cargo")
            .args(["audit", "--json"])
            .current_dir(&ctx.repo_root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await;

        let out = match result {
            Ok(o) => o,
            Err(_) => return Ok(Vec::new()),
        };

        if !out.status.success() && out.stdout.is_empty() {
            // Likely cargo-audit isn't installed.
            return Ok(vec![Finding::new(
                "security",
                "tooling/cargo-audit-missing",
                "Install cargo-audit",
                "`cargo audit` exited non-zero with no output. Install with `cargo install cargo-audit` so the security sweep can report vulnerable deps.",
                Severity::Low,
            )]);
        }

        let parsed: Audit = match serde_json::from_slice(&out.stdout) {
            Ok(p) => p,
            Err(_) => return Ok(Vec::new()),
        };

        Ok(parsed
            .vulnerabilities
            .list
            .into_iter()
            .map(|v| {
                let id = v.advisory.id.clone();
                let title = format!("{} in {}", id, v.package.name);
                let body = format!(
                    "Advisory {} affects {} {}.\n\n{}\n\nMore: {}",
                    v.advisory.id,
                    v.package.name,
                    v.package.version,
                    v.advisory.title,
                    v.advisory.url.unwrap_or_default()
                );
                Finding::new("security", id.clone(), title, body, Severity::High)
                    .with_labels(["security".into(), "maintenance".into()])
            })
            .collect())
    }
}

#[derive(Debug, Deserialize)]
struct Audit {
    vulnerabilities: AuditVulns,
}

#[derive(Debug, Deserialize)]
struct AuditVulns {
    #[serde(default)]
    list: Vec<AuditVuln>,
}

#[derive(Debug, Deserialize)]
struct AuditVuln {
    advisory: AuditAdvisory,
    package: AuditPackage,
}

#[derive(Debug, Deserialize)]
struct AuditAdvisory {
    id: String,
    title: String,
    #[serde(default)]
    url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AuditPackage {
    name: String,
    version: String,
}
