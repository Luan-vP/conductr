//! Parse dependency declarations out of issue bodies.
//!
//! Recognised forms (case-insensitive):
//!
//! - `depends on #N` (also `depends on #N, #M`)
//! - `blocked by #N`
//! - `after #N`
//! - `requires #N`
//! - Checklist: `- [ ] #N must be done first`

use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::BTreeSet;

use crate::types::IssueNumber;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct DepRef(pub IssueNumber);

static PHRASE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(?:depends\s+on|blocked\s+by|after|requires)\b([^\n.]*)").unwrap()
});

static CHECKLIST_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?im)^\s*-\s*\[\s*[ x]\s*\]\s*#(\d+)\s+must\s+be\s+done\s+first").unwrap()
});

static ISSUE_NUM_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"#(\d+)").unwrap());

/// Returns the set of issue numbers that the given body declares as dependencies.
pub fn parse_dependencies(body: &str) -> BTreeSet<DepRef> {
    let mut out = BTreeSet::new();

    for cap in PHRASE_RE.captures_iter(body) {
        let tail = cap.get(1).map(|m| m.as_str()).unwrap_or("");
        for n in ISSUE_NUM_RE.captures_iter(tail) {
            if let Ok(num) = n.get(1).unwrap().as_str().parse::<IssueNumber>() {
                out.insert(DepRef(num));
            }
        }
    }

    for cap in CHECKLIST_RE.captures_iter(body) {
        if let Ok(num) = cap.get(1).unwrap().as_str().parse::<IssueNumber>() {
            out.insert(DepRef(num));
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deps(s: &str) -> Vec<IssueNumber> {
        parse_dependencies(s).into_iter().map(|d| d.0).collect()
    }

    #[test]
    fn depends_on_single() {
        assert_eq!(deps("This task depends on #42 to land first."), vec![42]);
    }

    #[test]
    fn depends_on_multiple() {
        assert_eq!(deps("Depends on #1, #2, and #3."), vec![1, 2, 3]);
    }

    #[test]
    fn blocked_by_after_requires() {
        assert_eq!(deps("Blocked by #5. After #6. requires #7."), vec![5, 6, 7]);
    }

    #[test]
    fn checklist_must_be_done_first() {
        let body = "## Plan\n\n- [ ] #11 must be done first\n- [x] #12 must be done first\n";
        assert_eq!(deps(body), vec![11, 12]);
    }

    #[test]
    fn ignores_unrelated_issue_mentions() {
        // Plain `see #99` should not be treated as a dependency.
        assert_eq!(deps("See #99 for context."), Vec::<IssueNumber>::new());
    }

    #[test]
    fn case_insensitive() {
        assert_eq!(deps("DEPENDS ON #4"), vec![4]);
    }
}
