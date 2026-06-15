//! Tempo write-back: append `[[tempo.prs]]` and `[[ci.runs]]` rows to `.conductr`.
//!
//! All writes for one orchestrate pass are collected and flushed in a single
//! call to `append_rows`, keeping the file coherent.

use std::collections::BTreeSet;
use std::path::Path;

use conductr_core::types::{CiRunRow, TempoPrRow};

/// Append new `[[tempo.prs]]` and `[[ci.runs]]` rows to the `.conductr` file
/// at `path`. Rows whose PR number is already present in the file are skipped
/// to avoid duplicates.
///
/// All rows are written in a single `fs::write` call (batched commit).
pub fn append_rows(path: &Path, pr_rows: &[TempoPrRow], ci_rows: &[CiRunRow]) -> anyhow::Result<()> {
    if pr_rows.is_empty() && ci_rows.is_empty() {
        return Ok(());
    }

    let existing = std::fs::read_to_string(path).unwrap_or_default();

    let existing_pr_numbers = parse_existing_numbers(&existing, "[[tempo.prs]]", "number");
    let existing_ci_prs = parse_existing_numbers(&existing, "[[ci.runs]]", "pr");

    let mut out = existing.trim_end_matches('\n').to_string();
    let mut changed = false;

    for row in pr_rows {
        if existing_pr_numbers.contains(&row.number) {
            continue;
        }
        out.push_str("\n\n[[tempo.prs]]\n");
        out.push_str(&format!("number     = {}\n", row.number));
        out.push_str(&format!("title      = {}\n", toml_str(&row.title)));
        if let Some(phrase) = &row.phrase {
            out.push_str(&format!("phrase     = {}\n", toml_str(phrase)));
        }
        if let Some(chord) = &row.chord {
            out.push_str(&format!("chord      = {}\n", toml_str(chord)));
        }
        out.push_str(&format!("complexity = {}\n", toml_str(row.complexity.as_str())));
        out.push_str(&format!(
            "opened     = {}\n",
            toml_str(&row.opened.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        ));
        out.push_str(&format!(
            "closed     = {}\n",
            toml_str(&row.closed.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        ));
        out.push_str(&format!("merged     = {}\n", row.merged));
        changed = true;
    }

    for row in ci_rows {
        if existing_ci_prs.contains(&row.pr) {
            continue;
        }
        out.push_str("\n\n[[ci.runs]]\n");
        out.push_str(&format!("pr      = {}\n", row.pr));
        out.push_str(&format!("minutes = {:.1}\n", row.minutes));
        out.push_str(&format!(
            "ts      = {}\n",
            toml_str(&row.ts.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        ));
        changed = true;
    }

    if changed {
        out.push('\n');
        std::fs::write(path, &out)?;
    }

    Ok(())
}

fn parse_existing_numbers(text: &str, section_header: &str, key: &str) -> BTreeSet<u64> {
    let mut nums = BTreeSet::new();
    let mut in_section = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed == section_header {
            in_section = true;
        } else if trimmed.starts_with("[[") {
            in_section = false;
        } else if in_section {
            if let Some(rest) = trimmed.strip_prefix(key) {
                let rest = rest.trim_start_matches([' ', '=']).trim();
                if let Ok(n) = rest.parse::<u64>() {
                    nums.insert(n);
                }
            }
        }
    }
    nums
}

fn toml_str(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use conductr_core::types::{CiRunRow, Complexity, PrNumber, TempoPrRow};
    use tempfile::NamedTempFile;

    fn now() -> chrono::DateTime<Utc> {
        Utc::now()
    }

    fn pr_row(number: PrNumber) -> TempoPrRow {
        TempoPrRow {
            number,
            title: format!("PR #{number}"),
            phrase: Some("begin".into()),
            chord: None,
            complexity: Complexity::M,
            opened: now(),
            closed: now(),
            merged: true,
        }
    }

    fn ci_row(pr: PrNumber) -> CiRunRow {
        CiRunRow { pr, minutes: 4.2, ts: now() }
    }

    #[test]
    fn appends_new_rows() {
        let f = NamedTempFile::new().unwrap();
        std::fs::write(f.path(), "project_tag = \"test\"\n").unwrap();

        append_rows(f.path(), &[pr_row(21)], &[ci_row(21)]).unwrap();

        let content = std::fs::read_to_string(f.path()).unwrap();
        assert!(content.contains("[[tempo.prs]]"));
        assert!(content.contains("number     = 21"));
        assert!(content.contains("[[ci.runs]]"));
        assert!(content.contains("pr      = 21"));
    }

    #[test]
    fn skips_duplicate_pr_numbers() {
        let f = NamedTempFile::new().unwrap();
        let initial = "project_tag = \"test\"\n\n[[tempo.prs]]\nnumber     = 21\ntitle      = \"old\"\n";
        std::fs::write(f.path(), initial).unwrap();

        append_rows(f.path(), &[pr_row(21)], &[]).unwrap();

        let content = std::fs::read_to_string(f.path()).unwrap();
        // Should still have exactly one [[tempo.prs]] block
        assert_eq!(content.matches("[[tempo.prs]]").count(), 1);
    }

    #[test]
    fn noop_when_empty() {
        let f = NamedTempFile::new().unwrap();
        let initial = "project_tag = \"test\"\n";
        std::fs::write(f.path(), initial).unwrap();

        append_rows(f.path(), &[], &[]).unwrap();

        let content = std::fs::read_to_string(f.path()).unwrap();
        assert_eq!(content, initial);
    }
}
