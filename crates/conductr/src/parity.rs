//! CLI–skill parity predicate.
//!
//! `check_cli_skill_parity` compares the top-level clap subcommands against
//! `skills/*/SKILL.md` files and returns a `Finding` for each drift.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use clap::CommandFactory;

use crate::idle::{Finding, FindingSeverity};

// ── Public types ───────────────────────────────────────────────────────────────

/// Handle to the repository root used by analysis predicates.
pub struct Workspace {
    pub root: PathBuf,
}

impl Workspace {
    /// Open a workspace, walking up from `path` to find the directory
    /// containing `skills/`.
    pub fn open(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let start = std::fs::canonicalize(path.as_ref()).unwrap_or_else(|_| path.as_ref().to_path_buf());
        let root = find_workspace_root(&start)?;
        Ok(Self { root })
    }
}

// ── Predicate ──────────────────────────────────────────────────────────────────

/// Compare the clap CLI tree against `skills/*/SKILL.md` and return a
/// `Finding` for each drift between them.
pub fn check_cli_skill_parity(ws: &Workspace) -> Vec<Finding> {
    let mut findings = Vec::new();

    // Collect top-level CLI subcommands via clap introspection.
    let root_cmd = crate::Cli::command();
    let cli_subcmds: Vec<CliSubcmd> = root_cmd
        .get_subcommands()
        .map(CliSubcmd::from_clap)
        .collect();

    // Collect single-word skills (multi-word names are sub-subcommand skills, skipped).
    let skills_dir = ws.root.join("skills");
    let skills = collect_skills(&skills_dir);

    // Drift 1: skill without matching CLI subcommand.
    for skill in &skills {
        if !cli_subcmds.iter().any(|c| c.name == skill.name) {
            let fp = format!("parity/skill-without-cli/{}", skill.name);
            findings.push(Finding {
                title: format!("parity: skill `{}` has no matching CLI subcommand", skill.name),
                body: format!(
                    "## Finding\n\n\
                     CLI–skill parity violated: `skills/{name}/SKILL.md` declares \
                     `name: {name}` but `conductr {name}` is not a registered subcommand.\n\n\
                     ## Acceptance criteria\n\n\
                     - [ ] Either add `conductr {name}` as a top-level CLI subcommand, or \
                     update the skill `name:` frontmatter to reflect the correct command.\n\
                     - [ ] `cargo test cli_skill_parity_is_clean` passes.\n\n\
                     <!-- conductr-idle-fingerprint: {fp} -->",
                    name = skill.name,
                    fp = fp,
                ),
                severity: FindingSeverity::Architecture,
                fingerprint: fp,
            });
        }
    }

    // Drift 2: CLI subcommand without matching skill.
    for cmd in &cli_subcmds {
        if !skills.iter().any(|s| s.name == cmd.name) {
            let fp = format!("parity/cli-without-skill/{}", cmd.name);
            findings.push(Finding {
                title: format!("parity: `conductr {}` has no matching skill", cmd.name),
                body: format!(
                    "## Finding\n\n\
                     CLI–skill parity violated: `conductr {name}` is a registered \
                     subcommand but `skills/{name}/SKILL.md` does not exist.\n\n\
                     ## Acceptance criteria\n\n\
                     - [ ] Create `skills/{name}/SKILL.md` with `name: {name}` in frontmatter \
                     and a `cli:` field matching the clap invocation.\n\
                     - [ ] `cargo test cli_skill_parity_is_clean` passes.\n\n\
                     <!-- conductr-idle-fingerprint: {fp} -->",
                    name = cmd.name,
                    fp = fp,
                ),
                severity: FindingSeverity::Architecture,
                fingerprint: fp,
            });
        }
    }

    // Drifts 3–5: flag-level parity for matching pairs.
    for skill in &skills {
        let Some(cmd) = cli_subcmds.iter().find(|c| c.name == skill.name) else {
            continue; // already reported above
        };

        for sf in &skill.flags {
            match cmd.flags.iter().find(|f| f.name == sf.name) {
                None => {
                    // Skill invocation references a flag clap doesn't expose.
                    let fp = format!("parity/flag-extra/{}/{}", skill.name, sf.name);
                    findings.push(Finding {
                        title: format!(
                            "parity: skill `{}` references unknown flag `--{}`",
                            skill.name, sf.name
                        ),
                        body: format!(
                            "## Finding\n\n\
                             CLI–skill parity violated: `skills/{skill}/SKILL.md` \
                             references `--{flag}` in its invocation lines but \
                             `conductr {skill}` does not expose that flag in clap.\n\n\
                             ## Acceptance criteria\n\n\
                             - [ ] Either add `--{flag}` to `conductr {skill}` in \
                             `crates/conductr/src/main.rs`, or remove it from the skill.\n\
                             - [ ] `cargo test cli_skill_parity_is_clean` passes.\n\n\
                             <!-- conductr-idle-fingerprint: {fp} -->",
                            skill = skill.name,
                            flag = sf.name,
                            fp = fp,
                        ),
                        severity: FindingSeverity::Architecture,
                        fingerprint: fp,
                    });
                }
                Some(cf) => {
                    // Flag exists in both — check type.
                    if sf.kind != FlagKind::Unknown && cf.kind != sf.kind {
                        let fp = format!("parity/flag-type/{}/{}", skill.name, sf.name);
                        findings.push(Finding {
                            title: format!(
                                "parity: flag type mismatch `{}` `--{}`",
                                skill.name, sf.name
                            ),
                            body: format!(
                                "## Finding\n\n\
                                 CLI–skill parity violated: `--{flag}` for \
                                 `conductr {skill}` is `{cli_kind:?}` in clap but the \
                                 skill invocation implies `{skill_kind:?}`.\n\n\
                                 ## Acceptance criteria\n\n\
                                 - [ ] Align the flag kind between clap and the skill.\n\
                                 - [ ] `cargo test cli_skill_parity_is_clean` passes.\n\n\
                                 <!-- conductr-idle-fingerprint: {fp} -->",
                                flag = sf.name,
                                skill = skill.name,
                                cli_kind = cf.kind,
                                skill_kind = sf.kind,
                                fp = fp,
                            ),
                            severity: FindingSeverity::Architecture,
                            fingerprint: fp,
                        });
                    }

                    // Check description drift (normalised whitespace comparison).
                    if !cf.help.is_empty()
                        && !sf.help.is_empty()
                        && normalise_desc(&cf.help) != normalise_desc(&sf.help)
                    {
                        let fp = format!("parity/flag-desc/{}/{}", skill.name, sf.name);
                        findings.push(Finding {
                            title: format!(
                                "parity: flag description drift `{}` `--{}`",
                                skill.name, sf.name
                            ),
                            body: format!(
                                "## Finding\n\n\
                                 CLI–skill parity violated: the description of `--{flag}` \
                                 for `conductr {skill}` differs between clap and the skill.\n\n\
                                 clap: `{cli_help}`\n\
                                 skill: `{skill_help}`\n\n\
                                 ## Acceptance criteria\n\n\
                                 - [ ] Reconcile the flag description in clap and the skill.\n\
                                 - [ ] `cargo test cli_skill_parity_is_clean` passes.\n\n\
                                 <!-- conductr-idle-fingerprint: {fp} -->",
                                flag = sf.name,
                                skill = skill.name,
                                cli_help = cf.help,
                                skill_help = sf.help,
                                fp = fp,
                            ),
                            severity: FindingSeverity::Architecture,
                            fingerprint: fp,
                        });
                    }
                }
            }
        }
    }

    findings
}

// ── Internal types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
enum FlagKind {
    Bool,
    Value,
    Unknown,
}

#[derive(Debug, Clone)]
struct CliFlag {
    name: String,
    help: String,
    kind: FlagKind,
}

#[derive(Debug)]
struct CliSubcmd {
    name: String,
    flags: Vec<CliFlag>,
}

impl CliSubcmd {
    fn from_clap(cmd: &clap::Command) -> Self {
        let flags = cmd
            .get_arguments()
            .filter(|a| is_user_flag(a))
            .map(|a| CliFlag {
                name: a.get_long().unwrap().to_string(),
                help: a.get_help().map(|h| h.to_string()).unwrap_or_default(),
                kind: cli_flag_kind(a),
            })
            .collect();
        Self {
            name: cmd.get_name().to_string(),
            flags,
        }
    }
}

#[derive(Debug)]
struct SkillFlag {
    name: String,
    help: String,
    kind: FlagKind,
}

#[derive(Debug)]
struct SkillInfo {
    /// Frontmatter `name:` value (single-word = top-level subcommand).
    name: String,
    /// Flags extracted from invocation lines in the skill body.
    flags: Vec<SkillFlag>,
}

// ── Skill scanner ──────────────────────────────────────────────────────────────

fn collect_skills(skills_dir: &Path) -> Vec<SkillInfo> {
    let Ok(entries) = std::fs::read_dir(skills_dir) else {
        return vec![];
    };
    let mut skills = Vec::new();
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path().join("SKILL.md");
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Some(info) = parse_skill_md(&content) else {
            continue;
        };
        // Only top-level (single-word) skills participate in the parity check.
        if !info.name.contains(' ') && !info.name.is_empty() {
            skills.push(info);
        }
    }
    skills
}

fn parse_skill_md(content: &str) -> Option<SkillInfo> {
    // Content must start with `---` frontmatter delimiter.
    let rest = content.trim_start();
    let rest = rest.strip_prefix("---")?;
    let rest = rest.strip_prefix('\n').unwrap_or(rest);

    // Find the closing `---` on its own line.
    let fm_end = rest.find("\n---")?;
    let fm = &rest[..fm_end];
    let body = &rest[fm_end..];

    // Extract `name:` (required).
    let name = fm.lines().find_map(|line| {
        line.trim()
            .strip_prefix("name:")
            .map(|v| v.trim().trim_matches('"').to_string())
    })?;
    if name.is_empty() {
        return None;
    }

    // Extract `cli:` (optional; provides flag hints).
    let cli_field = fm.lines().find_map(|line| {
        line.trim()
            .strip_prefix("cli:")
            .map(|v| v.trim().to_string())
    });

    let flags = extract_skill_flags(cli_field.as_deref(), body);
    Some(SkillInfo { name, flags })
}

/// Extract `--flag [<value>]` patterns from skill invocation lines.
///
/// Sources: the `cli:` frontmatter field, plus any body line that contains
/// `conductr ` (CLI invocation examples in the skill prose).
fn extract_skill_flags(cli_field: Option<&str>, body: &str) -> Vec<SkillFlag> {
    let mut flags: Vec<SkillFlag> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    let mut sources: Vec<String> = Vec::new();
    if let Some(cli) = cli_field {
        sources.push(cli.to_string());
    }
    for line in body.lines() {
        let t = line.trim();
        if t.contains("conductr ") {
            sources.push(t.to_string());
        }
    }

    for src in &sources {
        let mut pos = 0;
        while pos < src.len() {
            let Some(rel) = src[pos..].find("--") else {
                break;
            };
            let name_start = pos + rel + 2;
            pos = name_start;

            let name_end = src[name_start..]
                .find(|c: char| !c.is_alphanumeric() && c != '-')
                .map(|e| name_start + e)
                .unwrap_or(src.len());
            let flag_name = &src[name_start..name_end];

            if flag_name.is_empty() || !seen.insert(flag_name.to_string()) {
                pos = name_end;
                continue;
            }

            // Infer type from what follows the flag name.
            let after = src[name_end..].trim_start_matches('=').trim_start_matches(' ');
            let kind = if after.starts_with('<')
                || (after.starts_with('[') && after.contains('<'))
            {
                FlagKind::Value
            } else {
                FlagKind::Bool
            };

            flags.push(SkillFlag {
                name: flag_name.to_string(),
                help: String::new(), // skills don't document flag help in structured form
                kind,
            });
            pos = name_end;
        }
    }

    flags
}

// ── Helpers ────────────────────────────────────────────────────────────────────

/// Walk up from `start` to find the directory containing `skills/`.
fn find_workspace_root(start: &Path) -> std::io::Result<PathBuf> {
    let mut current = start.to_path_buf();
    loop {
        if current.join("skills").is_dir() {
            return Ok(current);
        }
        if !current.pop() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "could not find workspace root (no `skills/` directory in any ancestor)",
            ));
        }
    }
}

fn is_user_flag(arg: &clap::Arg) -> bool {
    // Filter out clap's auto-generated help/version flags by long name,
    // since ArgAction variants for built-ins differ across clap minor releases.
    match arg.get_long() {
        Some("help") | Some("version") => false,
        Some(_) => true,
        None => false,
    }
}

fn cli_flag_kind(arg: &clap::Arg) -> FlagKind {
    use clap::ArgAction;
    match arg.get_action() {
        ArgAction::SetTrue | ArgAction::SetFalse | ArgAction::Count => FlagKind::Bool,
        ArgAction::Set | ArgAction::Append => FlagKind::Value,
        _ => FlagKind::Unknown,
    }
}

fn normalise_desc(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_skill_parity_is_clean() {
        let ws = Workspace::open(".").expect("workspace root not found");
        let findings = check_cli_skill_parity(&ws);
        assert!(findings.is_empty(), "CLI–skill parity drift detected:\n{findings:#?}");
    }

    #[test]
    fn workspace_open_walks_up() {
        // Running from the crates/conductr directory, we should still find skills/.
        let ws = Workspace::open(".").expect("workspace root not found");
        assert!(ws.root.join("skills").is_dir());
    }

    #[test]
    fn parse_skill_md_extracts_name_and_flags() {
        let content = "---\nname: myskill\ncli: conductr myskill --dry-run [--repo <path>]\n---\n# Body\n";
        let info = parse_skill_md(content).unwrap();
        assert_eq!(info.name, "myskill");
        let flag_names: Vec<&str> = info.flags.iter().map(|f| f.name.as_str()).collect();
        assert!(flag_names.contains(&"dry-run"), "expected --dry-run");
        assert!(flag_names.contains(&"repo"), "expected --repo");
        let dry_run = info.flags.iter().find(|f| f.name == "dry-run").unwrap();
        assert_eq!(dry_run.kind, FlagKind::Bool);
        let repo = info.flags.iter().find(|f| f.name == "repo").unwrap();
        assert_eq!(repo.kind, FlagKind::Value);
    }

    #[test]
    fn parse_skill_md_skips_multi_word_name() {
        let content = "---\nname: architect diagram\ndescription: sub-skill\n---\n# Body\n";
        let info = parse_skill_md(content).unwrap();
        // collect_skills skips multi-word names; verify the parser still returns them
        assert!(info.name.contains(' '));
    }

    #[test]
    fn parse_skill_md_no_frontmatter_returns_none() {
        let content = "# No frontmatter here\n\nJust body text.";
        assert!(parse_skill_md(content).is_none());
    }

    #[test]
    fn find_workspace_root_finds_skills_dir() {
        let root = find_workspace_root(
            &std::fs::canonicalize(".").unwrap_or_else(|_| PathBuf::from(".")),
        )
        .expect("workspace root");
        assert!(root.join("skills").is_dir());
    }
}
