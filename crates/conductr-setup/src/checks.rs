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

// ── Catalogue + bulk runner ───────────────────────────────────────────────────

/// Run every check against `repo` and return results in level order.
pub fn run_all(repo: &Path) -> Vec<MaturityCheckResult> {
    vec![
        check_ci_workflow(repo),
        check_gitignore_target(repo),
        check_dev_branch(repo),
        check_default_base_dev(repo),
        check_claude_base_md(repo),
        check_contributing(repo),
        check_codeowners(repo),
        check_skill_md(repo),
        check_claude_agents(repo),
        check_claude_app(repo),
        check_claude_workflow(repo),
    ]
}
