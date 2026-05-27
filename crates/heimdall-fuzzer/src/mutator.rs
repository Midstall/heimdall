//! Classic AFL-style mutators. Each impl preserves the parent's ArtifactKind
//! and Provenance lineage.

use bytes::Bytes;
use heimdall_core::{Artifact, Provenance};
use rand::{Rng, RngCore};

use crate::traits::Mutator;

/// Flip a single random bit in the parent's bytes.
pub struct BitFlipMutator;

impl Mutator for BitFlipMutator {
    fn name(&self) -> &str {
        "bit-flip"
    }

    fn mutate(&mut self, parent: &Artifact, rng: &mut dyn RngCore) -> Artifact {
        let mut bytes: Vec<u8> = parent.bytes.to_vec();
        if !bytes.is_empty() {
            let byte_idx = rng.gen_range(0..bytes.len());
            let bit_idx = rng.gen_range(0..8u8);
            bytes[byte_idx] ^= 1 << bit_idx;
        }
        derived_artifact(parent, bytes, self.name())
    }
}

/// Overwrite a single random byte with a random value.
pub struct ByteFlipMutator;

impl Mutator for ByteFlipMutator {
    fn name(&self) -> &str {
        "byte-flip"
    }

    fn mutate(&mut self, parent: &Artifact, rng: &mut dyn RngCore) -> Artifact {
        let mut bytes: Vec<u8> = parent.bytes.to_vec();
        if !bytes.is_empty() {
            let byte_idx = rng.gen_range(0..bytes.len());
            // Pick a new byte value different from the current one to guarantee
            // the mutation actually changes the artifact.
            let current = bytes[byte_idx];
            let mut next = rng.r#gen::<u8>();
            if next == current {
                next = next.wrapping_add(1);
            }
            bytes[byte_idx] = next;
        }
        derived_artifact(parent, bytes, self.name())
    }
}

/// Splice a random sub-range of the parent into a different position.
/// The artifact length is preserved.
pub struct SpliceMutator;

impl Mutator for SpliceMutator {
    fn name(&self) -> &str {
        "splice"
    }

    fn mutate(&mut self, parent: &Artifact, rng: &mut dyn RngCore) -> Artifact {
        let mut bytes: Vec<u8> = parent.bytes.to_vec();
        if bytes.len() >= 4 {
            let len = bytes.len();
            let chunk_len = rng.gen_range(1..=len / 2);
            let src = rng.gen_range(0..=len - chunk_len);
            let dst = rng.gen_range(0..=len - chunk_len);
            if src != dst {
                let chunk: Vec<u8> = bytes[src..src + chunk_len].to_vec();
                bytes[dst..dst + chunk_len].copy_from_slice(&chunk);
            }
        }
        derived_artifact(parent, bytes, self.name())
    }
}

fn derived_artifact(parent: &Artifact, bytes: Vec<u8>, mutator: &str) -> Artifact {
    Artifact {
        kind: parent.kind.clone(),
        bytes: Bytes::from(bytes),
        provenance: Provenance {
            source_sha256: parent.provenance.source_sha256.clone(),
            tool_chain: {
                let mut tc = parent.provenance.tool_chain.clone();
                tc.push(format!("mutator:{mutator}"));
                tc
            },
            parent_sha256: Some(parent.sha256()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use heimdall_core::ArtifactKind;
    use rand::SeedableRng;
    use rand::rngs::StdRng;

    fn rng() -> StdRng {
        StdRng::seed_from_u64(0xdeadbeef)
    }

    fn parent() -> Artifact {
        Artifact::new(ArtifactKind::RawBytes, (0u8..32).collect::<Vec<u8>>())
    }

    #[test]
    fn bit_flip_preserves_kind_and_changes_bytes() {
        let p = parent();
        let mutated = BitFlipMutator.mutate(&p, &mut rng());
        assert!(matches!(mutated.kind, ArtifactKind::RawBytes));
        assert_eq!(mutated.bytes.len(), p.bytes.len());
        assert_ne!(mutated.bytes, p.bytes);
        assert_eq!(
            mutated.provenance.parent_sha256.as_deref(),
            Some(p.sha256()).as_deref()
        );
        assert!(
            mutated
                .provenance
                .tool_chain
                .iter()
                .any(|s| s == "mutator:bit-flip")
        );
    }

    #[test]
    fn byte_flip_changes_exactly_one_byte() {
        let p = parent();
        let mutated = ByteFlipMutator.mutate(&p, &mut rng());
        assert_eq!(mutated.bytes.len(), p.bytes.len());
        let diffs = p
            .bytes
            .iter()
            .zip(mutated.bytes.iter())
            .filter(|(a, b)| a != b)
            .count();
        assert_eq!(diffs, 1);
    }

    #[test]
    fn splice_preserves_length_and_kind() {
        let p = parent();
        let mutated = SpliceMutator.mutate(&p, &mut rng());
        assert!(matches!(mutated.kind, ArtifactKind::RawBytes));
        assert_eq!(mutated.bytes.len(), p.bytes.len());
    }

    #[test]
    fn mutators_handle_empty_input() {
        let p = Artifact::new(ArtifactKind::RawBytes, Vec::<u8>::new());
        let m1 = BitFlipMutator.mutate(&p, &mut rng());
        let m2 = ByteFlipMutator.mutate(&p, &mut rng());
        let m3 = SpliceMutator.mutate(&p, &mut rng());
        assert!(m1.bytes.is_empty());
        assert!(m2.bytes.is_empty());
        assert!(m3.bytes.is_empty());
    }

    #[test]
    fn provenance_chain_extends() {
        let p = parent();
        let a = BitFlipMutator.mutate(&p, &mut rng());
        let b = BitFlipMutator.mutate(&a, &mut rng());
        // The second mutation's parent_sha256 should point to a's sha.
        assert_eq!(
            b.provenance.parent_sha256.as_deref(),
            Some(a.sha256()).as_deref()
        );
        // Both mutator entries appear in b's tool_chain.
        assert_eq!(b.provenance.tool_chain.len(), 2);
    }
}
