//! Cloud instance spin-up and connection.
//!
//! This crate is a placeholder. Provider implementations (provisioner, SSH
//! client, agent install) are future work.
//!
//! For now the crate re-exports the port surface from `conductr-core` so that
//! callers can migrate away from the old `InstanceManager` / `StubManager` names.

pub use conductr_core::ports::InstanceProvider;
pub use conductr_core::types::*;
