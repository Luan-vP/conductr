//! Architecture Reference Notes (ARNs) and phrase inference.
//!
//! Mirrors the [`architect`](../../skills/architect/architect.md) agent. We
//! model the data shape and rendering; the LLM-driven analysis itself is
//! intentionally out of scope for this crate (callers can plug in their own
//! analysis pipeline and feed the results into [`Arn::render`]).

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::types::{Complexity, Issue, IssueNumber};

#[derive(Debug, Clone)]
pub struct Arn {
    pub local_map: String,
    /// Complexity estimate assigned by `architect plan`. Consumed by the
    /// complexity precedence chain in [`complexity_for`].
    pub complexity: Option<Complexity>,
    pub scope: ArnScope,
    pub patterns: Vec<String>,
    pub interfaces: ArnInterfaces,
    pub constraints: Vec<String>,
    pub testing: Vec<String>,
    pub open_questions: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ArnScope {
    pub modules_affected: Vec<String>,
    pub new_files: Vec<String>,
    pub modified_files: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ArnInterfaces {
    pub provides_to_others: Vec<String>,
    pub consumes_from_others: Vec<String>,
    pub shared_types: Vec<String>,
}

impl Arn {
    pub fn render(&self) -> String {
        let mut s = String::new();
        s.push_str("## Architecture Reference Note\n\n### Local Map\n");
        s.push_str(&self.local_map);
        if let Some(c) = self.complexity {
            s.push_str(&format!("\n\n**Complexity:** {c}"));
        }
        s.push_str("\n\n### Scope\n");
        bullets(&mut s, "Modules affected", &self.scope.modules_affected);
        bullets(&mut s, "New files", &self.scope.new_files);
        bullets(&mut s, "Modified files", &self.scope.modified_files);
        s.push_str("\n### Patterns to Follow\n");
        for p in &self.patterns { s.push_str(&format!("- {p}\n")); }
        s.push_str("\n### Interfaces & Contracts\n");
        bullets(&mut s, "Provides to others", &self.interfaces.provides_to_others);
        bullets(&mut s, "Consumes from others", &self.interfaces.consumes_from_others);
        bullets(&mut s, "Shared types", &self.interfaces.shared_types);
        s.push_str("\n### Constraints\n");
        for c in &self.constraints { s.push_str(&format!("- {c}\n")); }
        s.push_str("\n### Testing Strategy\n");
        for t in &self.testing { s.push_str(&format!("- {t}\n")); }
        s.push_str("\n### Open Questions\n");
        for q in &self.open_questions { s.push_str(&format!("- {q}\n")); }
        s
    }
}

fn bullets(s: &mut String, label: &str, items: &[String]) {
    if items.is_empty() { return; }
    s.push_str(&format!("- **{label}**:\n"));
    for i in items { s.push_str(&format!("  - {i}\n")); }
}

/// Render an ASCII local-map for the batch with `current` highlighted.
pub fn render_local_map(
    titles: &BTreeMap<IssueNumber, String>,
    edges: &BTreeMap<IssueNumber, Vec<IssueNumber>>,
    current: IssueNumber,
) -> String {
    let mut out = String::new();
    for (&n, title) in titles {
        let prefix = if n == current { "◄── YOU ARE HERE" } else { "" };
        let deps = edges.get(&n).cloned().unwrap_or_default();
        if deps.is_empty() {
            out.push_str(&format!("#{n}  {title}  {prefix}\n"));
        } else {
            let dep_str = deps.iter().map(|d| format!("#{d}")).collect::<Vec<_>>().join(", ");
            out.push_str(&format!("#{n}  {title}  (depends on {dep_str})  {prefix}\n"));
        }
    }
    out
}

// ── Complexity precedence chain ───────────────────────────────────────────────

/// Walk the complexity precedence chain for an issue.
///
/// 1. GitHub label `complexity/{xs,s,m,l}` (case-insensitive)
/// 2. ARN complexity field (set by `architect plan`)
/// 3. Default `M`
pub fn complexity_for(issue: &Issue, arn_complexity: Option<Complexity>) -> Complexity {
    for label in &issue.labels {
        let lower = label.to_lowercase();
        if let Some(level) = lower.strip_prefix("complexity/") {
            match level {
                "xs" => return Complexity::XS,
                "s"  => return Complexity::S,
                "m"  => return Complexity::M,
                "l"  => return Complexity::L,
                _    => {}
            }
        }
    }
    arn_complexity.unwrap_or(Complexity::M)
}

// ── Phrase inference ──────────────────────────────────────────────────────────

/// A phrase groups a dependency cluster of issues under a single identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Phrase {
    /// Slugified phrase identifier derived from the lead issue title.
    pub id: String,
    /// Sorted list of issue numbers in this cluster.
    pub issues: Vec<IssueNumber>,
    /// The highest-level issue (sink in the dependency graph) used to name the phrase.
    pub lead_issue: IssueNumber,
}

/// Slugify an issue title into a phrase ID.
///
/// Takes the segment before the first colon (if any), lowercases it, replaces
/// non-alphanumeric runs with a single hyphen, and trims leading/trailing hyphens.
///
/// Example: `"begin: cron-friendly entry point"` → `"begin"`
pub fn phrase_id_from_title(title: &str) -> String {
    let base = title.split(':').next().unwrap_or(title);
    let slug = base
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect::<String>();
    let parts: Vec<&str> = slug.split('-').filter(|s| !s.is_empty()).collect();
    let result = parts.join("-");
    if result.is_empty() { "phrase".to_string() } else { result }
}

/// Group issues into dependency clusters (connected components).
///
/// Dependencies are treated as undirected edges: if A depends on B both A and B
/// are in the same cluster regardless of direction.  Only issues present in
/// `issues` are considered; dependencies pointing outside the set are ignored.
pub fn cluster_issues(
    issues: &[IssueNumber],
    deps: &BTreeMap<IssueNumber, BTreeSet<IssueNumber>>,
) -> Vec<Vec<IssueNumber>> {
    let all: BTreeSet<IssueNumber> = issues.iter().copied().collect();
    let mut visited: BTreeSet<IssueNumber> = BTreeSet::new();
    let mut clusters: Vec<Vec<IssueNumber>> = Vec::new();

    for &start in issues {
        if visited.contains(&start) {
            continue;
        }
        let mut cluster: Vec<IssueNumber> = Vec::new();
        let mut queue: VecDeque<IssueNumber> = VecDeque::new();
        queue.push_back(start);

        while let Some(n) = queue.pop_front() {
            if visited.contains(&n) || !all.contains(&n) {
                continue;
            }
            visited.insert(n);
            cluster.push(n);

            // Forward edges: n depends on these
            if let Some(fwd) = deps.get(&n) {
                for &d in fwd {
                    if !visited.contains(&d) && all.contains(&d) {
                        queue.push_back(d);
                    }
                }
            }
            // Reverse edges: these depend on n
            for (&other, other_deps) in deps {
                if other_deps.contains(&n) && !visited.contains(&other) && all.contains(&other) {
                    queue.push_back(other);
                }
            }
        }

        cluster.sort();
        clusters.push(cluster);
    }

    clusters
}

/// Identify the highest-level issue in a cluster: the sink in the dependency
/// graph (no other cluster member depends on it).  Ties broken by largest
/// issue number.
fn highest_level_in_cluster(
    cluster: &[IssueNumber],
    deps: &BTreeMap<IssueNumber, BTreeSet<IssueNumber>>,
) -> IssueNumber {
    let sinks: Vec<IssueNumber> = cluster
        .iter()
        .copied()
        .filter(|&n| {
            !cluster.iter().any(|&other| {
                other != n && deps.get(&other).map_or(false, |d| d.contains(&n))
            })
        })
        .collect();

    sinks.into_iter().max().or_else(|| cluster.iter().copied().max()).unwrap_or(0)
}

/// Infer phrases from a set of issues.
///
/// Groups issues into dependency clusters, then names each phrase from the
/// highest-level issue's title (the cluster's terminal goal).
pub fn infer_phrases(
    issues: &[IssueNumber],
    deps: &BTreeMap<IssueNumber, BTreeSet<IssueNumber>>,
    titles: &BTreeMap<IssueNumber, String>,
) -> Vec<Phrase> {
    cluster_issues(issues, deps)
        .into_iter()
        .map(|cluster| {
            let lead = highest_level_in_cluster(&cluster, deps);
            let title = titles.get(&lead).map(String::as_str).unwrap_or("");
            let id = phrase_id_from_title(title);
            Phrase { id, issues: cluster, lead_issue: lead }
        })
        .collect()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use conductr_core::types::IssueState;

    fn issue(number: IssueNumber, labels: Vec<&str>) -> Issue {
        Issue {
            number,
            title: format!("Issue #{number}"),
            body: String::new(),
            labels: labels.into_iter().map(str::to_string).collect(),
            state: IssueState::Open,
        }
    }

    #[test]
    fn renders_local_map() {
        let mut titles = BTreeMap::new();
        titles.insert(1, "Scaffold".to_string());
        titles.insert(2, "API".to_string());
        let mut edges = BTreeMap::new();
        edges.insert(2, vec![1]);
        let s = render_local_map(&titles, &edges, 2);
        assert!(s.contains("YOU ARE HERE"));
        assert!(s.contains("depends on #1"));
    }

    #[test]
    fn arn_render_includes_complexity() {
        let arn = Arn {
            local_map: "map".into(),
            complexity: Some(Complexity::L),
            scope: ArnScope::default(),
            patterns: vec![],
            interfaces: ArnInterfaces::default(),
            constraints: vec![],
            testing: vec![],
            open_questions: vec![],
        };
        let rendered = arn.render();
        assert!(rendered.contains("**Complexity:** L"), "rendered: {rendered}");
    }

    #[test]
    fn arn_render_omits_complexity_when_none() {
        let arn = Arn {
            local_map: "map".into(),
            complexity: None,
            scope: ArnScope::default(),
            patterns: vec![],
            interfaces: ArnInterfaces::default(),
            constraints: vec![],
            testing: vec![],
            open_questions: vec![],
        };
        assert!(!arn.render().contains("Complexity"));
    }

    // ── complexity_for ────────────────────────────────────────────────────────

    #[test]
    fn label_wins_over_arn_and_default() {
        let i = issue(1, vec!["complexity/xs"]);
        assert_eq!(complexity_for(&i, Some(Complexity::L)), Complexity::XS);
    }

    #[test]
    fn arn_wins_over_default_when_no_label() {
        let i = issue(1, vec![]);
        assert_eq!(complexity_for(&i, Some(Complexity::S)), Complexity::S);
    }

    #[test]
    fn default_m_when_no_label_no_arn() {
        let i = issue(1, vec![]);
        assert_eq!(complexity_for(&i, None), Complexity::M);
    }

    #[test]
    fn label_case_insensitive() {
        let i = issue(1, vec!["Complexity/L"]);
        assert_eq!(complexity_for(&i, None), Complexity::L);
    }

    // ── phrase_id_from_title ──────────────────────────────────────────────────

    #[test]
    fn phrase_id_colon_prefix() {
        assert_eq!(phrase_id_from_title("begin: cron-friendly entry point"), "begin");
    }

    #[test]
    fn phrase_id_no_colon() {
        assert_eq!(phrase_id_from_title("Scaffold Frontend"), "scaffold-frontend");
    }

    #[test]
    fn phrase_id_special_chars() {
        assert_eq!(phrase_id_from_title("(#1) My Feature!"), "1-my-feature");
    }

    // ── cluster_issues ────────────────────────────────────────────────────────

    #[test]
    fn independent_issues_form_separate_clusters() {
        let deps: BTreeMap<IssueNumber, BTreeSet<IssueNumber>> = BTreeMap::new();
        let mut clusters = cluster_issues(&[1, 2, 3], &deps);
        clusters.sort();
        assert_eq!(clusters, vec![vec![1], vec![2], vec![3]]);
    }

    #[test]
    fn dependent_issues_form_one_cluster() {
        let mut deps: BTreeMap<IssueNumber, BTreeSet<IssueNumber>> = BTreeMap::new();
        deps.entry(2).or_default().insert(1);
        deps.entry(3).or_default().insert(2);
        let clusters = cluster_issues(&[1, 2, 3], &deps);
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0], vec![1, 2, 3]);
    }

    #[test]
    fn two_disconnected_chains() {
        let mut deps: BTreeMap<IssueNumber, BTreeSet<IssueNumber>> = BTreeMap::new();
        deps.entry(2).or_default().insert(1); // chain A: 1→2
        deps.entry(4).or_default().insert(3); // chain B: 3→4
        let mut clusters = cluster_issues(&[1, 2, 3, 4], &deps);
        clusters.sort();
        assert_eq!(clusters, vec![vec![1, 2], vec![3, 4]]);
    }

    // ── infer_phrases ─────────────────────────────────────────────────────────

    #[test]
    fn infer_phrases_names_from_lead_issue() {
        let mut deps: BTreeMap<IssueNumber, BTreeSet<IssueNumber>> = BTreeMap::new();
        deps.entry(2).or_default().insert(1); // 1 is prerequisite; 2 is goal
        let mut titles: BTreeMap<IssueNumber, String> = BTreeMap::new();
        titles.insert(1, "scaffold: base setup".into());
        titles.insert(2, "feat: add api".into());

        let phrases = infer_phrases(&[1, 2], &deps, &titles);
        assert_eq!(phrases.len(), 1);
        assert_eq!(phrases[0].lead_issue, 2); // 2 is the sink
        assert_eq!(phrases[0].id, "feat"); // phrase from "feat: add api"
        assert_eq!(phrases[0].issues, vec![1, 2]);
    }
}
