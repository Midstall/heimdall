//! Per-DUT latest architectural snapshot cache.
//!
//! A background task subscribes to the [`EventBus`] and stores the most
//! recent `DutStateSnapshot` for each DUT, keyed by `(DutId, source)`. The
//! `/duts` HTTP route reads from this cache so a freshly-opened web client
//! sees the last-known register state without waiting for the next observe()
//! cycle.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use heimdall_core::{DutId, State};
use serde::Serialize;
use tokio::sync::RwLock;

use crate::event_bus::EventBus;
use crate::types::{Event, JobId, SnapshotSourceTag};

/// One cached entry: the most recent snapshot we've seen for a given
/// `(DutId, source)` pair, plus the timestamp the bus stamped on the
/// event and the job that produced it.
#[derive(Debug, Clone, Serialize)]
pub struct DutStateLatest {
    pub job: JobId,
    pub source: SnapshotSourceTag,
    pub ts: DateTime<Utc>,
    pub state: State,
}

/// Thread-safe map of `(DutId, source) -> DutStateLatest`. Cloning is cheap
/// (Arc-shared). Mutating goes through [`Self::insert`]; reading goes
/// through [`Self::snapshot_for`] or [`Self::all`].
#[derive(Clone, Default)]
pub struct DutStateCache {
    inner: Arc<RwLock<HashMap<(DutId, SnapshotSourceTag), DutStateLatest>>>,
}

impl DutStateCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Overwrite the entry for `(dut, source)`.
    pub async fn insert(&self, dut: DutId, latest: DutStateLatest) {
        let mut g = self.inner.write().await;
        g.insert((dut, latest.source), latest);
    }

    /// Return every entry for a single DUT (across all snapshot sources).
    /// Order: dut snapshots first, then golden.
    pub async fn snapshot_for(&self, dut: &DutId) -> Vec<DutStateLatest> {
        let g = self.inner.read().await;
        let mut out: Vec<DutStateLatest> = g
            .iter()
            .filter(|((d, _), _)| d == dut)
            .map(|(_, v)| v.clone())
            .collect();
        out.sort_by_key(|v| match v.source {
            SnapshotSourceTag::Dut => 0,
            SnapshotSourceTag::Golden => 1,
        });
        out
    }

    /// Borrow the whole map keyed by DUT id. Mostly for diagnostics.
    pub async fn all(&self) -> HashMap<DutId, Vec<DutStateLatest>> {
        let g = self.inner.read().await;
        let mut out: HashMap<DutId, Vec<DutStateLatest>> = HashMap::new();
        for ((dut, _), latest) in g.iter() {
            out.entry(dut.clone()).or_default().push(latest.clone());
        }
        out
    }
}

/// Spawn a background task that subscribes to `bus` and pipes every
/// `DutStateSnapshot` event into `cache`. Returns the join handle so the
/// caller can abort on shutdown.
pub fn spawn_snapshot_task(bus: &EventBus, cache: DutStateCache) -> tokio::task::JoinHandle<()> {
    let mut rx = bus.subscribe();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(stamped) => {
                    if let Event::DutStateSnapshot {
                        dut,
                        job,
                        source,
                        state,
                    } = stamped.event
                    {
                        cache
                            .insert(
                                dut.clone(),
                                DutStateLatest {
                                    job,
                                    source,
                                    ts: stamped.ts,
                                    state,
                                },
                            )
                            .await;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use heimdall_core::ValueRepr;

    #[tokio::test]
    async fn insert_overwrites_per_source() {
        let cache = DutStateCache::new();
        let dut = DutId::new("d1");
        let job = JobId::new();
        let ts = Utc::now();
        cache
            .insert(
                dut.clone(),
                DutStateLatest {
                    job,
                    source: SnapshotSourceTag::Dut,
                    ts,
                    state: State::new().with("x10", ValueRepr::U64(0x10)),
                },
            )
            .await;
        cache
            .insert(
                dut.clone(),
                DutStateLatest {
                    job,
                    source: SnapshotSourceTag::Dut,
                    ts,
                    state: State::new().with("x10", ValueRepr::U64(0x20)),
                },
            )
            .await;
        cache
            .insert(
                dut.clone(),
                DutStateLatest {
                    job,
                    source: SnapshotSourceTag::Golden,
                    ts,
                    state: State::new().with("x10", ValueRepr::U64(0x30)),
                },
            )
            .await;
        let snaps = cache.snapshot_for(&dut).await;
        assert_eq!(
            snaps.len(),
            2,
            "dut + golden survive, dut overwrites itself"
        );
        assert_eq!(snaps[0].source, SnapshotSourceTag::Dut);
        let dut_val = match snaps[0].state.fields.get("x10").unwrap() {
            ValueRepr::U64(v) => *v,
            other => panic!("wrong variant: {other:?}"),
        };
        assert_eq!(dut_val, 0x20, "second dut insert should win");
    }

    #[tokio::test]
    async fn snapshot_for_unknown_dut_returns_empty() {
        let cache = DutStateCache::new();
        assert!(cache.snapshot_for(&DutId::new("missing")).await.is_empty());
    }
}
