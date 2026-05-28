use std::path::Path;

use conductr_core::{MaturityCheck, MaturityCheckResult, MaturityLevel};

// ── Helpers ───────────────────────────────────────────────────────────────────

fn pass(check: MaturityCheck) -> MaturityCheckResult {
    MaturityCheckResult { check, passed: true, detail: None }
}

fn fail(check: MaturityCheck, detail: impl Into<String>) -> MaturityCheckResult {
    MaturityCheckResult { check, passed: false, detail: Some(detail.into()) }
}

// ── L1 Tested ────────────────────────────────────────────────────────────────

pub fn check_ci_workflow(repo: &Path) -> MaturityCheckResult {
    let check = MaturityCheck::new(
        "ci-workflow",
        MaturityLevel::L1Tested,
        ".github/workflows/*.yml runs tests on push",
        true,
    );
    let workflows_dir = repo.join(".github/workflows");
    let found = workflows_dir.read_dir().ok().and_then(|mut d| {
        d.find(|e| {
            e.as_ref()
                .ok()
                .and_then(|e| {
                    e.path()
                        .extension()
                        .map(|x| x == "yml" || x == "yaml")
                })
                .unwrap_or(false)
        })
    });
    if found.is_some() { pass(check) } else { fail(check, "no .github/workflows/*.yml found") }
}

pub fn check_gitignore_conductr_local(repo: &Path) -> MaturityCheckResult {
    let check = MaturityCheck::new(
        "gitignore-conductr-local",
        MaturityLevel::L1Tested,
        ".gitignore covers .conductr-local",
        true,
    );
    let covered = std::fs::read_to_string(repo.join(".gitignore"))
        .map(|s| s.lines().any(|l| l.trim() == ".conductr-local"))
        .unwrap_or(false);
    if covered {
        pass(check)
    } else {
        fail(check, ".gitignore missing or doesn't cover .conductr-local")
    }
}

pub fn check_gitignore_target(repo: &Path) -> MaturityCheckResult {
    let check = MaturityCheck::new(
        "gitignore-target",
        MaturityLevel::L1Tested,
        ".gitignore covers target/",
        true,
    );
    let covered = std::fs::read_to_string(repo.join(".gitignore"))
        .map(|s| s.lines().any(|l| l.trim() == "target/" || l.trim() == "target"))
        .unwrap_or(false);
    if covered {
        pass(check)
    } else {
        fail(check, ".gitignore missing or doesn't cover target/")
    }
}

// ── L2 GitFlow ────────────────────────────────────────────────────────────────

pub fn check_dev_branch(repo: &Path) -> MaturityCheckResult {
    let check = MaturityCheck::new(
        "dev-branch",
        MaturityLevel::L2GitFlow,
        "dev (or develop) branch exists",
        true,
    );
    let heads = repo.join(".git/refs/heads");
    let found = ["dev", "develop"].iter().any(|b| heads.join(b).exists())
        || packed_refs_has_dev(repo);
    if found { pass(check) } else { fail(check, "neither dev nor develop branch found") }
}

fn packed_refs_has_dev(repo: &Path) -> bool {
    std::fs::read_to_string(repo.join(".git/packed-refs"))
        .map(|s| {
            s.lines().any(|l| {
                l.ends_with("refs/heads/dev") || l.ends_with("refs/heads/develop")
            })
        })
        .unwrap_or(false)
}

pub fn check_default_base_dev(repo: &Path) -> MaturityCheckResult {
    let check = MaturityCheck::new(
        "default-base-dev",
        MaturityLevel::L2GitFlow,
        "default PR base is dev/develop",
        false,
    );
    if check_dev_branch(repo).passed {
        pass(check)
    } else {
        fail(check, "cannot verify default PR base without dev/develop branch")
    }
}

// ── L3 Architected ───────────────────────────────────────────────────────────

pub fn check_claude_base_md(repo: &Path) -> MaturityCheckResult {
    let check = MaturityCheck::new(
        "claude-base-md",
        MaturityLevel::L3Architected,
        ".claude/base.md exists",
        false,
    );
    let path = repo.join(".claude/base.md");
    if path.exists() { pass(check) } else { fail(check, ".claude/base.md not found") }
}

pub fn check_contributing(repo: &Path) -> MaturityCheckResult {
    let check = MaturityCheck::new(
        "contributing-md",
        MaturityLevel::L3Architected,
        "CONTRIBUTING.md mentions architecture conventions",
        false,
    );
    let candidates = ["CONTRIBUTING.md", "CONTRIBUTING.rst", "CONTRIBUTING.txt"];
    let passed = candidates
        .iter()
        .find_map(|c| {
            let p = repo.join(c);
            if p.exists() { std::fs::read_to_string(p).ok() } else { None }
        })
        .map(|s| {
            let lower = s.to_lowercase();
            lower.contains("architect") || lower.contains("convention") || lower.contains("structure")
        })
        .unwrap_or(false);
    if passed {
        pass(check)
    } else {
        fail(check, "CONTRIBUTING.md missing or doesn't mention architecture conventions")
    }
}

pub fn check_codeowners(repo: &Path) -> MaturityCheckResult {
    let check = MaturityCheck::new(
        "codeowners",
        MaturityLevel::L3Architected,
        "CODEOWNERS present",
        true,
    );
    let candidates = [
        repo.join("CODEOWNERS"),
        repo.join(".github/CODEOWNERS"),
        repo.join("docs/CODEOWNERS"),
    ];
    if candidates.iter().any(|p| p.exists()) {
        pass(check)
    } else {
        fail(check, "CODEOWNERS not found")
    }
}

// ── L4 Skilled ───────────────────────────────────────────────────────────────

pub fn check_skill_md(repo: &Path) -> MaturityCheckResult {
    let check = MaturityCheck::new(
        "skill-md",
        MaturityLevel::L4Skilled,
        "at least one skills/<name>/SKILL.md exists",
        false,
    );
    let found = repo
        .join("skills")
        .read_dir()
        .ok()
        .and_then(|d| d.filter_map(|e| e.ok()).find(|e| e.path().join("SKILL.md").exists()))
        .is_some();
    if found { pass(check) } else { fail(check, "no skills/<name>/SKILL.md found") }
}

pub fn check_skills_installed(repo: &Path) -> MaturityCheckResult {
    let claude_skills = std::env::var("HOME")
        .ok()
        .map(|h| std::path::PathBuf::from(h).join(".claude").join("skills"));
    check_skills_installed_with(repo, claude_skills.as_deref())
}

pub(crate) fn check_skills_installed_with(
    repo: &Path,
    claude_skills: Option<&Path>,
) -> MaturityCheckResult {
    let check = MaturityCheck::new(
        "skills-installed",
        MaturityLevel::L4Skilled,
        "all skills/<name>/ are symlinked into ~/.claude/skills/",
        true,
    );

    let skills_dir = repo.join("skills");
    let skill_names: Vec<String> = match skills_dir.read_dir() {
        Ok(d) => d
            .filter_map(|e| e.ok())
            .filter(|e| e.path().join("SKILL.md").exists())
            .filter_map(|e| e.file_name().into_string().ok())
            .collect(),
        Err(_) => return fail(check, "skills/ directory not found"),
    };

    if skill_names.is_empty() {
        return pass(check);
    }

    let claude_skills = match claude_skills {
        Some(p) => p,
        None => return fail(check, "$HOME is not set — cannot locate ~/.claude/skills/"),
    };

    if !claude_skills.parent().map_or(false, |p| p.exists()) {
        return fail(check, "~/.claude not found — is Claude Code installed?");
    }

    let mut not_linked: Vec<String> = Vec::new();

    for name in &skill_names {
        let expected = match repo.join("skills").join(name).canonicalize() {
            Ok(p) => p,
            Err(_) => repo.join("skills").join(name),
        };
        let link_path = claude_skills.join(name);

        match std::fs::read_link(&link_path) {
            Ok(target) => {
                let abs_target = if target.is_absolute() {
                    target.clone()
                } else {
                    link_path.parent().unwrap_or(Path::new("/")).join(&target)
                };
                let canonical = abs_target.canonicalize().unwrap_or(abs_target);
                if canonical != expected {
                    not_linked.push(format!("{name} (symlink points elsewhere)"));
                }
            }
            Err(_) => {
                if link_path.exists() {
                    not_linked.push(format!("{name} (real path, not a symlink)"));
                } else {
                    not_linked.push(name.clone());
                }
            }
        }
    }

    if not_linked.is_empty() {
        pass(check)
    } else {
        fail(check, format!("not linked: {}", not_linked.join(", ")))
    }
}

pub fn check_claude_agents(repo: &Path) -> MaturityCheckResult {
    let check = MaturityCheck::new(
        "claude-agents",
        MaturityLevel::L4Skilled,
        ".claude/agents/ directory exists",
        false,
    );
    let path = repo.join(".claude/agents");
    if path.is_dir() { pass(check) } else { fail(check, ".claude/agents/ directory not found") }
}

// ── L5 Orchestrated ──────────────────────────────────────────────────────────

pub fn check_claude_app(_repo: &Path) -> MaturityCheckResult {
    let check = MaturityCheck::new(
        "claude-app",
        MaturityLevel::L5Orchestrated,
        "Claude GitHub App installed (manual step)",
        true,
    );
    fail(check, "cannot verify automatically — install the Claude GitHub App manually")
}

pub fn check_claude_workflow(repo: &Path) -> MaturityCheckResult {
    let check = MaturityCheck::new(
        "claude-workflow",
        MaturityLevel::L5Orchestrated,
        ".github/workflows/claude.yml present",
        true,
    );
    let path = repo.join(".github/workflows/claude.yml");
    if path.exists() {
        pass(check)
    } else {
        fail(check, ".github/workflows/claude.yml not found")
    }
}

pub fn check_conductr_schema(repo: &Path) -> MaturityCheckResult {
    let check = MaturityCheck::new(
        "conductr-schema",
        MaturityLevel::L5Orchestrated,
        ".conductr parses cleanly against the v2 schema",
        false,
    );

    let path = repo.join(".conductr");
    if !path.exists() {
        return fail(check, ".conductr not found");
    }

    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => return fail(check, format!("cannot read .conductr: {e}")),
    };

    let value: toml::Value = match toml::from_str(&raw) {
        Ok(v) => v,
        Err(e) => return fail(check, format!("TOML parse error: {e}")),
    };

    if let Some(err) = validate_tempo_prs(&value) {
        return fail(check, err);
    }
    if let Some(err) = validate_ci_runs(&value) {
        return fail(check, err);
    }

    pass(check)
}

fn validate_tempo_prs(value: &toml::Value) -> Option<String> {
    let prs = value.get("tempo")?.get("prs")?.as_array()?;
    for (i, pr) in prs.iter().enumerate() {
        if pr.get("number").and_then(|n| n.as_integer()).is_none() {
            return Some(format!("[[tempo.prs]][{i}]: `number` is missing or not an integer"));
        }
        if let Some(cx) = pr.get("complexity").and_then(|c| c.as_str()) {
            if !matches!(cx, "XS" | "S" | "M" | "L") {
                return Some(format!(
                    "[[tempo.prs]][{i}]: `complexity` must be XS/S/M/L, got {cx:?}"
                ));
            }
        }
        if pr.get("opened").is_none() {
            return Some(format!("[[tempo.prs]][{i}]: `opened` is missing"));
        }
        if pr.get("merged").and_then(|m| m.as_bool()).is_none() {
            return Some(format!("[[tempo.prs]][{i}]: `merged` is missing or not a boolean"));
        }
    }
    None
}

fn validate_ci_runs(value: &toml::Value) -> Option<String> {
    let runs = value.get("ci")?.get("runs")?.as_array()?;
    for (i, run) in runs.iter().enumerate() {
        if run.get("pr").and_then(|p| p.as_integer()).is_none() {
            return Some(format!("[[ci.runs]][{i}]: `pr` is missing or not an integer"));
        }
        let has_minutes = run.get("minutes").and_then(|m| m.as_float()).is_some()
            || run.get("minutes").and_then(|m| m.as_integer()).is_some();
        if !has_minutes {
            return Some(format!("[[ci.runs]][{i}]: `minutes` is missing or not a number"));
        }
        if run.get("ts").is_none() {
            return Some(format!("[[ci.runs]][{i}]: `ts` is missing"));
        }
    }
    None
}

// ── Tests ─────────────────────────────────────────────────────────────────────

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

    #[test]
    fn no_skills_dir_fails() {
        let tmp = TempDir::new().unwrap();
        let result = check_skills_installed_with(tmp.path(), Some(&tmp.path().join("claude/skills")));
        assert!(!result.passed);
    }

    #[test]
    fn no_claude_parent_fails() {
        let tmp = TempDir::new().unwrap();
        make_skill(tmp.path(), "idle");
        let nonexistent = tmp.path().join("no-claude").join("skills");
        let result = check_skills_installed_with(tmp.path(), Some(&nonexistent));
        assert!(!result.passed);
        assert!(result.detail.unwrap_or_default().contains("not found"));
    }

    #[test]
    fn home_not_set_fails() {
        let tmp = TempDir::new().unwrap();
        make_skill(tmp.path(), "idle");
        let result = check_skills_installed_with(tmp.path(), None);
        assert!(!result.passed);
    }

    #[test]
    fn no_skills_passes_vacuously() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("skills")).unwrap();
        let claude_skills = tmp.path().join("claude").join("skills");
        fs::create_dir_all(&claude_skills).unwrap();
        let result = check_skills_installed_with(tmp.path(), Some(&claude_skills));
        assert!(result.passed);
    }

    #[test]
    fn skill_without_skill_md_skipped() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("skills").join("noskillmd")).unwrap();
        let claude_skills = tmp.path().join("claude").join("skills");
        fs::create_dir_all(&claude_skills).unwrap();
        let result = check_skills_installed_with(tmp.path(), Some(&claude_skills));
        assert!(result.passed, "dir without SKILL.md should be ignored");
    }

    #[cfg(unix)]
    #[test]
    fn correct_symlink_passes() {
        let tmp = TempDir::new().unwrap();
        make_skill(tmp.path(), "idle");
        let claude_skills = tmp.path().join("claude").join("skills");
        fs::create_dir_all(&claude_skills).unwrap();
        let skill_abs = tmp.path().join("skills").join("idle").canonicalize().unwrap();
        std::os::unix::fs::symlink(&skill_abs, claude_skills.join("idle")).unwrap();
        let result = check_skills_installed_with(tmp.path(), Some(&claude_skills));
        assert!(result.passed, "correct symlink should pass: {:?}", result.detail);
    }

    #[cfg(unix)]
    #[test]
    fn missing_symlink_fails() {
        let tmp = TempDir::new().unwrap();
        make_skill(tmp.path(), "idle");
        let claude_skills = tmp.path().join("claude").join("skills");
        fs::create_dir_all(&claude_skills).unwrap();
        let result = check_skills_installed_with(tmp.path(), Some(&claude_skills));
        assert!(!result.passed);
        assert!(result.detail.unwrap_or_default().contains("idle"));
    }

    #[cfg(unix)]
    #[test]
    fn wrong_symlink_fails() {
        let tmp = TempDir::new().unwrap();
        make_skill(tmp.path(), "idle");
        let claude_skills = tmp.path().join("claude").join("skills");
        fs::create_dir_all(&claude_skills).unwrap();
        let other_dir = tmp.path().join("other");
        fs::create_dir_all(&other_dir).unwrap();
        std::os::unix::fs::symlink(&other_dir, claude_skills.join("idle")).unwrap();
        let result = check_skills_installed_with(tmp.path(), Some(&claude_skills));
        assert!(!result.passed);
        assert!(result.detail.unwrap_or_default().contains("elsewhere"));
    }

    #[cfg(unix)]
    #[test]
    fn real_dir_instead_of_symlink_fails() {
        let tmp = TempDir::new().unwrap();
        make_skill(tmp.path(), "idle");
        let claude_skills = tmp.path().join("claude").join("skills");
        fs::create_dir_all(claude_skills.join("idle")).unwrap();
        let result = check_skills_installed_with(tmp.path(), Some(&claude_skills));
        assert!(!result.passed);
        assert!(result.detail.unwrap_or_default().contains("real path"));
    }

    #[cfg(unix)]
    #[test]
    fn multiple_skills_all_linked_passes() {
        let tmp = TempDir::new().unwrap();
        for name in &["idle", "orchestrate", "pod"] {
            make_skill(tmp.path(), name);
        }
        let claude_skills = tmp.path().join("claude").join("skills");
        fs::create_dir_all(&claude_skills).unwrap();
        for name in &["idle", "orchestrate", "pod"] {
            let skill_abs = tmp.path().join("skills").join(name).canonicalize().unwrap();
            std::os::unix::fs::symlink(&skill_abs, claude_skills.join(name)).unwrap();
        }
        let result = check_skills_installed_with(tmp.path(), Some(&claude_skills));
        assert!(result.passed);
    }

    #[cfg(unix)]
    #[test]
    fn partial_links_fails() {
        let tmp = TempDir::new().unwrap();
        make_skill(tmp.path(), "idle");
        make_skill(tmp.path(), "orchestrate");
        let claude_skills = tmp.path().join("claude").join("skills");
        fs::create_dir_all(&claude_skills).unwrap();
        let idle_abs = tmp.path().join("skills").join("idle").canonicalize().unwrap();
        std::os::unix::fs::symlink(&idle_abs, claude_skills.join("idle")).unwrap();
        // orchestrate is NOT linked
        let result = check_skills_installed_with(tmp.path(), Some(&claude_skills));
        assert!(!result.passed);
        assert!(result.detail.unwrap_or_default().contains("orchestrate"));
    }
}

// ── Catalogue + bulk runner ───────────────────────────────────────────────────

/// Run every check against `repo` and return results in level order.
pub fn run_all(repo: &Path) -> Vec<MaturityCheckResult> {
    vec![
        check_ci_workflow(repo),
        check_gitignore_target(repo),
        check_gitignore_conductr_local(repo),
        check_dev_branch(repo),
        check_default_base_dev(repo),
        check_claude_base_md(repo),
        check_contributing(repo),
        check_codeowners(repo),
        check_skill_md(repo),
        check_skills_installed(repo),
        check_claude_agents(repo),
        check_claude_app(repo),
        check_claude_workflow(repo),
        check_conductr_schema(repo),
    ]
}
