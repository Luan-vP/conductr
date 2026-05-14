//! GitHub-issue dependency orchestration.
//!
//! Mirrors the [`orchestrate` skill](../../skills/orchestrate/SKILL.md).
//!
//! Surveys open GitHub issues, builds a dependency graph, classifies each
//! issue into a bucket, then drives the implementation forward by triggering
//! the `@claude` GitHub Action bot in dependency order and merging PRs as
//! their CI passes.
//!
//! The actual GitHub I/O is abstracted behind [`conductr_core::ports::ScmHost`], so
//! the orchestration loop is unit-testable without hitting the network.

pub mod architect;
pub mod classifier;
pub mod deps;
pub mod graph;
pub mod orchestrator;
pub mod types;

pub use conductr_core::ports::ScmHost;
pub use conductr_core::types::*;
pub use architect::{cluster_issues, complexity_for, infer_phrases, phrase_id_from_title, Phrase};
pub use classifier::classify;
pub use deps::{parse_dependencies, DepRef};
pub use graph::topo_batches;
pub use orchestrator::Orchestrator;
