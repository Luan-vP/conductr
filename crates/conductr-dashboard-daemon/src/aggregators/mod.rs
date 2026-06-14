pub mod cron;
pub mod pod;
pub mod repos;

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use conductr_core::ports::{CrontabAgent, TmuxAgent};

use crate::state::SharedState;
use conductr_dashboard_core::SseEvent;
use tokio::sync::broadcast;

/// A pluggable aggregator that refreshes one section of `DashboardState` and
/// optionally emits SSE events.
#[async_trait]
pub trait Aggregator: Send + Sync {
    async fn refresh(
        &self,
        state: &SharedState,
        tx: &broadcast::Sender<SseEvent>,
    ) -> Result<()>;
}

/// Start all aggregators on a recurring poll loop.
pub async fn run_all(
    state: SharedState,
    tx: broadcast::Sender<SseEvent>,
    interval: std::time::Duration,
    tmux: Arc<dyn TmuxAgent>,
    crontab: Arc<dyn CrontabAgent>,
) {
    let aggregators: Vec<Box<dyn Aggregator>> = vec![
        Box::new(repos::ReposAggregator::new()),
        Box::new(pod::PodAggregator::new(tmux)),
        Box::new(cron::CronAggregator::new(crontab)),
    ];

    let mut ticker = tokio::time::interval(interval);
    loop {
        ticker.tick().await;
        for agg in &aggregators {
            if let Err(e) = agg.refresh(&state, &tx).await {
                tracing::warn!("aggregator error: {e:#}");
            }
        }
    }
}
