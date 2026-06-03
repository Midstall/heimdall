//! Tiny in-memory cache of "the last program we loaded onto the DUT
//! for this job." Populated by the worker's Fuzz dispatch (the engine
//! fires a [`heimdall_fuzzer::ProgramCallback`] before each iter's
//! load), consumed by the `/jobs/:id/disasm` route.
//!
//! Kept in-memory because the cache is informational: a daemon restart
//! drops it and the next iter repopulates. Persisting per-iter program
//! bytes is wasteful for a UX feature; if we later want post-hoc replay
//! we can plumb a BlobStore write at the same observer call site.
//!
//! [`LoadedProgramCache`] is `Clone` (the inner `Arc` is shared) so
//! the daemon can hand the same handle to both [`crate::AppState`]
//! and the worker's per-job dispatch.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use heimdall_core::ArtifactKind;

use crate::types::JobId;

/// One snapshot of "the program currently loaded on the DUT" for a
/// fuzz job. `iter` is the 0-based iteration index the engine was on
/// when it generated the artifact, surfaced in the UI so the operator
/// knows which iter they're looking at when the WS PC highlight
/// catches up.
#[derive(Debug, Clone)]
pub struct LoadedProgram {
    pub iter: u64,
    pub kind: ArtifactKind,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Default)]
pub struct LoadedProgramCache {
    inner: Arc<RwLock<HashMap<JobId, LoadedProgram>>>,
}

impl LoadedProgramCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace any prior entry for `job` with `program`. Single-writer
    /// per job in practice (the worker owns the engine), so a write
    /// lock is fine.
    pub fn record(&self, job: JobId, program: LoadedProgram) {
        let mut map = self.inner.write().expect("loaded-program lock poisoned");
        map.insert(job, program);
    }

    /// Fetch the most recently loaded program for `job`, or `None` if
    /// no iter has been recorded yet (job hasn't started or wasn't a
    /// Fuzz dispatch).
    pub fn latest(&self, job: JobId) -> Option<LoadedProgram> {
        let map = self.inner.read().expect("loaded-program lock poisoned");
        map.get(&job).cloned()
    }

    /// Drop the entry for `job`. Called from the worker on terminal
    /// transitions so the cache doesn't grow unboundedly across long-
    /// running daemons. Idempotent.
    pub fn drop_job(&self, job: JobId) {
        let mut map = self.inner.write().expect("loaded-program lock poisoned");
        map.remove(&job);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn jid() -> JobId {
        JobId(Uuid::new_v4())
    }

    #[test]
    fn record_then_latest_round_trips() {
        let cache = LoadedProgramCache::new();
        let id = jid();
        let prog = LoadedProgram {
            iter: 3,
            kind: ArtifactKind::RawBytes,
            bytes: vec![0x93, 0x00, 0x00, 0x00],
        };
        cache.record(id, prog.clone());
        let got = cache.latest(id).expect("must round-trip");
        assert_eq!(got.iter, 3);
        assert_eq!(got.bytes, prog.bytes);
        assert_eq!(got.kind, ArtifactKind::RawBytes);
    }

    #[test]
    fn record_replaces_prior_entry() {
        let cache = LoadedProgramCache::new();
        let id = jid();
        cache.record(
            id,
            LoadedProgram {
                iter: 0,
                kind: ArtifactKind::RawBytes,
                bytes: vec![1],
            },
        );
        cache.record(
            id,
            LoadedProgram {
                iter: 5,
                kind: ArtifactKind::RawBytes,
                bytes: vec![2, 3],
            },
        );
        let got = cache.latest(id).unwrap();
        assert_eq!(got.iter, 5);
        assert_eq!(got.bytes, vec![2, 3]);
    }

    #[test]
    fn latest_returns_none_for_unknown_job() {
        let cache = LoadedProgramCache::new();
        assert!(cache.latest(jid()).is_none());
    }

    #[test]
    fn drop_job_removes_entry() {
        let cache = LoadedProgramCache::new();
        let id = jid();
        cache.record(
            id,
            LoadedProgram {
                iter: 0,
                kind: ArtifactKind::RawBytes,
                bytes: vec![0],
            },
        );
        cache.drop_job(id);
        assert!(cache.latest(id).is_none());
    }
}
