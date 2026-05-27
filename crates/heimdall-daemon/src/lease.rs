//! In-memory lease manager. Grants exclusive lease per DUT id and
//! auto-releases on heartbeat expiry.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use heimdall_core::DutId;
use tokio::sync::Mutex;
use tokio::time::Instant;

use crate::error::{DaemonError, Result};
use crate::types::{JobId, Lease, LeaseId};

#[derive(Debug, Clone, Copy)]
pub struct LeaseTtl(pub Duration);

impl Default for LeaseTtl {
    fn default() -> Self {
        Self(Duration::from_secs(60))
    }
}

#[derive(Clone)]
pub struct LeaseManager {
    inner: Arc<Mutex<LeaseInner>>,
    ttl: LeaseTtl,
}

struct LeaseInner {
    by_dut: HashMap<DutId, (Lease, Instant)>,
}

impl LeaseManager {
    pub fn new(ttl: LeaseTtl) -> Self {
        Self {
            inner: Arc::new(Mutex::new(LeaseInner {
                by_dut: HashMap::new(),
            })),
            ttl,
        }
    }

    /// Attempt to acquire an exclusive lease on the DUT for the given job.
    /// Returns `Err(LeaseExpired)` if the DUT is already leased and not yet
    /// expired.
    pub async fn acquire(&self, dut: DutId, holder: JobId) -> Result<Lease> {
        let mut inner = self.inner.lock().await;
        self.gc_expired(&mut inner);
        if let Some((existing, _)) = inner.by_dut.get(&dut) {
            return Err(DaemonError::LeaseExpired(format!(
                "dut `{}` already leased by job {}",
                dut.0, existing.holder
            )));
        }
        let now = Utc::now();
        let lease = Lease {
            id: LeaseId::new(),
            dut: dut.clone(),
            holder,
            acquired_at: now,
            expires_at: now
                + chrono::Duration::from_std(self.ttl.0)
                    .unwrap_or_else(|_| chrono::Duration::seconds(60)),
        };
        inner
            .by_dut
            .insert(dut, (lease.clone(), Instant::now() + self.ttl.0));
        Ok(lease)
    }

    /// Heartbeat: extend the lease if the lease id matches what the manager
    /// holds. Returns `Err` if there's no live lease for this DUT or the id
    /// does not match.
    pub async fn heartbeat(&self, dut: &DutId, lease: LeaseId) -> Result<()> {
        let mut inner = self.inner.lock().await;
        self.gc_expired(&mut inner);
        match inner.by_dut.get_mut(dut) {
            Some((existing, deadline)) if existing.id == lease => {
                *deadline = Instant::now() + self.ttl.0;
                existing.expires_at = Utc::now()
                    + chrono::Duration::from_std(self.ttl.0)
                        .unwrap_or_else(|_| chrono::Duration::seconds(60));
                Ok(())
            }
            _ => Err(DaemonError::LeaseExpired(dut.0.clone())),
        }
    }

    pub async fn release(&self, dut: &DutId, lease: LeaseId) -> Result<()> {
        let mut inner = self.inner.lock().await;
        self.gc_expired(&mut inner);
        match inner.by_dut.get(dut) {
            Some((existing, _)) if existing.id == lease => {
                inner.by_dut.remove(dut);
                Ok(())
            }
            _ => Err(DaemonError::LeaseExpired(dut.0.clone())),
        }
    }

    /// Return a snapshot of currently held leases.
    pub async fn list(&self) -> Vec<Lease> {
        let inner = self.inner.lock().await;
        inner.by_dut.values().map(|(l, _)| l.clone()).collect()
    }

    fn gc_expired(&self, inner: &mut LeaseInner) {
        let now = Instant::now();
        inner.by_dut.retain(|_, (_, deadline)| *deadline > now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn mgr() -> LeaseManager {
        LeaseManager::new(LeaseTtl(Duration::from_millis(100)))
    }

    #[tokio::test]
    async fn acquire_grants_lease() {
        let mgr = mgr();
        let lease = mgr.acquire(DutId::new("d1"), JobId::new()).await.unwrap();
        assert_eq!(lease.dut.0, "d1");
    }

    #[tokio::test]
    async fn acquire_twice_same_dut_fails() {
        let mgr = mgr();
        let _ = mgr.acquire(DutId::new("d1"), JobId::new()).await.unwrap();
        let err = mgr
            .acquire(DutId::new("d1"), JobId::new())
            .await
            .unwrap_err();
        assert!(matches!(err, DaemonError::LeaseExpired(_)));
    }

    #[tokio::test]
    async fn acquire_different_duts_independent() {
        let mgr = mgr();
        let _ = mgr.acquire(DutId::new("d1"), JobId::new()).await.unwrap();
        let _ = mgr.acquire(DutId::new("d2"), JobId::new()).await.unwrap();
    }

    #[tokio::test]
    async fn expiry_releases_lease() {
        let mgr = mgr();
        let _ = mgr.acquire(DutId::new("d1"), JobId::new()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(150)).await;
        // After TTL, the DUT is free to acquire again.
        let _ = mgr.acquire(DutId::new("d1"), JobId::new()).await.unwrap();
    }

    #[tokio::test]
    async fn release_frees_dut() {
        let mgr = mgr();
        let lease = mgr.acquire(DutId::new("d1"), JobId::new()).await.unwrap();
        mgr.release(&DutId::new("d1"), lease.id).await.unwrap();
        // Can acquire again immediately.
        let _ = mgr.acquire(DutId::new("d1"), JobId::new()).await.unwrap();
    }

    #[tokio::test]
    async fn heartbeat_extends_lease() {
        let mgr = mgr();
        let lease = mgr.acquire(DutId::new("d1"), JobId::new()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(60)).await;
        mgr.heartbeat(&DutId::new("d1"), lease.id).await.unwrap();
        tokio::time::sleep(Duration::from_millis(60)).await;
        // Still locked because heartbeat reset the deadline.
        let err = mgr
            .acquire(DutId::new("d1"), JobId::new())
            .await
            .unwrap_err();
        assert!(matches!(err, DaemonError::LeaseExpired(_)));
    }
}
