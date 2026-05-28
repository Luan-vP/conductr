use std::path::Path;

use anyhow::Result;
use conductr_core::{MaturityLevel, MaturityReport};

use crate::{checks, fixes};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WizardMode {
    Interactive,
    NonInteractive,
    DryRun,
}

/// Run the setup wizard against `repo`.
///
/// - `DryRun`: print what would be done, touch nothing.
/// - `NonInteractive`: apply all fixable checks automatically.
/// - `Interactive`: prompt before each fix (falls back to NonInteractive if
///   stdin is not a TTY).
pub async fn run(repo: &Path, mode: WizardMode) -> Result<MaturityReport> {
    let results = checks::run_all(repo);
    let report = MaturityReport::compute(results);

    println!("=== Conductr Setup Wizard ===");
    println!("Repo:  {}", repo.display());
    println!("Level: {}", report.level);
    println!();

    print_report(&report);

    let failing: Vec<_> = report.failed_checks().collect();
    if failing.is_empty() {
        println!("All checks pass — repo is at {}.", report.level);
        return Ok(report);
    }

    let dry_run = mode == WizardMode::DryRun;
    println!();
    if dry_run {
        println!("--- Fixes (dry-run) ---");
    } else {
        println!("--- Applying fixes ---");
    }

    for result in &failing {
        if !result.check.fixable {
            if dry_run {
                println!(
                    "  [{}] {} — not fixable automatically",
                    result.check.id, result.check.label
                );
            }
            continue;
        }

        let apply = match mode {
            WizardMode::DryRun => true,
            WizardMode::NonInteractive => true,
            WizardMode::Interactive => prompt_yes_no(&format!(
                "Apply fix for '{}'?",
                result.check.label
            )),
        };

        if !apply {
            println!("  skipped: {}", result.check.label);
            continue;
        }

        apply_fix(&result.check.id, repo, dry_run)?;
    }

    // Re-run checks to reflect applied fixes.
    let updated_results = checks::run_all(repo);
    let updated_report = MaturityReport::compute(updated_results);

    if !dry_run {
        println!();
        println!("--- After fixes ---");
        print_report(&updated_report);

        if updated_report.level >= MaturityLevel::L5Orchestrated {
            println!("Repo is fully orchestrated at {}!", updated_report.level);
        } else {
            println!(
                "Repo is now at {}. Re-run the wizard after completing manual steps.",
                updated_report.level
            );
        }
    }

    Ok(updated_report)
}

fn print_report(report: &MaturityReport) {
    use conductr_core::MaturityLevel::*;
    let levels = [L1Tested, L2GitFlow, L3Architected, L4Skilled, L5Orchestrated];
    for &lvl in &levels {
        println!("  {}:", lvl);
        for r in report.results.iter().filter(|r| r.check.level == lvl) {
            let icon = if r.passed { "✓" } else { "✗" };
            let fix_tag = if !r.passed && r.check.fixable { " [fixable]" } else { "" };
            println!("    {} {}{}", icon, r.check.label, fix_tag);
            if let Some(detail) = &r.detail {
                println!("      → {}", detail);
            }
        }
    }
}

fn apply_fix(id: &str, repo: &Path, dry_run: bool) -> Result<()> {
    match id {
        "ci-workflow" => fixes::add_ci_workflow(repo, dry_run),
        "gitignore-target" => fixes::add_gitignore_target(repo, dry_run),
        "gitignore-conductr-local" => fixes::add_gitignore_conductr_local(repo, dry_run),
        "dev-branch" => fixes::init_git_flow(repo, dry_run),
        "codeowners" => fixes::add_codeowners(repo, dry_run),
        "claude-app" => fixes::install_claude_app(repo, dry_run),
        "claude-workflow" => fixes::add_claude_workflow(repo, dry_run),
        "skills-installed" => fixes::install_skills(repo, dry_run, false),
        other => {
            println!("  [no fix registered for '{other}']");
            Ok(())
        }
    }
}

fn prompt_yes_no(question: &str) -> bool {
    use std::io::{self, BufRead, Write};
    print!("{question} [y/N] ");
    io::stdout().flush().ok();
    let stdin = io::stdin();
    let line = stdin.lock().lines().next().and_then(|l| l.ok()).unwrap_or_default();
    matches!(line.trim().to_lowercase().as_str(), "y" | "yes")
}
