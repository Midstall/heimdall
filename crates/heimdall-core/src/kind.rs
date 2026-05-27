use serde::{Deserialize, Serialize};

/// All DUT kinds heimdall knows about.
/// Adding a variant is intentional and a minor-version bump.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DutKind {
    AegisLuna1,
    AegisTerra1,
    RiverRc1Nano,
    RiverRc1Micro,
    RiverRc1Small,
    RiverRc1Medium,
}

impl DutKind {
    pub fn family(self) -> Family {
        match self {
            Self::AegisLuna1 | Self::AegisTerra1 => Family::Fpga,
            Self::RiverRc1Nano
            | Self::RiverRc1Micro
            | Self::RiverRc1Small
            | Self::RiverRc1Medium => Family::Cpu,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Fpga,
    Cpu,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_roundtrip() {
        let k = DutKind::RiverRc1Nano;
        let j = serde_json::to_string(&k).unwrap();
        assert_eq!(j, "\"river-rc1-nano\"");
        let back: DutKind = serde_json::from_str(&j).unwrap();
        assert_eq!(back, k);
    }

    #[test]
    fn family_classification() {
        assert_eq!(DutKind::AegisLuna1.family(), Family::Fpga);
        assert_eq!(DutKind::RiverRc1Small.family(), Family::Cpu);
    }
}
