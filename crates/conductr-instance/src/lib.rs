//! Cloud instance spin-up and connection.
//!
//! This crate is a placeholder. The intent is to port the patterns from the
//! private `Luan-vP/agentic` repo (provisioner + SSH client + agent install)
//! into Rust once we can read it. Add it as a submodule with auth:
//!
//! ```text
//! git submodule add git@github.com:Luan-vP/agentic.git vendor/agentic
//! ```
//!
//! For now the crate re-exports the port surface from `conductr-core` so that
//! callers can migrate away from the old `InstanceManager` / `StubManager` names.

pub use conductr_core::ports::InstanceProvider;
pub use conductr_core::types::*;
