//! Per-ISA raw instruction generator. RV32 + RV64 today. aarch64/x86 are
//! follow-ups when those DUT families come online.

use std::collections::HashSet;
use std::marker::PhantomData;

use heimdall_core::{Artifact, ArtifactKind, DutKind, SeedId};
use rand::{Rng, RngCore};
use serde::{Deserialize, Serialize};

use crate::traits::Generator;

/// Marker types for the supported ISAs. `xlen()` returns the
/// architectural register width in bits so encoders that depend on
/// it (shift shamt width: 5-bit on RV32, 6-bit on RV64) can pick the
/// right encoding at compile time without runtime branching.
pub trait IsaTag: Send + Sync + 'static {
    fn target_dut() -> DutKind;
    fn name() -> &'static str;
    fn xlen() -> u8;
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
    fn xlen() -> u8 {
        64
    }
}

/// RV32 marker. No concrete DUT in the registry today. The target_dut
/// returns the same RiverRc1Nano variant as RV64 since `DutKind` is
/// XLEN-agnostic. Future commits add a real RV32 DutKind variant when a
/// RV32 DUT shows up.
#[derive(Debug, Clone, Copy)]
pub struct Rv32;

impl IsaTag for Rv32 {
    fn target_dut() -> DutKind {
        DutKind::RiverRc1Nano
    }
    fn name() -> &'static str {
        "raw-asm-rv32"
    }
    fn xlen() -> u8 {
        32
    }
}

/// Subset of the RISC-V ISA the generator may emit. Names match the
/// canonical extension letters / Zxxx identifiers, mirroring what
/// Harbor's `lib/src/riscv/extensions/` exposes. The full set is
/// listed for forward compat: today only `I` and `M` actually have
/// instruction encodings in [`Class::ALL`], but downstream config
/// (TOML / `misa` probe) can carry the others as advisory information
/// and future PRs add the encodings without changing the config
/// surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RvExtension {
    /// Base integer. Always implicitly enabled.
    I,
    /// Multiply / divide. Fault-free, register-only.
    M,
    /// Atomics. Memory-bearing, so requires bounded address logic before
    /// the fuzzer can emit safely.
    A,
    /// Single-precision FP.
    F,
    /// Double-precision FP.
    D,
    /// Compressed (16-bit) instructions.
    C,
    /// Bitmanip umbrella (Zba/Zbb/Zbs/Zbc per the B-extension spec).
    B,
    /// Vector.
    V,
    /// Hypervisor.
    H,
    /// Conditional move.
    Zicond,
    /// CSR access.
    Zicsr,
    /// Instruction fence.
    Zifencei,
    /// Half-precision FP minimum.
    Zfhmin,
}

impl RvExtension {
    /// Letter on the misa CSR for base I/M/A/F/D/C/B/V/H, or None for
    /// Zxxx-style extensions which don't appear in misa.
    pub fn misa_letter(self) -> Option<char> {
        match self {
            RvExtension::I => Some('i'),
            RvExtension::M => Some('m'),
            RvExtension::A => Some('a'),
            RvExtension::F => Some('f'),
            RvExtension::D => Some('d'),
            RvExtension::C => Some('c'),
            RvExtension::B => Some('b'),
            RvExtension::V => Some('v'),
            RvExtension::H => Some('h'),
            RvExtension::Zicond
            | RvExtension::Zicsr
            | RvExtension::Zifencei
            | RvExtension::Zfhmin => None,
        }
    }
}

/// Parse an extension from its misa letter. Lowercase / uppercase
/// both accepted. `'g'` is rejected here. It's an umbrella alias
/// (I+M+A+F+D) and only meaningful inside the `rvNN<letters>` parser,
/// not as a single-extension identifier.
impl TryFrom<char> for RvExtension {
    type Error = IsaStringError;
    fn try_from(c: char) -> Result<Self, Self::Error> {
        match c.to_ascii_lowercase() {
            'i' => Ok(RvExtension::I),
            'm' => Ok(RvExtension::M),
            'a' => Ok(RvExtension::A),
            'f' => Ok(RvExtension::F),
            'd' => Ok(RvExtension::D),
            'c' => Ok(RvExtension::C),
            'b' => Ok(RvExtension::B),
            'v' => Ok(RvExtension::V),
            'h' => Ok(RvExtension::H),
            _ => Err(IsaStringError::UnknownLetter(c)),
        }
    }
}

/// Parse an extension from either a single misa letter (`"i"`) or a
/// Z-style identifier (`"zicsr"`, `"zifencei"`, ...). The TOML
/// `[dut.isa] extensions = [...]` list mixes both forms. This impl
/// is the single normalisation point.
impl std::str::FromStr for RvExtension {
    type Err = IsaStringError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let lower = s.to_ascii_lowercase();
        match lower.as_str() {
            "zicond" => Ok(RvExtension::Zicond),
            "zicsr" => Ok(RvExtension::Zicsr),
            "zifencei" => Ok(RvExtension::Zifencei),
            "zfhmin" => Ok(RvExtension::Zfhmin),
            other if other.len() == 1 => RvExtension::try_from(other.chars().next().unwrap()),
            _ => Err(IsaStringError::UnknownZName(s.to_string())),
        }
    }
}

/// Error from parsing an ISA string. Variants distinguish the three
/// failure shapes operators and probe-decoders care about: the
/// `rvNN` prefix is missing, the XLEN field isn't 32/64, a misa
/// letter doesn't resolve to a known extension, or a Z-style chunk
/// after an underscore doesn't match a known name.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IsaStringError {
    #[error("isa string must start with `rv32` or `rv64`")]
    MissingRvPrefix,
    #[error("isa xlen must be 32 or 64")]
    BadXlen,
    #[error("unknown isa extension letter `{0}`")]
    UnknownLetter(char),
    #[error("unknown Z-extension `{0}`")]
    UnknownZName(String),
}

/// Parsed form of a canonical ISA string. `xlen` is 32 or 64. The
/// `extensions` set always contains [`RvExtension::I`] (the base
/// integer extension is implicit).
///
/// Construct via [`std::str::FromStr`]:
/// `let isa: ParsedIsa = "rv64imac_zicsr".parse()?;`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedIsa {
    pub xlen: u8,
    pub extensions: HashSet<RvExtension>,
}

impl std::str::FromStr for ParsedIsa {
    type Err = IsaStringError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let lower = s.to_ascii_lowercase();
        let rest = lower
            .strip_prefix("rv")
            .ok_or(IsaStringError::MissingRvPrefix)?;
        let (xlen, body) = if let Some(rest) = rest.strip_prefix("32") {
            (32u8, rest)
        } else if let Some(rest) = rest.strip_prefix("64") {
            (64u8, rest)
        } else {
            return Err(IsaStringError::BadXlen);
        };

        let mut extensions: HashSet<RvExtension> = HashSet::new();
        extensions.insert(RvExtension::I);

        // Split on `_` so `"rv64imac_zicsr_zifencei"` parses cleanly:
        // the first chunk is letter-encoded, subsequent chunks are
        // Z-style.
        let mut parts = body.split('_');
        if let Some(letters) = parts.next() {
            for c in letters.chars() {
                // `g` is the umbrella shorthand = I+M+A+F+D.
                if c == 'g' {
                    extensions.extend([
                        RvExtension::M,
                        RvExtension::A,
                        RvExtension::F,
                        RvExtension::D,
                    ]);
                    continue;
                }
                extensions.insert(RvExtension::try_from(c)?);
            }
        }
        for part in parts {
            extensions.insert(part.parse::<RvExtension>()?);
        }
        Ok(ParsedIsa { xlen, extensions })
    }
}

/// Build a [`ParsedIsa`] from a TOML-style `(xlen, ["i", "m", ...])`
/// pair. Unlike the [`std::str::FromStr`] path, unknown extension
/// strings are silently dropped here. The per-DUT config layer is
/// forward-compat and must keep working when a future heimdall.toml
/// names an extension this version of the fuzzer doesn't recognise.
/// [`RvExtension::I`] is always added.
impl<'a, I> From<(u8, I)> for ParsedIsa
where
    I: IntoIterator<Item = &'a str>,
{
    fn from((xlen, names): (u8, I)) -> Self {
        let mut extensions: HashSet<RvExtension> = HashSet::new();
        extensions.insert(RvExtension::I);
        for n in names {
            if let Ok(ext) = n.parse::<RvExtension>() {
                extensions.insert(ext);
            }
        }
        ParsedIsa { xlen, extensions }
    }
}

/// Thin shim around `"...".parse::<ParsedIsa>()` that yields the
/// `(xlen, extensions)` tuple older callers expected. Prefer
/// `ParsedIsa::from_str` in new code.
pub fn parse_isa_string(s: &str) -> Result<(u8, HashSet<RvExtension>), IsaStringError> {
    let ParsedIsa { xlen, extensions } = s.parse()?;
    Ok((xlen, extensions))
}

/// Generates a fixed-length sequence of legal RV64 instructions drawn from a
/// small register-to-register ALU subset (LUI/OP-IMM/OP), optionally
/// preceded by a register-zeroing prologue and terminated by `ebreak` so
/// the program has a deterministic architectural stop point.
///
/// Output: `ArtifactKind::RawBytes`, length = `4 * (prologue? + instructions + ebreak?)`.
/// With both defaults on (the typical fuzz path) that's
/// `4 * (31 + instructions + 1)` bytes.
///
/// Why the prologue default is on: spike's bootrom seeds `t0` to the
/// program entry and `a1` to the DTB pointer before jumping to user
/// code. Real silicon / the River emulator enter user code with all-zero
/// GPRs. Differential fuzz needs both sides to enter the body in
/// identical architectural state, otherwise any body insn that reads
/// `t0` or `a1` cascades the initial-state mismatch into the GPRs the
/// diff inspects. Spike's interactive `reg <core> <reg> <value>` debug
/// command is read-only, so we can't fix this from the spike harness
/// side. Baking 31 `addi xN, x0, 0` instructions into the program
/// itself runs symmetrically on both models. Programs that genuinely
/// want non-zero initial state (rare in fuzz, useful for directed
/// regression vectors) can disable via [`Self::with_zero_init_prologue`].
///
/// Why the ebreak default is on: when used as a differential fuzz input
/// against a golden ISS (e.g. Spike), DUT and golden need to halt at the
/// same instruction or GPR comparison turns into pure trap-handling
/// alignment noise. The DUT is force-halted by a wait_halt budget. Spike
/// stops at exactly `r <N>`. They never agree without an explicit halt
/// marker in the program. A trailing `ebreak` flips both into a halted
/// state at the same retired-instruction count, so any remaining GPR
/// divergence is genuinely architectural. Crash/liveness fuzzing that
/// doesn't compare against a golden can disable the terminator via
/// [`Self::with_ebreak_terminator`].
pub struct RawAsmGen<I: IsaTag> {
    pub instructions: usize,
    /// When true (default), prepend 31 `addi xN, x0, 0` instructions
    /// (N=1..31) to neutralise any bootloader-seeded GPR state. Aligns
    /// spike's bootrom-stamped initial state with the DUT's all-zero
    /// reset.
    pub zero_init_prologue: bool,
    /// When true (default), append an `ebreak` (0x00100073) after the
    /// random body. Aligns spike and DUT halts for differential fuzz.
    pub ebreak_terminator: bool,
    /// Extensions the generator may draw instructions from.
    /// `RvExtension::I` is always implicitly included (the base
    /// integer encoder is the only one that can produce arithmetic
    /// without depending on M/F/A/etc.). Defaults to `{I, M}` so the
    /// generator behaves as it always has when callers don't supply
    /// a configuration.
    pub extensions: HashSet<RvExtension>,
    _isa: PhantomData<I>,
}

impl<I: IsaTag> RawAsmGen<I> {
    pub fn new(instructions: usize) -> Self {
        let mut extensions = HashSet::new();
        extensions.insert(RvExtension::I);
        extensions.insert(RvExtension::M);
        Self {
            instructions,
            zero_init_prologue: true,
            ebreak_terminator: true,
            extensions,
            _isa: PhantomData,
        }
    }

    /// Toggle the zero-init prologue. Off when the caller wants
    /// bootloader-seeded GPR state to flow into the body unchanged.
    pub fn with_zero_init_prologue(mut self, on: bool) -> Self {
        self.zero_init_prologue = on;
        self
    }

    /// Toggle the trailing ebreak. Off for raw crash/liveness fuzzing
    /// where halting policy is owned by the driver's wait_halt budget.
    /// On (default) for differential fuzz against a golden.
    pub fn with_ebreak_terminator(mut self, on: bool) -> Self {
        self.ebreak_terminator = on;
        self
    }

    /// Replace the enabled-extension set. The `I` extension is always
    /// added back unconditionally. Without it the random pool would
    /// be empty and `generate` would panic. Extensions that don't yet
    /// have an instruction encoding in [`Class::ALL`] (everything
    /// other than `I` and `M` today) are accepted and stored, but
    /// silently contribute nothing to the random pool until follow-up
    /// PRs add their classes.
    pub fn with_extensions(mut self, exts: impl IntoIterator<Item = RvExtension>) -> Self {
        let mut set: HashSet<RvExtension> = exts.into_iter().collect();
        set.insert(RvExtension::I);
        self.extensions = set;
        self
    }
}

/// RV64 EBREAK encoding (SYSTEM opcode, funct12=1).
pub const EBREAK_INSN: u32 = 0x0010_0073;

/// Encode `addi xN, x0, 0` to write 0 into GPR `n`. ADDI is OP-IMM with
/// funct3=0, immediate=0, rs1=x0.
pub const fn addi_xn_zero(n: u32) -> u32 {
    ((n & 0x1f) << 7) | 0b0010011
}

/// Number of registers the zero-init prologue clears: x1..x31. x0 is
/// hardwired to zero so we skip it.
pub const ZERO_INIT_PROLOGUE_LEN: usize = 31;

impl<I: IsaTag> Generator for RawAsmGen<I> {
    fn target(&self) -> DutKind {
        I::target_dut()
    }

    fn name(&self) -> &str {
        I::name()
    }

    fn generate(&mut self, rng: &mut dyn RngCore, _seed: SeedId) -> Artifact {
        let prologue = if self.zero_init_prologue {
            ZERO_INIT_PROLOGUE_LEN
        } else {
            0
        };
        let extra = usize::from(self.ebreak_terminator);
        let mut bytes = Vec::with_capacity((prologue + self.instructions + extra) * 4);
        if self.zero_init_prologue {
            for n in 1..32u32 {
                bytes.extend_from_slice(&addi_xn_zero(n).to_le_bytes());
            }
        }
        // Pre-filter the class pool once per generate() call. The
        // empty pool would mean only I-classless extensions were
        // enabled, which contradicts with_extensions()'s invariant
        // that I is always present. Assert it loudly.
        let pool: Vec<Class> = Class::ALL
            .iter()
            .copied()
            .filter(|c| self.extensions.contains(&c.extension()))
            .collect();
        assert!(
            !pool.is_empty(),
            "raw-asm generator class pool is empty (enabled exts: {:?})",
            self.extensions,
        );
        for _ in 0..self.instructions {
            let insn = random_insn(rng, &pool, I::xlen());
            bytes.extend_from_slice(&insn.to_le_bytes());
        }
        if self.ebreak_terminator {
            bytes.extend_from_slice(&EBREAK_INSN.to_le_bytes());
        }
        Artifact::new(ArtifactKind::RawBytes, bytes)
    }
}

/// Instruction classes the fuzzer emits. All are register-only or
/// register-immediate (no memory, no branches, no system) so every
/// encoding is fault-free regardless of register contents. The pool
/// covers RV64I integer + shift forms plus the M-extension's
/// mul/div/rem family. That's enough surface for differential fuzz
/// against spike to cover the entire arithmetic ISA without needing
/// address-bounding logic for loads/stores.
///
/// When adding a class here, increment `Class::COUNT` and the
/// `random_rv64_insn` match arm pool, and (if the class can produce a
/// fault) consider whether it belongs in the safe set at all.
#[derive(Debug, Clone, Copy)]
enum Class {
    // U-format
    Lui,
    // I-format ALU (rd = rs1 op imm12)
    Addi,
    Andi,
    Ori,
    Xori,
    Slti,
    Sltiu,
    // I-format shift (rd = rs1 << / >> shamt6). RV64 uses a 6-bit
    // shamt. The funct7-equivalent top bit must be 0 for SLLI/SRLI
    // and 0x20 (logical 0x10 in funct6 view) for SRAI.
    Slli,
    Srli,
    Srai,
    // R-format ALU (rd = rs1 op rs2)
    Add,
    Sub,
    And,
    Or,
    Xor,
    Slt,
    Sltu,
    Sll,
    Srl,
    Sra,
    // R-format M-extension (rd = rs1 op rs2). Div/rem by zero is
    // architecturally defined (returns -1 / dividend respectively),
    // so these are still fault-free.
    Mul,
    Mulh,
    Div,
    Divu,
    Rem,
    Remu,
}

impl Class {
    /// The RISC-V extension this class belongs to. The random picker
    /// filters [`Class::ALL`] by the generator's enabled extension set
    /// to honour the per-DUT ISA configuration.
    pub fn extension(self) -> RvExtension {
        match self {
            Class::Lui
            | Class::Addi
            | Class::Andi
            | Class::Ori
            | Class::Xori
            | Class::Slti
            | Class::Sltiu
            | Class::Slli
            | Class::Srli
            | Class::Srai
            | Class::Add
            | Class::Sub
            | Class::And
            | Class::Or
            | Class::Xor
            | Class::Slt
            | Class::Sltu
            | Class::Sll
            | Class::Srl
            | Class::Sra => RvExtension::I,
            Class::Mul | Class::Mulh | Class::Div | Class::Divu | Class::Rem | Class::Remu => {
                RvExtension::M
            }
        }
    }

    /// All variants in a fixed order, used by `random_rv64_insn` to
    /// pick uniformly without enumerating each match arm twice.
    const ALL: [Class; 26] = [
        Class::Lui,
        Class::Addi,
        Class::Andi,
        Class::Ori,
        Class::Xori,
        Class::Slti,
        Class::Sltiu,
        Class::Slli,
        Class::Srli,
        Class::Srai,
        Class::Add,
        Class::Sub,
        Class::And,
        Class::Or,
        Class::Xor,
        Class::Slt,
        Class::Sltu,
        Class::Sll,
        Class::Srl,
        Class::Sra,
        Class::Mul,
        Class::Mulh,
        Class::Div,
        Class::Divu,
        Class::Rem,
        Class::Remu,
    ];
}

const OPCODE_LUI: u32 = 0b0110111;
const OPCODE_OP_IMM: u32 = 0b0010011;
const OPCODE_OP: u32 = 0b0110011;

/// Shift-immediate encoder. The shamt field is XLEN-aware: RV32 uses
/// a 5-bit shamt (top 7 bits of imm12 encode the funct: 0x00 / 0x20),
/// RV64 widens it to 6 bits (top 6 bits encode the funct: 0x00 /
/// 0x10 viewed as 6-bit). Both end up at the same RISC-V word
/// encoding pattern. We just shift the funct mask by the right
/// number of bits.
fn encode_shift_i(shamt: u32, rs1: u32, funct3: u32, rd: u32, srai: bool, xlen: u8) -> u32 {
    let (shamt_bits, top_mask) = if xlen == 32 {
        (5u32, if srai { 0b0100000 } else { 0u32 } << 5)
    } else {
        // RV64
        (6u32, if srai { 0b010000 } else { 0u32 } << 6)
    };
    let mask = (1u32 << shamt_bits) - 1;
    let imm = top_mask | (shamt & mask);
    encode_i(imm, rs1, funct3, rd, OPCODE_OP_IMM)
}

fn random_shamt(rng: &mut dyn RngCore, xlen: u8) -> u32 {
    let max = if xlen == 32 { 32 } else { 64 };
    rng.gen_range(0..max)
}

fn random_insn(rng: &mut dyn RngCore, pool: &[Class], xlen: u8) -> u32 {
    let class = pool[rng.gen_range(0..pool.len())];
    let rd: u32 = rng.gen_range(0..32);
    let rs1: u32 = rng.gen_range(0..32);
    let rs2: u32 = rng.gen_range(0..32);
    match class {
        Class::Lui => {
            let imm20: u32 = rng.gen_range(0..(1u32 << 20));
            encode_u(imm20, rd, OPCODE_LUI)
        }
        // OP-IMM ALU
        Class::Addi => encode_i(random_imm12(rng), rs1, 0b000, rd, OPCODE_OP_IMM),
        Class::Andi => encode_i(random_imm12(rng), rs1, 0b111, rd, OPCODE_OP_IMM),
        Class::Ori => encode_i(random_imm12(rng), rs1, 0b110, rd, OPCODE_OP_IMM),
        Class::Xori => encode_i(random_imm12(rng), rs1, 0b100, rd, OPCODE_OP_IMM),
        Class::Slti => encode_i(random_imm12(rng), rs1, 0b010, rd, OPCODE_OP_IMM),
        Class::Sltiu => encode_i(random_imm12(rng), rs1, 0b011, rd, OPCODE_OP_IMM),
        // OP-IMM shift
        Class::Slli => encode_shift_i(random_shamt(rng, xlen), rs1, 0b001, rd, false, xlen),
        Class::Srli => encode_shift_i(random_shamt(rng, xlen), rs1, 0b101, rd, false, xlen),
        Class::Srai => encode_shift_i(random_shamt(rng, xlen), rs1, 0b101, rd, true, xlen),
        // OP base
        Class::Add => encode_r(0b0000000, rs2, rs1, 0b000, rd, OPCODE_OP),
        Class::Sub => encode_r(0b0100000, rs2, rs1, 0b000, rd, OPCODE_OP),
        Class::And => encode_r(0b0000000, rs2, rs1, 0b111, rd, OPCODE_OP),
        Class::Or => encode_r(0b0000000, rs2, rs1, 0b110, rd, OPCODE_OP),
        Class::Xor => encode_r(0b0000000, rs2, rs1, 0b100, rd, OPCODE_OP),
        Class::Slt => encode_r(0b0000000, rs2, rs1, 0b010, rd, OPCODE_OP),
        Class::Sltu => encode_r(0b0000000, rs2, rs1, 0b011, rd, OPCODE_OP),
        Class::Sll => encode_r(0b0000000, rs2, rs1, 0b001, rd, OPCODE_OP),
        Class::Srl => encode_r(0b0000000, rs2, rs1, 0b101, rd, OPCODE_OP),
        Class::Sra => encode_r(0b0100000, rs2, rs1, 0b101, rd, OPCODE_OP),
        // OP M-extension (funct7 = 0x01)
        Class::Mul => encode_r(0b0000001, rs2, rs1, 0b000, rd, OPCODE_OP),
        Class::Mulh => encode_r(0b0000001, rs2, rs1, 0b001, rd, OPCODE_OP),
        Class::Div => encode_r(0b0000001, rs2, rs1, 0b100, rd, OPCODE_OP),
        Class::Divu => encode_r(0b0000001, rs2, rs1, 0b101, rd, OPCODE_OP),
        Class::Rem => encode_r(0b0000001, rs2, rs1, 0b110, rd, OPCODE_OP),
        Class::Remu => encode_r(0b0000001, rs2, rs1, 0b111, rd, OPCODE_OP),
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
    fn default_output_includes_prologue_and_terminator() {
        // Default-on prologue (31) + body (N) + ebreak (1) = (N+32)*4.
        let mut g = RawAsmGen::<Rv64>::new(16);
        let a = g.generate(&mut rng(), SeedId(0));
        assert_eq!(a.bytes.len(), (16 + ZERO_INIT_PROLOGUE_LEN + 1) * 4);
    }

    #[test]
    fn terminator_and_prologue_off_yields_exact_body_length() {
        let mut g = RawAsmGen::<Rv64>::new(16)
            .with_ebreak_terminator(false)
            .with_zero_init_prologue(false);
        let a = g.generate(&mut rng(), SeedId(0));
        assert_eq!(a.bytes.len(), 16 * 4);
    }

    #[test]
    fn trailing_ebreak_is_last_instruction() {
        let mut g = RawAsmGen::<Rv64>::new(8);
        let a = g.generate(&mut rng(), SeedId(0));
        let last = u32::from_le_bytes([
            a.bytes[a.bytes.len() - 4],
            a.bytes[a.bytes.len() - 3],
            a.bytes[a.bytes.len() - 2],
            a.bytes[a.bytes.len() - 1],
        ]);
        assert_eq!(last, EBREAK_INSN, "last word must be ebreak");
    }

    #[test]
    fn prologue_is_31_addi_xn_x0_zero() {
        let mut g = RawAsmGen::<Rv64>::new(0).with_ebreak_terminator(false);
        let a = g.generate(&mut rng(), SeedId(0));
        assert_eq!(a.bytes.len(), ZERO_INIT_PROLOGUE_LEN * 4);
        for n in 1..32u32 {
            let off = ((n - 1) as usize) * 4;
            let w = u32::from_le_bytes([
                a.bytes[off],
                a.bytes[off + 1],
                a.bytes[off + 2],
                a.bytes[off + 3],
            ]);
            assert_eq!(
                w,
                addi_xn_zero(n),
                "prologue insn {n} must be addi xN, x0, 0",
            );
            // Sanity-check the encoding: OP-IMM opcode, funct3=0,
            // rs1=x0, imm=0, rd=N.
            assert_eq!(w & 0x7f, 0b0010011, "opcode");
            assert_eq!((w >> 12) & 0x7, 0, "funct3");
            assert_eq!((w >> 15) & 0x1f, 0, "rs1=x0");
            assert_eq!(w >> 20, 0, "imm=0");
            assert_eq!((w >> 7) & 0x1f, n, "rd=xN");
        }
    }

    #[test]
    fn prologue_off_skips_zeroing_block() {
        let mut g = RawAsmGen::<Rv64>::new(4).with_zero_init_prologue(false);
        let a = g.generate(&mut rng(), SeedId(0));
        // No prologue: just 4 body insns + 1 ebreak.
        assert_eq!(a.bytes.len(), 5 * 4);
    }

    #[test]
    fn artifact_kind_is_raw_bytes() {
        let mut g = RawAsmGen::<Rv64>::new(4);
        let a = g.generate(&mut rng(), SeedId(0));
        assert!(matches!(a.kind, ArtifactKind::RawBytes));
    }

    #[test]
    fn every_body_instruction_has_known_opcode() {
        // Body opcodes must stay within the ALU subset (LUI, OP-IMM,
        // OP). M-extension lives under OP (funct7=0x01). Shifts live
        // under OP-IMM/OP with their own funct3. The trailing ebreak
        // is a SYSTEM opcode and is checked separately.
        let mut g = RawAsmGen::<Rv64>::new(64)
            .with_ebreak_terminator(false)
            .with_zero_init_prologue(false);
        let a = g.generate(&mut rng(), SeedId(0));
        let chunks: Vec<u32> = a
            .bytes
            .chunks_exact(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        for insn in chunks {
            let opcode = insn & 0x7f;
            assert!(
                opcode == OPCODE_LUI || opcode == OPCODE_OP_IMM || opcode == OPCODE_OP,
                "unexpected opcode 0b{opcode:07b}"
            );
        }
    }

    /// The expanded class pool must actually surface every category we
    /// declared in `Class::ALL`. Picking a large body count and a
    /// fixed seed gives near-100% probability of hitting each class.
    /// The test fails if a class drops out of the random pool (e.g.
    /// a future refactor accidentally truncates `Class::ALL`).
    #[test]
    fn class_pool_covers_lui_op_imm_op_and_m_extension() {
        let mut g = RawAsmGen::<Rv64>::new(2000)
            .with_ebreak_terminator(false)
            .with_zero_init_prologue(false);
        let a = g.generate(&mut rng(), SeedId(0));
        let chunks: Vec<u32> = a
            .bytes
            .chunks_exact(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();

        let mut saw_lui = false;
        let mut saw_op_imm_shift = false;
        let mut saw_op_imm_alu = false;
        let mut saw_op_base = false;
        let mut saw_op_sub_or_sra = false;
        let mut saw_m_ext = false;
        for insn in chunks {
            let opcode = insn & 0x7f;
            let funct3 = (insn >> 12) & 0x7;
            let funct7 = (insn >> 25) & 0x7f;
            match opcode {
                OPCODE_LUI => saw_lui = true,
                OPCODE_OP_IMM => {
                    if funct3 == 0b001 || funct3 == 0b101 {
                        saw_op_imm_shift = true;
                    } else {
                        saw_op_imm_alu = true;
                    }
                }
                OPCODE_OP => {
                    if funct7 == 0b0000001 {
                        saw_m_ext = true;
                    } else if funct7 == 0b0100000 {
                        saw_op_sub_or_sra = true;
                    } else {
                        saw_op_base = true;
                    }
                }
                other => panic!("unexpected opcode 0b{other:07b}"),
            }
        }
        assert!(saw_lui, "Lui must appear");
        assert!(
            saw_op_imm_shift,
            "OP-IMM shifts (slli/srli/srai) must appear"
        );
        assert!(saw_op_imm_alu, "OP-IMM ALU (addi/andi/...) must appear");
        assert!(saw_op_base, "OP base (add/and/...) must appear");
        assert!(saw_op_sub_or_sra, "OP funct7=0x20 (sub/sra) must appear",);
        assert!(saw_m_ext, "M-extension (mul/div/rem) must appear");
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

    #[test]
    fn rv32_marker_reports_xlen_32_and_distinct_name() {
        let g = RawAsmGen::<Rv32>::new(1);
        assert_eq!(g.name(), "raw-asm-rv32");
        assert_eq!(Rv32::xlen(), 32);
        assert_eq!(Rv64::xlen(), 64);
    }

    /// RV32 shift-immediate must fit in 5 bits of shamt. The encoder
    /// must NOT generate a 6-bit shamt when xlen=32, since that would
    /// be illegal RV32 encoding (the top bit overlaps with funct7's
    /// arithmetic-vs-logical flag and would corrupt SRAI/SRLI).
    #[test]
    fn rv32_shift_immediate_uses_5bit_shamt() {
        // Force the generator to emit shift-immediates by restricting
        // extensions to just I and using a large body count.
        let mut g = RawAsmGen::<Rv32>::new(500)
            .with_ebreak_terminator(false)
            .with_zero_init_prologue(false);
        let a = g.generate(&mut rng(), SeedId(0));
        let chunks: Vec<u32> = a
            .bytes
            .chunks_exact(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        let mut saw_shift = false;
        for insn in chunks {
            let opcode = insn & 0x7f;
            let funct3 = (insn >> 12) & 0x7;
            if opcode == OPCODE_OP_IMM && (funct3 == 0b001 || funct3 == 0b101) {
                saw_shift = true;
                // The imm12 field is bits 20..32. The top 7 bits of
                // imm12 are bits 25..32 of the instruction. For RV32
                // shifts the shamt sits at bits 20..25 (5 bits) and
                // the funct7 at bits 25..32 must be 0 (SLLI/SRLI) or
                // 0x20 (SRAI). The bit at position 25 (shamt[5])
                // must always be zero.
                let shamt_top_bit = (insn >> 25) & 1;
                assert_eq!(
                    shamt_top_bit, 0,
                    "RV32 shamt[5] must be zero in shift insn 0x{insn:08x}",
                );
            }
        }
        assert!(saw_shift, "test must have observed at least one shift insn");
    }

    #[test]
    fn rv64_shift_immediate_uses_6bit_shamt() {
        // RV64 shamt[5] CAN be 1 (shamts 32..63 are legal). Across a
        // big sample, at least one shift should set that top bit.
        let mut g = RawAsmGen::<Rv64>::new(500)
            .with_ebreak_terminator(false)
            .with_zero_init_prologue(false);
        let a = g.generate(&mut rng(), SeedId(0));
        let chunks: Vec<u32> = a
            .bytes
            .chunks_exact(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        let mut saw_wide_shamt = false;
        for insn in chunks {
            let opcode = insn & 0x7f;
            let funct3 = (insn >> 12) & 0x7;
            if opcode == OPCODE_OP_IMM && (funct3 == 0b001 || funct3 == 0b101) {
                let shamt_top_bit = (insn >> 25) & 1;
                if shamt_top_bit == 1 {
                    saw_wide_shamt = true;
                    break;
                }
            }
        }
        assert!(
            saw_wide_shamt,
            "RV64 random shifts should produce shamts >= 32 (top bit set)",
        );
    }

    /// Extensions filter the class pool: disabling M must drop
    /// mul/div/rem from the random output.
    #[test]
    fn disabling_m_extension_drops_mul_div_rem() {
        let mut g = RawAsmGen::<Rv64>::new(500)
            .with_ebreak_terminator(false)
            .with_zero_init_prologue(false)
            .with_extensions([RvExtension::I]);
        let a = g.generate(&mut rng(), SeedId(0));
        let chunks: Vec<u32> = a
            .bytes
            .chunks_exact(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        for insn in chunks {
            let opcode = insn & 0x7f;
            let funct7 = (insn >> 25) & 0x7f;
            assert_ne!(
                (opcode, funct7),
                (OPCODE_OP, 0b0000001),
                "M-extension insn leaked into I-only pool: 0x{insn:08x}",
            );
        }
    }

    #[test]
    fn isa_string_parser_basic_rv64imac() {
        let (xlen, exts) = parse_isa_string("rv64imac").expect("parse");
        assert_eq!(xlen, 64);
        assert!(exts.contains(&RvExtension::I));
        assert!(exts.contains(&RvExtension::M));
        assert!(exts.contains(&RvExtension::A));
        assert!(exts.contains(&RvExtension::C));
    }

    #[test]
    fn isa_string_parser_rv32i_minimal() {
        let (xlen, exts) = parse_isa_string("rv32i").expect("parse");
        assert_eq!(xlen, 32);
        assert_eq!(exts.len(), 1);
        assert!(exts.contains(&RvExtension::I));
    }

    #[test]
    fn isa_string_parser_g_shorthand_expands_to_imafd() {
        let (xlen, exts) = parse_isa_string("rv64g").expect("parse");
        assert_eq!(xlen, 64);
        for ext in [
            RvExtension::I,
            RvExtension::M,
            RvExtension::A,
            RvExtension::F,
            RvExtension::D,
        ] {
            assert!(exts.contains(&ext), "G shorthand must include {ext:?}");
        }
    }

    #[test]
    fn isa_string_parser_zicsr_via_underscore_chunk() {
        let (xlen, exts) = parse_isa_string("rv64imac_zicsr_zifencei").expect("parse");
        assert_eq!(xlen, 64);
        assert!(exts.contains(&RvExtension::Zicsr));
        assert!(exts.contains(&RvExtension::Zifencei));
    }

    #[test]
    fn isa_string_parser_rejects_missing_rv_prefix() {
        let err = parse_isa_string("64imac").unwrap_err();
        assert_eq!(err, IsaStringError::MissingRvPrefix);
    }

    #[test]
    fn isa_string_parser_rejects_unknown_letter() {
        let err = parse_isa_string("rv64ixq").unwrap_err();
        assert!(
            matches!(err, IsaStringError::UnknownLetter('x')),
            "expected UnknownLetter('x'), got {err:?}",
        );
    }

    #[test]
    fn rv_extension_try_from_char() {
        assert_eq!(RvExtension::try_from('i').unwrap(), RvExtension::I);
        assert_eq!(RvExtension::try_from('M').unwrap(), RvExtension::M);
        assert!(matches!(
            RvExtension::try_from('x'),
            Err(IsaStringError::UnknownLetter('x')),
        ));
    }

    #[test]
    fn rv_extension_from_str_letter_and_zname() {
        let i: RvExtension = "i".parse().unwrap();
        assert_eq!(i, RvExtension::I);
        let zicsr: RvExtension = "zicsr".parse().unwrap();
        assert_eq!(zicsr, RvExtension::Zicsr);
        let bad: Result<RvExtension, _> = "rocketship".parse();
        assert!(
            matches!(bad, Err(IsaStringError::UnknownZName(_))),
            "got {bad:?}",
        );
    }

    #[test]
    fn parsed_isa_from_str_round_trips_through_parse_isa_string() {
        let parsed: ParsedIsa = "rv64imac_zicsr".parse().unwrap();
        let (xlen, exts) = parse_isa_string("rv64imac_zicsr").unwrap();
        assert_eq!(parsed.xlen, xlen);
        assert_eq!(parsed.extensions, exts);
    }

    #[test]
    fn parsed_isa_from_tuple_drops_unknown_extensions() {
        // Forward-compat path: a TOML config naming a future
        // extension shouldn't break a downscoped fuzz session. The
        // From<(u8, IntoIter<&str>)> impl silently filters them.
        let parsed: ParsedIsa = (64u8, ["i", "m", "futureXYZ", "zicsr"].into_iter()).into();
        assert_eq!(parsed.xlen, 64);
        assert!(parsed.extensions.contains(&RvExtension::I));
        assert!(parsed.extensions.contains(&RvExtension::M));
        assert!(parsed.extensions.contains(&RvExtension::Zicsr));
        assert_eq!(parsed.extensions.len(), 3, "futureXYZ must be dropped");
    }

    #[test]
    fn parsed_isa_always_contains_i() {
        // Even when the input doesn't mention it: I is implicit.
        let parsed: ParsedIsa = (32u8, ["m"].into_iter()).into();
        assert!(parsed.extensions.contains(&RvExtension::I));
        assert!(parsed.extensions.contains(&RvExtension::M));
    }
}
