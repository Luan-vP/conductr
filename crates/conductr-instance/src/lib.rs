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

pub use conductr_core::ports::InstanceProvider;
pub use conductr_core::types::*;

/// Stub implementation that errors on every call. Replace with real
/// providers once the agentic port lands.
#[derive(Debug, Clone, Default)]
pub struct StubManager;

#[async_trait]
impl InstanceProvider for StubManager {
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
