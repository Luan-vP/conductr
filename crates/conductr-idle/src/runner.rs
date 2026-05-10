use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::sink::{PrintSink, Sink, SubmitOutcome};
use crate::sweep::{Finding, Sweep, SweepContext};

/// Partition findings across `n` instances. Each finding's stable id is hashed
/// to a slot; only findings whose slot equals `index` are kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shard {
    pub index: u32,
    pub of: u32,
}

impl Shard {
    pub fn includes(&self, finding_id: &str) -> bool {
        if self.of <= 1 {
            return true;
        }
        let mut h = DefaultHasher::new();
        finding_id.hash(&mut h);
        (h.finish() % self.of as u64) as u32 == self.index
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunReport {
    pub findings: Vec<Finding>,
    pub created: Vec<String>,
    pub already_existed: Vec<String>,
    pub skipped: Vec<String>,
    pub errors: Vec<(String, String)>,
}

pub struct Runner {
    pub sweeps: Vec<Arc<dyn Sweep>>,
    pub sink: Arc<dyn Sink>,
    pub shard: Option<Shard>,
    /// If true, only collect findings; do not call into the sink (other than for printing).
    pub dry_run: bool,
}

impl Runner {
    pub fn new(sweeps: Vec<Arc<dyn Sweep>>, sink: Arc<dyn Sink>) -> Self {
        Self { sweeps, sink, shard: None, dry_run: false }
    }

    pub fn with_shard(mut self, shard: Shard) -> Self {
        self.shard = Some(shard);
        self
    }

    pub fn dry_run(mut self, on: bool) -> Self {
        self.dry_run = on;
        self
    }

    /// Run all sweeps, dispatch findings (after sharding) through the sink.
    pub async fn run(&self, ctx: &SweepContext) -> anyhow::Result<RunReport> {
        let mut report = RunReport {
            findings: Vec::new(),
            created: Vec::new(),
            already_existed: Vec::new(),
            skipped: Vec::new(),
            errors: Vec::new(),
        };

        for sweep in &self.sweeps {
            match sweep.run(ctx).await {
                Ok(findings) => {
                    for f in findings {
                        if let Some(shard) = self.shard {
                            if !shard.includes(&f.id) {
                                continue;
                            }
                        }
                        report.findings.push(f.clone());
                        if self.dry_run {
                            // Dry-run still routes through PrintSink so the
                            // operator sees what *would* be filed.
                            let _ = PrintSink.submit(&f).await;
                            continue;
                        }
                        match self.sink.submit(&f).await {
                            Ok(SubmitOutcome::Created(id)) => report.created.push(id),
                            Ok(SubmitOutcome::AlreadyExists) => report.already_existed.push(f.id.clone()),
                            Ok(SubmitOutcome::Skipped) => report.skipped.push(f.id.clone()),
                            Err(e) => report.errors.push((f.id.clone(), e.to_string())),
                        }
                    }
                }
                Err(e) => {
                    report.errors.push((sweep.name().to_string(), e.to_string()));
                }
            }
        }
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sweep::Severity;

    #[test]
    fn shard_keeps_or_drops_consistently() {
        let s = Shard { index: 0, of: 4 };
        let id = "security:cve-x".to_string();
        let first = s.includes(&id);
        for _ in 0..10 {
            assert_eq!(s.includes(&id), first);
        }
    }

    #[test]
    fn shard_partitions_distinct_ids() {
        let n = 4u32;
        let shards: Vec<_> = (0..n).map(|i| Shard { index: i, of: n }).collect();
        let ids = (0..200).map(|i| format!("git-flow:gap-{i}"));
        for id in ids {
            let count = shards.iter().filter(|s| s.includes(&id)).count();
            assert_eq!(count, 1);
        }
    }

    #[tokio::test]
    async fn runner_dry_run_collects_without_calling_sink() {
        struct Fake;
        #[async_trait::async_trait]
        impl Sink for Fake {
            async fn submit(&self, _: &Finding) -> anyhow::Result<SubmitOutcome> {
                panic!("dry run should not hit the sink")
            }
        }
        struct DummySweep;
        #[async_trait::async_trait]
        impl Sweep for DummySweep {
            fn name(&self) -> &'static str { "dummy" }
            async fn run(&self, _: &SweepContext) -> anyhow::Result<Vec<Finding>> {
                Ok(vec![Finding::new("dummy", "k", "t", "b", Severity::Info)])
            }
        }
        let runner = Runner::new(vec![Arc::new(DummySweep)], Arc::new(Fake)).dry_run(true);
        let ctx = SweepContext::new(std::path::PathBuf::from("."));
        let report = runner.run(&ctx).await.unwrap();
        assert_eq!(report.findings.len(), 1);
        assert!(report.created.is_empty());
    }
}
