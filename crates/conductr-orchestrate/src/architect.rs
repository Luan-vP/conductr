//! Architecture Reference Notes (ARNs).
//!
//! Mirrors the `architect.md` skill from poorchestrator. We model the data
//! shape and rendering; the LLM-driven analysis itself is intentionally out
//! of scope for this crate (callers can plug in their own analysis pipeline
//! and feed the results into [`Arn::render`]).

use std::collections::BTreeMap;

use crate::types::IssueNumber;

#[derive(Debug, Clone)]
pub struct Arn {
    pub local_map: String,
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

#[cfg(test)]
mod tests {
    use super::*;
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
}
