//! Issue dependency graph: validation, cycle detection, batched topological sort.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::types::IssueNumber;

#[derive(Debug, Clone, Default)]
pub struct DepGraph {
    /// Maps issue -> set of issues it depends on.
    pub edges: BTreeMap<IssueNumber, BTreeSet<IssueNumber>>,
}

#[derive(Debug, thiserror::Error)]
pub enum GraphError {
    #[error("cycle detected involving issues: {0:?}")]
    Cycle(Vec<IssueNumber>),
    #[error("issue #{from} depends on #{to} which is not in the batch")]
    DependencyOutsideBatch { from: IssueNumber, to: IssueNumber },
}

impl DepGraph {
    pub fn new() -> Self { Self::default() }

    pub fn add_issue(&mut self, issue: IssueNumber) {
        self.edges.entry(issue).or_default();
    }

    pub fn add_dep(&mut self, from: IssueNumber, to: IssueNumber) {
        self.edges.entry(from).or_default().insert(to);
        self.edges.entry(to).or_default();
    }

    pub fn issues(&self) -> impl Iterator<Item = IssueNumber> + '_ {
        self.edges.keys().copied()
    }

    pub fn deps_of(&self, issue: IssueNumber) -> Option<&BTreeSet<IssueNumber>> {
        self.edges.get(&issue)
    }

    /// Validate that every dependency is also a node in the graph.
    pub fn check_closed(&self) -> Result<(), GraphError> {
        for (from, deps) in &self.edges {
            for to in deps {
                if !self.edges.contains_key(to) {
                    return Err(GraphError::DependencyOutsideBatch { from: *from, to: *to });
                }
            }
        }
        Ok(())
    }
}

/// Kahn-style topological sort returning *batches* of nodes that can be run in parallel.
///
/// `closed` is the set of issues already complete (not in the batch); their deps are
/// considered satisfied. Use this to mirror the SKILL.md rule that dependencies
/// outside the batch which are already merged are fine.
pub fn topo_batches(
    graph: &DepGraph,
    closed: &BTreeSet<IssueNumber>,
) -> Result<Vec<Vec<IssueNumber>>, GraphError> {
    let mut indegree: BTreeMap<IssueNumber, usize> = BTreeMap::new();
    for n in graph.issues() {
        indegree.insert(n, 0);
    }
    for (from, deps) in &graph.edges {
        let mut count = 0;
        for d in deps {
            if !closed.contains(d) && graph.edges.contains_key(d) {
                count += 1;
            }
        }
        indegree.insert(*from, count);
    }

    let mut batches: Vec<Vec<IssueNumber>> = Vec::new();
    let mut ready: VecDeque<IssueNumber> = indegree
        .iter()
        .filter(|(_, &d)| d == 0)
        .map(|(n, _)| *n)
        .collect();

    let mut visited = 0usize;
    let total = indegree.len();

    while !ready.is_empty() {
        let mut batch: Vec<IssueNumber> = ready.iter().copied().collect();
        batch.sort();
        ready.clear();
        visited += batch.len();

        let next: Vec<_> = graph
            .edges
            .iter()
            .filter_map(|(&node, deps)| {
                if batch.contains(&node) {
                    return None;
                }
                let prior_indegree = *indegree.get(&node).unwrap_or(&0);
                if prior_indegree == 0 {
                    return None; // already emitted in an earlier batch
                }
                let still_blocked = deps
                    .iter()
                    .filter(|d| {
                        !closed.contains(d)
                            && graph.edges.contains_key(d)
                            && !batch.contains(d)
                            // also exclude any issue emitted in earlier batches
                            && !already_emitted(&batches, **d)
                    })
                    .count();
                if still_blocked == 0 {
                    Some(node)
                } else {
                    None
                }
            })
            .collect();

        for n in next {
            indegree.insert(n, 0);
            ready.push_back(n);
        }
        batches.push(batch);
    }

    if visited != total {
        let remaining: Vec<_> = indegree
            .into_iter()
            .filter(|(_, d)| *d > 0)
            .map(|(n, _)| n)
            .collect();
        return Err(GraphError::Cycle(remaining));
    }

    Ok(batches)
}

fn already_emitted(batches: &[Vec<IssueNumber>], n: IssueNumber) -> bool {
    batches.iter().any(|b| b.contains(&n))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph_from(edges: &[(IssueNumber, &[IssueNumber])]) -> DepGraph {
        let mut g = DepGraph::new();
        for (from, deps) in edges {
            g.add_issue(*from);
            for d in *deps {
                g.add_dep(*from, *d);
            }
        }
        g
    }

    #[test]
    fn parallel_roots_then_dependent() {
        let g = graph_from(&[(1, &[]), (2, &[]), (3, &[1, 2])]);
        let batches = topo_batches(&g, &BTreeSet::new()).unwrap();
        assert_eq!(batches, vec![vec![1, 2], vec![3]]);
    }

    #[test]
    fn cycle_detected() {
        let g = graph_from(&[(1, &[2]), (2, &[1])]);
        let res = topo_batches(&g, &BTreeSet::new());
        assert!(matches!(res, Err(GraphError::Cycle(_))));
    }

    #[test]
    fn closed_dep_is_satisfied() {
        // Graph contains only the open issue #2; #1 is already closed and not a node.
        let mut g = DepGraph::new();
        g.add_issue(2);
        g.edges.entry(2).or_default().insert(1);
        let mut closed = BTreeSet::new();
        closed.insert(1);
        let batches = topo_batches(&g, &closed).unwrap();
        assert_eq!(batches, vec![vec![2]]);
    }

    #[test]
    fn dependency_outside_batch_is_flagged_by_check_closed() {
        let mut g = DepGraph::new();
        g.add_dep(2, 1);
        // node 1 was not added explicitly with add_issue, but add_dep adds both nodes.
        // remove 1 to simulate "outside batch":
        g.edges.remove(&1);
        assert!(g.check_closed().is_err());
    }
}
