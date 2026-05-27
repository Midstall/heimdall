//! Fuzzer generators: produce randomized inputs by target family.

pub mod raw_asm;

#[cfg(feature = "aegis")]
pub mod bitstream;

#[cfg(feature = "cranelift")]
pub mod cranelift_gen;

pub use raw_asm::{RawAsmGen, Rv64};

#[cfg(feature = "aegis")]
pub use bitstream::BitstreamGen;

#[cfg(feature = "cranelift")]
pub use cranelift_gen::CraneliftGen;
