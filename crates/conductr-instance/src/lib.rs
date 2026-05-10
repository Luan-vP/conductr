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
//! For now the crate exposes the trait surface so the CLI can compile.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstanceSpec {
    pub name: String,
    pub provider: Provider,
    pub size: String,
    pub region: Option<String>,
    pub image: Option<String>,
    pub ssh_key: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Aws,
    Hetzner,
    DigitalOcean,
    Local,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstanceHandle {
    pub id: String,
    pub provider: Provider,
    pub host: String,
    pub user: String,
}

#[derive(Debug, thiserror::Error)]
pub enum InstanceError {
    #[error("provider {0:?} not implemented yet (port from agentic)")]
    NotImplemented(Provider),
    #[error("ssh: {0}")]
    Ssh(String),
    #[error("provider error: {0}")]
    Provider(String),
}

#[async_trait]
pub trait InstanceManager: Send + Sync {
    async fn spin_up(&self, spec: &InstanceSpec) -> Result<InstanceHandle, InstanceError>;
    async fn connect(&self, handle: &InstanceHandle) -> Result<(), InstanceError>;
    async fn run(&self, handle: &InstanceHandle, cmd: &str) -> Result<String, InstanceError>;
    async fn tear_down(&self, handle: &InstanceHandle) -> Result<(), InstanceError>;
}

/// Stub implementation that errors on every call. Replace with real
/// providers once the agentic port lands.
#[derive(Debug, Clone, Default)]
pub struct StubManager;

#[async_trait]
impl InstanceManager for StubManager {
    async fn spin_up(&self, spec: &InstanceSpec) -> Result<InstanceHandle, InstanceError> {
        Err(InstanceError::NotImplemented(spec.provider))
    }
    async fn connect(&self, handle: &InstanceHandle) -> Result<(), InstanceError> {
        Err(InstanceError::NotImplemented(handle.provider))
    }
    async fn run(&self, handle: &InstanceHandle, _cmd: &str) -> Result<String, InstanceError> {
        Err(InstanceError::NotImplemented(handle.provider))
    }
    async fn tear_down(&self, handle: &InstanceHandle) -> Result<(), InstanceError> {
        Err(InstanceError::NotImplemented(handle.provider))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stub_returns_not_implemented() {
        let m = StubManager;
        let spec = InstanceSpec {
            name: "foo".into(),
            provider: Provider::Aws,
            size: "t3.small".into(),
            region: None,
            image: None,
            ssh_key: None,
        };
        let r = m.spin_up(&spec).await;
        assert!(matches!(r, Err(InstanceError::NotImplemented(Provider::Aws))));
    }
}
