//! Fuzzer generators: produce randomized inputs by target family.

pub mod raw_asm;

#[cfg(feature = "aegis")]
pub mod bitstream;

#[cfg(feature = "cranelift")]
pub mod cranelift_gen;

pub use raw_asm::{
    EBREAK_INSN, IsaStringError, IsaTag, ParsedIsa, RawAsmGen, Rv32, Rv64, RvExtension,
    parse_isa_string,
};

#[cfg(feature = "aegis")]
pub use bitstream::BitstreamGen;

#[cfg(feature = "cranelift")]
pub use cranelift_gen::{CraneliftGen, CraneliftInitError};
