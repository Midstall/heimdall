//! In-memory cache of per-DUT ISA probe results.
//!
//! The fuzz worker, on first dispatch against a given DUT, calls
//! `driver.probe_isa()` over JTAG (reads CSR `misa` via DMI abstract
//! command) and stores the result here keyed by [`DutId`]. Subsequent
//! fuzz jobs against the same DUT skip the probe and reuse the
//! cached value, so the once-per-daemon-lifetime probe cost stays
//! amortised.
//!
//! Precedence (resolved by the worker, not this module):
//! 1. Explicit `[dut.isa]` block in heimdall.toml. Operator-authoritative
//!    so the fuzzer can be told to emit a subset of what the DUT
//!    actually supports.
//! 2. Cached probe result from this cache.
//! 3. Default RV32I.
//!
//! In-memory only; a daemon restart drops it and the next fuzz
//! against each DUT re-probes.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use heimdall_core::DutId;
use heimdall_driver::IsaProbe;

use crate::dut_registry::IsaSpec;

/// Probed ISA capabilities for a DUT, normalised to the same shape
/// the registry-side [`crate::dut_registry::IsaSpec`] uses so the
/// worker can treat both sources uniformly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbedIsa {
    pub xlen: u8,
    /// Canonical extension identifiers: misa letters (`"i"`, `"m"`,
    /// ...) or Z-style names. Empty when no extensions are decoded
    /// (e.g. misa returned all zeros).
    pub extensions: Vec<String>,
}

/// A driver-returned probe maps 1:1 to a cache entry with the same
/// shape and semantics. The `From` impl lets the worker stash a probe
/// result with `cache.insert(dut, probe.into())`.
impl From<IsaProbe> for ProbedIsa {
    fn from(probe: IsaProbe) -> Self {
        Self {
            xlen: probe.xlen,
            extensions: probe.extensions,
        }
    }
}

/// Same shape as the TOML-mirror [`IsaSpec`], differing only in
/// intent. Lets the worker layer cache results into the
/// `Option<&IsaSpec>` pipeline `resolve_isa` consumes.
impl From<&ProbedIsa> for IsaSpec {
    fn from(probe: &ProbedIsa) -> Self {
        Self {
            xlen: probe.xlen,
            extensions: probe.extensions.clone(),
        }
    }
}

#[derive(Clone, Default)]
pub struct IsaProbeCache {
    inner: Arc<RwLock<HashMap<DutId, ProbedIsa>>>,
}

impl IsaProbeCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, dut: &DutId) -> Option<ProbedIsa> {
        let map = self.inner.read().expect("isa-probe lock poisoned");
        map.get(dut).cloned()
    }

    pub fn insert(&self, dut: DutId, probe: ProbedIsa) {
        let mut map = self.inner.write().expect("isa-probe lock poisoned");
        map.insert(dut, probe);
    }

    /// Drop a cached entry. Useful when the operator re-flashes a
    /// DUT and the misa shape may have changed.
    pub fn forget(&self, dut: &DutId) {
        let mut map = self.inner.write().expect("isa-probe lock poisoned");
        map.remove(dut);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_get_insert_forget() {
        let cache = IsaProbeCache::new();
        let id = DutId::new("river-1");
        assert!(cache.get(&id).is_none());
        cache.insert(
            id.clone(),
            ProbedIsa {
                xlen: 64,
                extensions: vec!["i".into(), "m".into()],
            },
        );
        let got = cache.get(&id).unwrap();
        assert_eq!(got.xlen, 64);
        assert_eq!(got.extensions, vec!["i".to_string(), "m".to_string()]);
        cache.forget(&id);
        assert!(cache.get(&id).is_none());
    }

    #[test]
    fn insert_replaces_prior() {
        let cache = IsaProbeCache::new();
        let id = DutId::new("river-1");
        cache.insert(
            id.clone(),
            ProbedIsa {
                xlen: 32,
                extensions: vec!["i".into()],
            },
        );
        cache.insert(
            id.clone(),
            ProbedIsa {
                xlen: 64,
                extensions: vec!["i".into(), "m".into(), "c".into()],
            },
        );
        let got = cache.get(&id).unwrap();
        assert_eq!(got.xlen, 64);
        assert_eq!(got.extensions.len(), 3);
    }
}
