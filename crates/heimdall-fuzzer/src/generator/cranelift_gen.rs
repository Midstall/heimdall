//! Cranelift-backed structured RV64 code generator.
//!
//! Builds a small randomized function in Cranelift IR (i64 params; N random
//! binary ops; return) and lowers it to RV64 machine code via
//! `cranelift-codegen`. The output is raw RV64 instruction bytes (no ELF
//! wrapping); consumers wrap or load as needed.
//!
//! Multi-ISA support: bound to riscv64-unknown-elf today; aarch64/x86_64
//! become trivial to add when a TestDriver for that silicon arrives.

use std::marker::PhantomData;

use cranelift_codegen::Context;
use cranelift_codegen::control::ControlPlane;
use cranelift_codegen::ir::{AbiParam, Function, InstBuilder, Signature, UserFuncName, types};
use cranelift_codegen::isa::{OwnedTargetIsa, lookup};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use heimdall_core::{Artifact, ArtifactKind, DutKind, SeedId};
use rand::{Rng, RngCore};
use target_lexicon::Triple;

use crate::generator::raw_asm::{IsaTag, Rv64, ZERO_INIT_PROLOGUE_LEN, addi_xn_zero};
use crate::traits::Generator;

/// Random binary i64 operation classes Cranelift IR can emit.
#[derive(Debug, Clone, Copy)]
enum BinOp {
    Iadd,
    Isub,
    Imul,
    Band,
    Bor,
    Bxor,
}

fn rand_op(rng: &mut dyn RngCore) -> BinOp {
    match rng.r#gen::<u8>() % 6 {
        0 => BinOp::Iadd,
        1 => BinOp::Isub,
        2 => BinOp::Imul,
        3 => BinOp::Band,
        4 => BinOp::Bor,
        _ => BinOp::Bxor,
    }
}

/// Cranelift-backed structured generator. Bounded by IsaTag for the
/// target_dut family selection; v1 only supports Rv64.
pub struct CraneliftGen<I: IsaTag> {
    /// Number of operations in the generated function. The IR includes one
    /// instruction per op plus the prologue/epilogue.
    pub ops_per_function: usize,
    /// When true (default), prepend the same 31-insn `addi xN, x0, 0`
    /// prologue RawAsmGen uses so differential fuzz against spike sees
    /// the same all-zero GPR entry state on both sides regardless of
    /// the bootrom's seeded values.
    pub zero_init_prologue: bool,
    isa: OwnedTargetIsa,
    _isa: PhantomData<I>,
}

/// Concrete error returned by [`CraneliftGen::rv64`]. Each variant
/// pinpoints which step of the Cranelift backend initialisation
/// failed so callers can log a structured error instead of a flat
/// string.
#[derive(Debug, thiserror::Error)]
pub enum CraneliftInitError {
    #[error("cranelift setting `{key}`: {detail}")]
    Setting { key: &'static str, detail: String },
    #[error("triple parse: {0}")]
    Triple(String),
    #[error("isa lookup: {0}")]
    IsaLookup(String),
    #[error("isa finish: {0}")]
    IsaFinish(String),
}

impl CraneliftGen<Rv64> {
    /// Construct a generator targeting riscv64-unknown-elf with the default
    /// cranelift backend settings (no optimizations beyond defaults).
    pub fn rv64() -> Result<Self, CraneliftInitError> {
        let mut flag_builder = settings::builder();
        flag_builder
            .set("opt_level", "speed")
            .map_err(|e| CraneliftInitError::Setting {
                key: "opt_level",
                detail: e.to_string(),
            })?;
        flag_builder
            .set("is_pic", "false")
            .map_err(|e| CraneliftInitError::Setting {
                key: "is_pic",
                detail: e.to_string(),
            })?;
        let flags = settings::Flags::new(flag_builder);

        let triple: Triple = "riscv64-unknown-elf"
            .parse()
            .map_err(|e: target_lexicon::ParseError| CraneliftInitError::Triple(e.to_string()))?;
        let isa_builder =
            lookup(triple).map_err(|e| CraneliftInitError::IsaLookup(e.to_string()))?;
        let isa = isa_builder
            .finish(flags)
            .map_err(|e| CraneliftInitError::IsaFinish(e.to_string()))?;

        Ok(Self {
            ops_per_function: 8,
            zero_init_prologue: true,
            isa,
            _isa: PhantomData,
        })
    }

    pub fn with_ops_per_function(mut self, n: usize) -> Self {
        self.ops_per_function = n;
        self
    }

    /// Toggle the zero-init prologue. Off when the caller wants
    /// bootloader-seeded GPR state to flow into the body unchanged.
    pub fn with_zero_init_prologue(mut self, on: bool) -> Self {
        self.zero_init_prologue = on;
        self
    }
}

impl Generator for CraneliftGen<Rv64> {
    fn target(&self) -> DutKind {
        Rv64::target_dut()
    }

    fn name(&self) -> &str {
        "cranelift-rv64"
    }

    fn generate(&mut self, rng: &mut dyn RngCore, _seed: SeedId) -> Artifact {
        // Function signature: fn(i64) -> i64
        let mut sig = Signature::new(self.isa.default_call_conv());
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        let mut func = Function::with_name_signature(UserFuncName::user(0, 0), sig);

        let mut fbctx = FunctionBuilderContext::new();
        {
            let mut builder = FunctionBuilder::new(&mut func, &mut fbctx);
            let entry = builder.create_block();
            builder.append_block_params_for_function_params(entry);
            builder.switch_to_block(entry);
            builder.seal_block(entry);

            let mut values: Vec<cranelift_codegen::ir::Value> =
                vec![builder.block_params(entry)[0]];

            for _ in 0..self.ops_per_function {
                // Pick op and two source values from `values`.
                let op = rand_op(rng);
                let a_idx = rng.gen_range(0..values.len());
                let b_idx = rng.gen_range(0..values.len());
                let a = values[a_idx];
                let b = values[b_idx];
                let v = match op {
                    BinOp::Iadd => builder.ins().iadd(a, b),
                    BinOp::Isub => builder.ins().isub(a, b),
                    BinOp::Imul => builder.ins().imul(a, b),
                    BinOp::Band => builder.ins().band(a, b),
                    BinOp::Bor => builder.ins().bor(a, b),
                    BinOp::Bxor => builder.ins().bxor(a, b),
                };
                values.push(v);
            }

            let last = *values.last().expect("at least one value");
            builder.ins().return_(&[last]);
            builder.finalize();
        }

        // Compile to machine code.
        let mut ctx = Context::for_function(func);
        let mut ctrl_plane = ControlPlane::default();
        let compiled = ctx
            .compile(&*self.isa, &mut ctrl_plane)
            .expect("cranelift compile");
        // `code_buffer()` gives the raw instruction bytes.
        let body = compiled.code_buffer();

        let prologue_len = if self.zero_init_prologue {
            ZERO_INIT_PROLOGUE_LEN
        } else {
            0
        };
        let mut bytes = Vec::with_capacity(prologue_len * 4 + body.len());
        if self.zero_init_prologue {
            for n in 1..32u32 {
                bytes.extend_from_slice(&addi_xn_zero(n).to_le_bytes());
            }
        }
        bytes.extend_from_slice(body);

        Artifact::new(ArtifactKind::RawBytes, bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand::rngs::StdRng;

    #[test]
    fn output_nonempty_and_rv64_aligned() {
        let mut g = CraneliftGen::rv64().expect("rv64");
        let mut rng = StdRng::seed_from_u64(0xdeadbeef);
        let a = g.generate(&mut rng, SeedId(0));
        assert!(a.bytes.len() > 0, "empty output");
        assert_eq!(a.bytes.len() % 4, 0, "expected RV64-aligned length");
    }

    #[test]
    fn deterministic_for_same_seed() {
        let mut g1 = CraneliftGen::rv64().expect("rv64");
        let mut g2 = CraneliftGen::rv64().expect("rv64");
        let a = g1.generate(&mut StdRng::seed_from_u64(7), SeedId(0));
        let b = g2.generate(&mut StdRng::seed_from_u64(7), SeedId(0));
        assert_eq!(a.bytes, b.bytes);
    }

    #[test]
    fn target_dut_is_river_nano() {
        let g = CraneliftGen::rv64().expect("rv64");
        assert_eq!(g.target(), DutKind::RiverRc1Nano);
        assert_eq!(g.name(), "cranelift-rv64");
    }

    #[test]
    fn longer_function_produces_more_bytes() {
        // Seed 3 is known to produce 8 bytes for 2 ops and 64 bytes for 32 ops
        // under cranelift opt_level=speed. Seed 1 hits a degenerate optimizer
        // path where all operations fold to the same value.
        let mut g_short = CraneliftGen::rv64().expect("rv64").with_ops_per_function(2);
        let mut g_long = CraneliftGen::rv64()
            .expect("rv64")
            .with_ops_per_function(32);
        let a = g_short.generate(&mut StdRng::seed_from_u64(3), SeedId(0));
        let b = g_long.generate(&mut StdRng::seed_from_u64(3), SeedId(0));
        assert!(
            b.bytes.len() > a.bytes.len(),
            "{} should be > {}",
            b.bytes.len(),
            a.bytes.len()
        );
    }
}
