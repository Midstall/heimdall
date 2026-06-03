//! Per-job cancellation handles for in-flight worker tasks.
//!
//! The cancel HTTP route looks up the token by [`JobId`] and calls
//! `.cancel()` on it; the worker races its run future against the
//! token via `tokio::select!`, propagating an early
//! [`crate::error::DaemonError::Cancelled`] when the operator
//! signals. Tokens are removed from the map once the worker
//! finishes handling the job (terminal transition or cancellation),
//! so the registry only ever holds tokens for jobs currently in
//! flight.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use tokio_util::sync::CancellationToken;

use crate::types::JobId;

/// Map of currently-running jobs to their cancellation token.
/// `Clone` is cheap (Arc); the same handle lives in [`AppState`]
/// (for the cancel HTTP route) and the worker (for registration +
/// race).
#[derive(Clone, Default)]
pub struct CancellationRegistry {
    inner: Arc<RwLock<HashMap<JobId, CancellationToken>>>,
}

impl CancellationRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a fresh token for `job` and return a handle. The
    /// worker holds the returned token to await its `.cancelled()`
    /// future inside `tokio::select!`; the registry retains a clone
    /// that the cancel route can fire.
    pub fn register(&self, job: JobId) -> CancellationToken {
        let token = CancellationToken::new();
        let mut map = self.inner.write().expect("cancellation lock poisoned");
        map.insert(job, token.clone());
        token
    }

    /// Drop the entry for `job`. Called by the worker after handling
    /// completes (either normally or via cancellation) so stale
    /// tokens don't pile up after long-running daemons see
    /// thousands of jobs.
    pub fn remove(&self, job: JobId) {
        let mut map = self.inner.write().expect("cancellation lock poisoned");
        map.remove(&job);
    }

    /// Fire the token for `job` if registered, returning `true` if
    /// a token was found and signalled. The cancel HTTP route uses
    /// the bool to distinguish "running job successfully signalled"
    /// from "job no longer running, cancel via queue transition
    /// instead."
    pub fn cancel(&self, job: JobId) -> bool {
        let map = self.inner.read().expect("cancellation lock poisoned");
        match map.get(&job) {
            Some(token) => {
                token.cancel();
                true
            }
            None => false,
        }
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
    fn register_then_cancel_signals_the_token() {
        let reg = CancellationRegistry::new();
        let id = jid();
        let token = reg.register(id);
        assert!(!token.is_cancelled());
        assert!(reg.cancel(id));
        assert!(token.is_cancelled());
    }

    #[test]
    fn cancel_unknown_job_reports_false() {
        let reg = CancellationRegistry::new();
        assert!(!reg.cancel(jid()));
    }

    #[test]
    fn remove_drops_the_registry_entry() {
        let reg = CancellationRegistry::new();
        let id = jid();
        let _t = reg.register(id);
        reg.remove(id);
        assert!(!reg.cancel(id));
    }

    /// Mirror of the `tokio::select!` race in `worker::handle_one`:
    /// a long-running future loses to the cancellation token when
    /// the route fires it. Guards against regressions where the
    /// registry's token is dropped or re-created somewhere along
    /// the path between registry and worker.
    #[tokio::test]
    async fn race_pattern_aborts_long_future_on_cancel() {
        use std::time::Duration;

        let reg = CancellationRegistry::new();
        let id = jid();
        let token = reg.register(id);
        let task = tokio::spawn(async move {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(60)) => false,
                _ = token.cancelled() => true,
            }
        });
        // Give the spawned task a chance to install both arms of the
        // select! before we fire. Without the yield the cancel can
        // race the task's first poll.
        tokio::task::yield_now().await;
        assert!(reg.cancel(id), "cancel must report a hit for a live job");
        let cancelled_arm_won = tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .expect("task did not finish in time");
        assert!(
            cancelled_arm_won.expect("task panicked"),
            "select! should have resolved on the cancelled() arm",
        );
    }
}
