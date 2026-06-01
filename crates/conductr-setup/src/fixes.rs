use std::path::Path;

use anyhow::{Context, Result};

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

/// Symlink every `skills/<name>/` (containing a `SKILL.md`) into `~/.claude/skills/<name>`.
///
/// - `dry_run`: print the plan without touching the filesystem.
/// - `force`: overwrite conflicting symlinks (never overwrites real directories).
pub fn install_skills(repo: &Path, dry_run: bool, force: bool) -> Result<()> {
    let home = std::env::var("HOME")
        .map(std::path::PathBuf::from)
        .context("$HOME is not set")?;
    let claude_skills = home.join(".claude").join("skills");
    install_skills_to(repo, &claude_skills, dry_run, force)
}

/// Same as [`install_skills`] but with an explicit target directory for `~/.claude/skills/`.
/// Used directly by tests and by the CLI subcommand when an override is needed.
pub fn install_skills_to(
    repo: &Path,
    claude_skills: &Path,
    dry_run: bool,
    force: bool,
) -> Result<()> {
    let claude_dir = claude_skills.parent().unwrap_or(claude_skills);

    if !claude_dir.exists() {
        if dry_run {
            println!("[dry-run] note: {} does not exist — is Claude Code installed?", claude_dir.display());
        } else {
            println!("warning: {} does not exist — is Claude Code installed?", claude_dir.display());
        }
    }

    if dry_run {
        if !claude_skills.exists() {
            println!("[dry-run] would create {}", claude_skills.display());
        }
    } else {
        std::fs::create_dir_all(claude_skills)
            .with_context(|| format!("creating {}", claude_skills.display()))?;
    }

    let skills_dir = repo.join("skills");
    let entries: Vec<_> = skills_dir
        .read_dir()
        .with_context(|| format!("reading {}", skills_dir.display()))?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().join("SKILL.md").exists())
        .collect();

    let mut linked = 0usize;
    let mut skipped = 0usize;
    let mut warned = 0usize;

    for entry in &entries {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        let skill_abs = entry
            .path()
            .canonicalize()
            .unwrap_or_else(|_| entry.path());
        let link_path = claude_skills.join(&name);

        match std::fs::symlink_metadata(&link_path) {
            Err(_) => {
                // Does not exist — create symlink.
                if dry_run {
                    println!("[dry-run] would link {} → {}", link_path.display(), skill_abs.display());
                } else {
                    create_symlink(&skill_abs, &link_path)
                        .with_context(|| format!("symlinking {name_str}"))?;
                    println!("linked: {name_str}");
                }
                linked += 1;
            }
            Ok(meta) if meta.file_type().is_symlink() => {
                let target = std::fs::read_link(&link_path)
                    .with_context(|| format!("reading symlink {}", link_path.display()))?;
                let abs_target = if target.is_absolute() {
                    target.clone()
                } else {
                    link_path.parent().unwrap_or(Path::new("/")).join(&target)
                };
                let canonical = abs_target.canonicalize().unwrap_or(abs_target);

                if canonical == skill_abs {
                    println!("ok (already linked): {name_str}");
                    skipped += 1;
                } else if force {
                    if dry_run {
                        println!(
                            "[dry-run] would re-link {} → {} (replacing: {})",
                            link_path.display(),
                            skill_abs.display(),
                            target.display()
                        );
                    } else {
                        std::fs::remove_file(&link_path)
                            .with_context(|| format!("removing old symlink for {name_str}"))?;
                        create_symlink(&skill_abs, &link_path)
                            .with_context(|| format!("re-symlinking {name_str}"))?;
                        println!("re-linked: {name_str}");
                    }
                    linked += 1;
                } else {
                    println!(
                        "warning: {name_str} symlink points to {} — skipping (use --force to override)",
                        target.display()
                    );
                    warned += 1;
                }
            }
            Ok(_) => {
                println!("warning: {name_str} is a real directory/file; refusing to overwrite");
                warned += 1;
            }
        }
    }

    println!(
        "\n{linked} skill(s) linked, {skipped} skipped (already correct), {warned} warned (conflict)"
    );
    Ok(())
}

#[cfg(unix)]
fn create_symlink(src: &Path, dst: &Path) -> Result<()> {
    std::os::unix::fs::symlink(src, dst)?;
    Ok(())
}

#[cfg(not(unix))]
fn create_symlink(_src: &Path, dst: &Path) -> Result<()> {
    anyhow::bail!(
        "symlink creation is not supported on this platform; \
         manually link {} into ~/.claude/skills/",
        dst.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn make_skill(repo: &Path, name: &str) {
        let dir = repo.join("skills").join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("SKILL.md"), format!("# {name}")).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn links_new_skill() {
        let tmp = TempDir::new().unwrap();
        make_skill(tmp.path(), "idle");
        let claude_skills = tmp.path().join("claude").join("skills");
        fs::create_dir_all(&claude_skills).unwrap();
        install_skills_to(tmp.path(), &claude_skills, false, false).unwrap();
        assert!(claude_skills.join("idle").is_symlink());
    }

    #[cfg(unix)]
    #[test]
    fn idempotent_on_correct_symlink() {
        let tmp = TempDir::new().unwrap();
        make_skill(tmp.path(), "idle");
        let claude_skills = tmp.path().join("claude").join("skills");
        fs::create_dir_all(&claude_skills).unwrap();
        install_skills_to(tmp.path(), &claude_skills, false, false).unwrap();
        install_skills_to(tmp.path(), &claude_skills, false, false).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn dry_run_creates_no_symlinks() {
        let tmp = TempDir::new().unwrap();
        make_skill(tmp.path(), "idle");
        let claude_skills = tmp.path().join("claude").join("skills");
        fs::create_dir_all(&claude_skills).unwrap();
        install_skills_to(tmp.path(), &claude_skills, true, false).unwrap();
        assert!(!claude_skills.join("idle").exists(), "dry-run must not create symlinks");
    }

    #[cfg(unix)]
    #[test]
    fn conflicting_symlink_skipped_without_force() {
        let tmp = TempDir::new().unwrap();
        make_skill(tmp.path(), "idle");
        let claude_skills = tmp.path().join("claude").join("skills");
        fs::create_dir_all(&claude_skills).unwrap();
        let other = tmp.path().join("other");
        fs::create_dir_all(&other).unwrap();
        std::os::unix::fs::symlink(&other, claude_skills.join("idle")).unwrap();
        install_skills_to(tmp.path(), &claude_skills, false, false).unwrap();
        let target = std::fs::read_link(claude_skills.join("idle")).unwrap();
        assert_eq!(target, other);
    }

    #[cfg(unix)]
    #[test]
    fn conflicting_symlink_replaced_with_force() {
        let tmp = TempDir::new().unwrap();
        make_skill(tmp.path(), "idle");
        let claude_skills = tmp.path().join("claude").join("skills");
        fs::create_dir_all(&claude_skills).unwrap();
        let other = tmp.path().join("other");
        fs::create_dir_all(&other).unwrap();
        std::os::unix::fs::symlink(&other, claude_skills.join("idle")).unwrap();
        install_skills_to(tmp.path(), &claude_skills, false, true).unwrap();
        let target = std::fs::read_link(claude_skills.join("idle")).unwrap();
        let expected = tmp.path().join("skills").join("idle").canonicalize().unwrap();
        assert_eq!(target.canonicalize().unwrap(), expected);
    }

    #[cfg(unix)]
    #[test]
    fn real_dir_not_overwritten() {
        let tmp = TempDir::new().unwrap();
        make_skill(tmp.path(), "idle");
        let claude_skills = tmp.path().join("claude").join("skills");
        fs::create_dir_all(claude_skills.join("idle")).unwrap();
        install_skills_to(tmp.path(), &claude_skills, false, true).unwrap();
        assert!(
            !claude_skills.join("idle").is_symlink(),
            "real directory must not be overwritten even with force"
        );
    }

    #[cfg(unix)]
    #[test]
    fn skill_dir_without_skill_md_skipped() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("skills").join("noskilldoc")).unwrap();
        let claude_skills = tmp.path().join("claude").join("skills");
        fs::create_dir_all(&claude_skills).unwrap();
        install_skills_to(tmp.path(), &claude_skills, false, false).unwrap();
        assert!(!claude_skills.join("noskilldoc").exists());
    }
}
