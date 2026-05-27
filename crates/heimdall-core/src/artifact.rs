use bytes::Bytes;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BitstreamFormat {
    AegisRaw,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ArtifactKind {
    Asm,
    ElfRiscv,
    Bitstream { format: BitstreamFormat },
    Verilog,
    RohdDart,
    RawBytes,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    pub source_sha256: String,
    pub tool_chain: Vec<String>,
    pub parent_sha256: Option<String>,
}

impl Provenance {
    pub fn source(source_sha256: String) -> Self {
        Self {
            source_sha256,
            tool_chain: Vec::new(),
            parent_sha256: None,
        }
    }
}

#[derive(Clone)]
pub struct Artifact {
    pub kind: ArtifactKind,
    pub bytes: Bytes,
    pub provenance: Provenance,
}

impl Artifact {
    pub fn new(kind: ArtifactKind, bytes: impl Into<Bytes>) -> Self {
        let bytes = bytes.into();
        let sha = sha256_hex(&bytes);
        Self {
            kind,
            bytes,
            provenance: Provenance::source(sha),
        }
    }

    pub fn sha256(&self) -> String {
        sha256_hex(&self.bytes)
    }
}

impl fmt::Debug for Artifact {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Artifact")
            .field("kind", &self.kind)
            .field("len", &self.bytes.len())
            .field("sha256", &self.sha256())
            .field("provenance", &self.provenance)
            .finish()
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_known_value() {
        assert_eq!(
            sha256_hex(&[]),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn artifact_kind_roundtrip() {
        let k = ArtifactKind::Bitstream {
            format: BitstreamFormat::AegisRaw,
        };
        let j = serde_json::to_string(&k).unwrap();
        let back: ArtifactKind = serde_json::from_str(&j).unwrap();
        assert_eq!(back, k);
    }

    #[test]
    fn artifact_new_sets_provenance() {
        let a = Artifact::new(ArtifactKind::RawBytes, &b"hi"[..]);
        assert_eq!(a.bytes.len(), 2);
        assert_eq!(a.sha256(), a.provenance.source_sha256);
    }
}
