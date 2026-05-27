//! Silicon-side coverage for RiverCpuDriver. v1 hashes the observed PC into
//! a 1024-byte (8192-bit) bitmap. Sparse but real signal.

use heimdall_golden::CoverageSource;

pub struct RiverSiliconCoverage {
    bits: Vec<u8>,
}

impl RiverSiliconCoverage {
    pub fn buckets() -> usize {
        1024
    }

    pub fn from_pc(pc: u64) -> Self {
        let mut bits = vec![0u8; Self::buckets()];
        let idx = ((pc >> 2) as usize) & (Self::buckets() * 8 - 1);
        let byte = idx / 8;
        let bit = (idx % 8) as u8;
        bits[byte] |= 1 << bit;
        Self { bits }
    }
}

impl CoverageSource for RiverSiliconCoverage {
    fn snapshot(&self) -> Vec<u8> {
        self.bits.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_pc_sets_one_bit() {
        let cov = RiverSiliconCoverage::from_pc(0x8000_0010);
        let total = cov
            .bits
            .iter()
            .map(|b| b.count_ones() as usize)
            .sum::<usize>();
        assert_eq!(total, 1);
    }

    #[test]
    fn different_pcs_set_different_bits() {
        let a = RiverSiliconCoverage::from_pc(0x8000_0010);
        let b = RiverSiliconCoverage::from_pc(0x8000_0020);
        assert_ne!(a.bits, b.bits);
    }
}
