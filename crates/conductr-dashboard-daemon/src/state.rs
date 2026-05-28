use std::sync::Arc;

use conductr_dashboard_core::model::DashboardState;
use tokio::sync::RwLock;

/// Shared daemon state — updated by aggregators, read by HTTP handlers.
pub type SharedState = Arc<RwLock<DashboardState>>;

pub fn new_state() -> SharedState {
    Arc::new(RwLock::new(DashboardState::default()))
}
