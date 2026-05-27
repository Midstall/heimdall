//! Per-ISA raw instruction generator. RV64 today; aarch64/x86 are follow-ups.

use std::marker::PhantomData;

use heimdall_core::{Artifact, ArtifactKind, DutKind, SeedId};
use rand::{Rng, RngCore};

use crate::traits::Generator;

/// Marker types for the supported ISAs.
pub trait IsaTag: Send + Sync + 'static {
    fn target_dut() -> DutKind;
    fn name() -> &'static str;
}

#[derive(Debug, Clone, Copy)]
pub struct Rv64;

impl IsaTag for Rv64 {
    fn target_dut() -> DutKind {
        DutKind::RiverRc1Nano
    }
    fn name() -> &'static str {
        "raw-asm-rv64"
    }
}

/// Generates a fixed-length sequence of legal RV64 instructions drawn from a
/// small subset. Output: ArtifactKind::RawBytes, length = 4 * instructions.
pub struct RawAsmGen<I: IsaTag> {
    pub instructions: usize,
    _isa: PhantomData<I>,
}

impl<I: IsaTag> RawAsmGen<I> {
    pub fn new(instructions: usize) -> Self {
        Self {
            instructions,
            _isa: PhantomData,
        }
    }
}

impl<I: IsaTag> Generator for RawAsmGen<I> {
    fn target(&self) -> DutKind {
        I::target_dut()
    }

    fn name(&self) -> &str {
        I::name()
    }

    fn generate(&mut self, rng: &mut dyn RngCore, _seed: SeedId) -> Artifact {
        let mut bytes = Vec::with_capacity(self.instructions * 4);
        for _ in 0..self.instructions {
            let insn = random_rv64_insn(rng);
            bytes.extend_from_slice(&insn.to_le_bytes());
        }
        Artifact::new(ArtifactKind::RawBytes, bytes)
    }
}

#[derive(Debug, Clone, Copy)]
enum Class {
    Lui,
    Addi,
    Ori,
    Xori,
    Add,
    Or,
    Xor,
}

fn random_rv64_insn(rng: &mut dyn RngCore) -> u32 {
    let class = match rng.r#gen::<u8>() % 7 {
        0 => Class::Lui,
        1 => Class::Addi,
        2 => Class::Ori,
        3 => Class::Xori,
        4 => Class::Add,
        5 => Class::Or,
        _ => Class::Xor,
    };
    let rd: u32 = rng.gen_range(0..32);
    let rs1: u32 = rng.gen_range(0..32);
    let rs2: u32 = rng.gen_range(0..32);
    match class {
        Class::Lui => {
            let imm20: u32 = rng.gen_range(0..(1u32 << 20));
            encode_u(imm20, rd, 0b0110111)
        }
        Class::Addi => encode_i(random_imm12(rng), rs1, 0b000, rd, 0b0010011),
        Class::Ori => encode_i(random_imm12(rng), rs1, 0b110, rd, 0b0010011),
        Class::Xori => encode_i(random_imm12(rng), rs1, 0b100, rd, 0b0010011),
        Class::Add => encode_r(0b0000000, rs2, rs1, 0b000, rd, 0b0110011),
        Class::Or => encode_r(0b0000000, rs2, rs1, 0b110, rd, 0b0110011),
        Class::Xor => encode_r(0b0000000, rs2, rs1, 0b100, rd, 0b0110011),
    }
}

fn random_imm12(rng: &mut dyn RngCore) -> u32 {
    // 12-bit sign-extended immediate, stored in the upper 12 bits of the instruction.
    rng.gen_range(0..(1u32 << 12))
}

fn encode_u(imm20: u32, rd: u32, opcode: u32) -> u32 {
    (imm20 & 0xfffff) << 12 | (rd & 0x1f) << 7 | (opcode & 0x7f)
}

fn encode_i(imm12: u32, rs1: u32, funct3: u32, rd: u32, opcode: u32) -> u32 {
    (imm12 & 0xfff) << 20
        | (rs1 & 0x1f) << 15
        | (funct3 & 0x7) << 12
        | (rd & 0x1f) << 7
        | (opcode & 0x7f)
}

fn encode_r(funct7: u32, rs2: u32, rs1: u32, funct3: u32, rd: u32, opcode: u32) -> u32 {
    (funct7 & 0x7f) << 25
        | (rs2 & 0x1f) << 20
        | (rs1 & 0x1f) << 15
        | (funct3 & 0x7) << 12
        | (rd & 0x1f) << 7
        | (opcode & 0x7f)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand::rngs::StdRng;

    fn rng() -> StdRng {
        StdRng::seed_from_u64(0x1234_5678)
    }

    #[test]
    fn output_length_matches_instructions() {
        let mut g = RawAsmGen::<Rv64>::new(16);
        let a = g.generate(&mut rng(), SeedId(0));
        assert_eq!(a.bytes.len(), 16 * 4);
    }

    #[test]
    fn artifact_kind_is_raw_bytes() {
        let mut g = RawAsmGen::<Rv64>::new(4);
        let a = g.generate(&mut rng(), SeedId(0));
        assert!(matches!(a.kind, ArtifactKind::RawBytes));
    }

    #[test]
    fn every_instruction_has_known_opcode() {
        let mut g = RawAsmGen::<Rv64>::new(64);
        let a = g.generate(&mut rng(), SeedId(0));
        let chunks: Vec<u32> = a
            .bytes
            .chunks_exact(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        for insn in chunks {
            let opcode = insn & 0x7f;
            assert!(
                opcode == 0b0110111 || opcode == 0b0010011 || opcode == 0b0110011,
                "unexpected opcode 0b{opcode:07b}"
            );
        }
    }

    #[test]
    fn deterministic_for_same_seed() {
        let mut g1 = RawAsmGen::<Rv64>::new(8);
        let mut g2 = RawAsmGen::<Rv64>::new(8);
        let a = g1.generate(&mut StdRng::seed_from_u64(42), SeedId(0));
        let b = g2.generate(&mut StdRng::seed_from_u64(42), SeedId(0));
        assert_eq!(a.bytes, b.bytes);
    }

    #[test]
    fn target_dut_is_river_nano() {
        let g = RawAsmGen::<Rv64>::new(1);
        assert_eq!(g.target(), DutKind::RiverRc1Nano);
        assert_eq!(g.name(), "raw-asm-rv64");
    }
}
