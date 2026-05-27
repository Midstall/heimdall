//! Coverage state used by the FuzzerEngine. CoverageSource (defined in
//! heimdall-golden) is the per-iteration snapshot producer; CoverageMap is
//! the fuzzer's global union of all snapshots seen.

use std::fmt;

/// Default size of the coverage bitmap, in bytes. 1024 B = 8,192 bits.
pub const DEFAULT_BUCKETS: usize = 1024;

/// A snapshot of coverage from one iteration. Just bytes; layout is
/// per-CoverageSource (for example spike's PC bitmap).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverageSnapshot(pub Vec<u8>);

impl CoverageSnapshot {
    pub fn new(size: usize) -> Self {
        Self(vec![0u8; size])
    }

    pub fn set_bit(&mut self, idx: usize) {
        let byte = idx / 8;
        let bit = (idx % 8) as u8;
        if let Some(b) = self.0.get_mut(byte) {
            *b |= 1 << bit;
        }
    }

    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }

    pub fn bits_set(&self) -> usize {
        self.0.iter().map(|b| b.count_ones() as usize).sum()
    }
}

/// Global union of all snapshots seen during a fuzz run. `merge` returns
/// true only if new bits were added.
#[derive(Debug, Clone)]
pub struct CoverageMap {
    bits: Vec<u8>,
}

impl CoverageMap {
    pub fn new(size: usize) -> Self {
        Self {
            bits: vec![0u8; size],
        }
    }

    pub fn with_buckets(buckets: usize) -> Self {
        Self::new(buckets)
    }

    pub fn merge(&mut self, snap: &CoverageSnapshot) -> bool {
        let n = self.bits.len().min(snap.0.len());
        let mut new_bits = false;
        for i in 0..n {
            let before = self.bits[i];
            let after = before | snap.0[i];
            if after != before {
                new_bits = true;
            }
            self.bits[i] = after;
        }
        new_bits
    }

    pub fn bits_set(&self) -> usize {
        self.bits.iter().map(|b| b.count_ones() as usize).sum()
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.bits
    }

    pub fn size(&self) -> usize {
        self.bits.len()
    }
}

impl Default for CoverageMap {
    fn default() -> Self {
        Self::new(DEFAULT_BUCKETS)
    }
}

/// Result of comparing two coverage bitmaps.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CoverageDiff {
    /// Bits set in self that are NOT set in other.
    pub self_only_bits: usize,
    /// Bits set in other that are NOT set in self.
    pub other_only_bits: usize,
    /// Bits set in both.
    pub common_bits: usize,
}

impl CoverageDiff {
    pub fn is_divergent(&self) -> bool {
        self.self_only_bits > 0 || self.other_only_bits > 0
    }
}

impl CoverageSnapshot {
    pub fn diff_from(&self, other: &CoverageSnapshot) -> CoverageDiff {
        let n = self.0.len().min(other.0.len());
        let mut self_only = 0usize;
        let mut other_only = 0usize;
        let mut common = 0usize;
        for i in 0..n {
            let a = self.0[i];
            let b = other.0[i];
            self_only += (a & !b).count_ones() as usize;
            other_only += (b & !a).count_ones() as usize;
            common += (a & b).count_ones() as usize;
        }
        CoverageDiff {
            self_only_bits: self_only,
            other_only_bits: other_only,
            common_bits: common,
        }
    }
}

impl CoverageMap {
    pub fn diff_from(&self, other: &CoverageMap) -> CoverageDiff {
        let n = self.bits.len().min(other.bits.len());
        let mut self_only = 0usize;
        let mut other_only = 0usize;
        let mut common = 0usize;
        for i in 0..n {
            let a = self.bits[i];
            let b = other.bits[i];
            self_only += (a & !b).count_ones() as usize;
            other_only += (b & !a).count_ones() as usize;
            common += (a & b).count_ones() as usize;
        }
        CoverageDiff {
            self_only_bits: self_only,
            other_only_bits: other_only,
            common_bits: common,
        }
    }
}

impl fmt::Display for CoverageMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "CoverageMap({}/{} bits)",
            self.bits_set(),
            self.bits.len() * 8
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_returns_true_only_when_new_bits() {
        let mut map = CoverageMap::new(4);
        let mut snap = CoverageSnapshot::new(4);
        snap.set_bit(3);
        snap.set_bit(8);
        assert!(map.merge(&snap));
        assert_eq!(map.bits_set(), 2);
        // Same snapshot again: no new bits.
        assert!(!map.merge(&snap));
        assert_eq!(map.bits_set(), 2);
    }

    #[test]
    fn snapshot_bit_setting() {
        let mut s = CoverageSnapshot::new(2);
        s.set_bit(0);
        s.set_bit(7);
        s.set_bit(8);
        s.set_bit(15);
        assert_eq!(s.bits_set(), 4);
        assert_eq!(s.as_slice(), &[0x81, 0x81]);
    }

    #[test]
    fn diff_detects_unique_bits() {
        let mut a = CoverageSnapshot::new(4);
        a.set_bit(3);
        a.set_bit(8);
        let mut b = CoverageSnapshot::new(4);
        b.set_bit(8);
        b.set_bit(15);
        let d = a.diff_from(&b);
        assert_eq!(d.self_only_bits, 1);
        assert_eq!(d.other_only_bits, 1);
        assert_eq!(d.common_bits, 1);
        assert!(d.is_divergent());
    }

    #[test]
    fn identical_snapshots_dont_diverge() {
        let mut a = CoverageSnapshot::new(4);
        a.set_bit(3);
        let b = a.clone();
        let d = a.diff_from(&b);
        assert!(!d.is_divergent());
        assert_eq!(d.common_bits, 1);
    }
}
