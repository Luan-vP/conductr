//! Port of [poorchestrator](https://github.com/Luan-vP/poorchestrator) to Rust.
//!
//! Surveys open GitHub issues, builds a dependency graph, classifies each
//! issue into a bucket, then drives the implementation forward by triggering
//! the `@claude` GitHub Action bot in dependency order and merging PRs as
//! their CI passes.
//!
//! The actual GitHub I/O is abstracted behind [`github::GitHubClient`], so
//! the orchestration loop is unit-testable without hitting the network.

pub mod architect;
pub mod classifier;
pub mod deps;
pub mod github;
pub mod graph;
pub mod orchestrator;
pub mod types;

pub use classifier::{classify, Bucket, Classification};
pub use deps::{parse_dependencies, DepRef};
pub use github::{GhCli, GitHubClient};
pub use graph::{topo_batches, DepGraph, GraphError};
pub use orchestrator::{Orchestrator, OrchestratorConfig};
pub use types::{Issue, IssueNumber, Pr, PrNumber, RepoSlug};
