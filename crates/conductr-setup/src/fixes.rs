use std::path::Path;

use anyhow::Result;

/// Writes a minimal GitHub Actions CI workflow that runs `cargo test`.
pub fn add_ci_workflow(repo: &Path, dry_run: bool) -> Result<()> {
    let dir = repo.join(".github/workflows");
    let path = dir.join("ci.yml");
    let content = r#"name: CI
on:
  push:
    branches: ["**"]
  pull_request:

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Run tests
        run: cargo test --workspace
"#;
    if dry_run {
        println!("[dry-run] would write {}", path.display());
    } else {
        std::fs::create_dir_all(&dir)?;
        std::fs::write(&path, content)?;
        println!("wrote {}", path.display());
    }
    Ok(())
}

/// Adds `target/` to `.gitignore` if not already present.
pub fn add_gitignore_target(repo: &Path, dry_run: bool) -> Result<()> {
    let path = repo.join(".gitignore");
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    if existing.lines().any(|l| l.trim() == "target/" || l.trim() == "target") {
        println!(".gitignore already covers target/");
        return Ok(());
    }
    if dry_run {
        println!("[dry-run] would append 'target/' to {}", path.display());
    } else {
        let mut content = existing;
        if !content.ends_with('\n') && !content.is_empty() {
            content.push('\n');
        }
        content.push_str("target/\n");
        std::fs::write(&path, content)?;
        println!("appended 'target/' to {}", path.display());
    }
    Ok(())
}

/// Creates a `dev` branch from the current HEAD (local only).
pub fn init_git_flow(repo: &Path, dry_run: bool) -> Result<()> {
    if dry_run {
        println!("[dry-run] would create dev branch from HEAD in {}", repo.display());
        return Ok(());
    }
    let output = std::process::Command::new("git")
        .args(["checkout", "-b", "dev"])
        .current_dir(repo)
        .output()?;
    if output.status.success() {
        println!("created dev branch");
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("already exists") {
            println!("dev branch already exists");
        } else {
            anyhow::bail!("git checkout -b dev failed: {}", stderr);
        }
    }
    Ok(())
}

/// Writes a minimal CODEOWNERS file.
pub fn add_codeowners(repo: &Path, dry_run: bool) -> Result<()> {
    let path = repo.join("CODEOWNERS");
    let content = "# CODEOWNERS\n# https://docs.github.com/en/repositories/managing-your-repositorys-settings-and-features/customizing-your-repository/about-code-owners\n\n* @maintainers\n";
    if dry_run {
        println!("[dry-run] would write {}", path.display());
    } else {
        std::fs::write(&path, content)?;
        println!("wrote {}", path.display());
    }
    Ok(())
}

/// Prints the Claude GitHub App install URL and follow-up instructions.
/// Never auto-installs — that would require browser auth.
pub fn install_claude_app(_repo: &Path, dry_run: bool) -> Result<()> {
    let url = "https://github.com/apps/claude";
    if dry_run {
        println!("[dry-run] would open {url}");
        println!("  After installing, re-run `conductr setup status` to verify.");
    } else {
        println!("Open the following URL in your browser to install the Claude GitHub App:");
        println!("  {url}");
        println!();
        println!("After installing:");
        println!("  1. Grant access to this repository.");
        println!("  2. Re-run `conductr setup status` to verify L5 checks.");
    }
    Ok(())
}

/// Adds `.conductr-local` to `.gitignore` if not already present.
pub fn add_gitignore_conductr_local(repo: &Path, dry_run: bool) -> Result<()> {
    let path = repo.join(".gitignore");
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    if existing.lines().any(|l| l.trim() == ".conductr-local") {
        println!(".gitignore already covers .conductr-local");
        return Ok(());
    }
    if dry_run {
        println!("[dry-run] would append '.conductr-local' to {}", path.display());
    } else {
        let mut content = existing;
        if !content.ends_with('\n') && !content.is_empty() {
            content.push('\n');
        }
        content.push_str(".conductr-local\n");
        std::fs::write(&path, content)?;
        println!("appended '.conductr-local' to {}", path.display());
    }
    Ok(())
}

/// Writes the `.github/workflows/claude.yml` workflow.
pub fn add_claude_workflow(repo: &Path, dry_run: bool) -> Result<()> {
    let dir = repo.join(".github/workflows");
    let path = dir.join("claude.yml");
    let content = r#"name: Claude
on:
  issue_comment:
    types: [created]
  pull_request_review_comment:
    types: [created]
  issues:
    types: [opened, assigned]
  pull_request_review:
    types: [submitted]

jobs:
  claude:
    if: |
      (github.event_name == 'issue_comment' && contains(github.event.comment.body, '@claude')) ||
      (github.event_name == 'pull_request_review_comment' && contains(github.event.comment.body, '@claude')) ||
      (github.event_name == 'pull_request_review' && contains(github.event.review.body, '@claude')) ||
      (github.event_name == 'issues' && (contains(github.event.issue.body, '@claude') || contains(github.event.issue.title, '@claude')))
    runs-on: ubuntu-latest
    permissions:
      contents: write
      pull-requests: write
      issues: write
      id-token: write
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 1
      - uses: anthropics/claude-code-action@v1
        with:
          anthropic_api_key: ${{ secrets.ANTHROPIC_API_KEY }}
"#;
    if dry_run {
        println!("[dry-run] would write {}", path.display());
    } else {
        std::fs::create_dir_all(&dir)?;
        std::fs::write(&path, content)?;
        println!("wrote {}", path.display());
    }
    Ok(())
}
