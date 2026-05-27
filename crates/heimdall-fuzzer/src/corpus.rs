use std::collections::HashMap;

use heimdall_core::{Artifact, SeedId, Verdict};

use crate::coverage::CoverageSnapshot;

#[derive(Debug, Clone)]
pub struct CorpusEntry {
    pub seed: SeedId,
    pub artifact: Artifact,
    pub parent: Option<SeedId>,
    pub last_verdict: Option<VerdictTag>,
    #[allow(dead_code)]
    pub last_snapshot: Option<CoverageSnapshot>,
    pub last_was_novel: bool,
}

/// Compact verdict summary; full Verdict is held elsewhere if needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerdictTag {
    Pass,
    Fail,
    Skip,
    Error,
}

impl From<&Verdict> for VerdictTag {
    fn from(v: &Verdict) -> Self {
        match v {
            Verdict::Pass => VerdictTag::Pass,
            Verdict::Fail { .. } => VerdictTag::Fail,
            Verdict::Skip { .. } => VerdictTag::Skip,
            Verdict::Error { .. } => VerdictTag::Error,
        }
    }
}

#[derive(Debug, Default)]
pub struct Corpus {
    by_seed: HashMap<SeedId, CorpusEntry>,
    by_sha: HashMap<String, SeedId>,
    order: Vec<SeedId>,
}

impl Corpus {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    /// Add an entry. Deduplicates on the artifact sha; if already present,
    /// returns the existing SeedId without modifying anything.
    pub fn add(&mut self, entry: CorpusEntry) -> SeedId {
        let sha = entry.artifact.sha256();
        if let Some(existing) = self.by_sha.get(&sha).copied() {
            return existing;
        }
        let seed = entry.seed;
        self.by_sha.insert(sha, seed);
        self.by_seed.insert(seed, entry);
        self.order.push(seed);
        seed
    }

    pub fn get_by_index(&self, idx: usize) -> Option<&CorpusEntry> {
        let seed = self.order.get(idx).copied()?;
        self.by_seed.get(&seed)
    }

    pub fn get_by_seed(&self, seed: SeedId) -> Option<&CorpusEntry> {
        self.by_seed.get(&seed)
    }

    pub fn update_verdict(&mut self, seed: SeedId, verdict: &heimdall_core::Verdict) {
        if let Some(entry) = self.by_seed.get_mut(&seed) {
            entry.last_verdict = Some(VerdictTag::from(verdict));
        }
    }

    pub fn update_coverage(&mut self, seed: SeedId, snapshot: CoverageSnapshot, novel: bool) {
        if let Some(entry) = self.by_seed.get_mut(&seed) {
            entry.last_snapshot = Some(snapshot);
            entry.last_was_novel = novel;
        }
    }

    pub fn novel_indices(&self) -> Vec<usize> {
        self.order
            .iter()
            .enumerate()
            .filter_map(|(i, seed)| {
                let e = self.by_seed.get(seed)?;
                if e.last_was_novel { Some(i) } else { None }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use heimdall_core::{Artifact, ArtifactKind};

    fn entry(seed_id: u64, bytes: &[u8]) -> CorpusEntry {
        CorpusEntry {
            seed: SeedId(seed_id),
            artifact: Artifact::new(ArtifactKind::RawBytes, bytes.to_vec()),
            parent: None,
            last_verdict: None,
            last_snapshot: None,
            last_was_novel: false,
        }
    }

    #[test]
    fn add_and_index() {
        let mut c = Corpus::new();
        c.add(entry(1, b"a"));
        c.add(entry(2, b"b"));
        assert_eq!(c.len(), 2);
        assert_eq!(c.get_by_index(0).unwrap().seed, SeedId(1));
        assert_eq!(c.get_by_index(1).unwrap().seed, SeedId(2));
    }

    #[test]
    fn dedup_on_sha() {
        let mut c = Corpus::new();
        c.add(entry(1, b"same"));
        let seed = c.add(entry(2, b"same"));
        assert_eq!(c.len(), 1, "dup not deduped");
        assert_eq!(seed, SeedId(1), "returned the original seed");
    }
}
