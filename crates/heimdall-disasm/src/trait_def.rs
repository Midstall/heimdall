use async_trait::async_trait;
use heimdall_core::Artifact;
use serde::{Deserialize, Serialize};

use crate::error::DisasmError;
use crate::listing::DisasmListing;

pub type Result<T> = std::result::Result<T, DisasmError>;

/// Instruction-set tag the disassembler needs to know about. RV64 today,
/// aarch64/x86 follow when other DUT families come online. Carrying ISA
/// in the opts (rather than inferring from artifact kind) keeps the
/// trait honest about the multi-ISA future and avoids surprise behavior
/// when a RawBytes blob is fed in without an ELF header to sniff.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS), ts(export))]
pub enum Isa {
    Rv64,
}

impl Isa {
    /// llvm-objdump triple for this ISA. Used when disassembling raw
    /// machine code (`-b binary`) where there is no ELF header.
    pub fn llvm_triple(self) -> &'static str {
        match self {
            Isa::Rv64 => "riscv64",
        }
    }
}

/// Options that control how an artifact is disassembled.
#[derive(Debug, Clone)]
pub struct DisasmOpts {
    pub isa: Isa,
    /// Base address to apply to raw-bytes artifacts. Ignored for ELF
    /// inputs (they carry their own load address via PT_LOAD/p_paddr).
    /// Default matches the fuzz path's [`FUZZ_LOAD_ADDR`] (0x10000).
    ///
    /// [`FUZZ_LOAD_ADDR`]: ../../heimdall_tools/raw_rv64_elf/constant.FUZZ_LOAD_ADDR.html
    pub raw_base_addr: u64,
    /// Extension flags to hand to llvm-objdump as `--mattr=+<a>,+<b>`.
    /// This is intentionally what the DUT actually has enabled, not a
    /// permissive superset: if the codegen accidentally emits an
    /// instruction the DUT can't execute (e.g. a `mul` on a non-M
    /// part), we want the listing to show `<unknown>` so the bug
    /// surfaces in review. Empty means base ISA only.
    pub extensions: Vec<String>,
}

impl Default for DisasmOpts {
    fn default() -> Self {
        Self {
            isa: Isa::Rv64,
            raw_base_addr: 0x10000,
            extensions: Vec::new(),
        }
    }
}

/// Turn a program artifact into a structured listing for rendering.
///
/// The trait is async because the initial implementation shells out;
/// in-process implementations are free to return immediately. Errors
/// must NOT be swallowed: a UI that silently shows "no disassembly"
/// when llvm-objdump is missing or the artifact is unsupported is
/// worse than one that says exactly which step failed. Consumers
/// surface the error verbatim ([`feedback_failures_should_fail`]).
#[async_trait]
pub trait Disassembler: Send + Sync {
    /// Stable identifier for diagnostics and choosing among multiple
    /// available implementations. Keep it slug-like; the daemon route
    /// echoes it back in the JSON response.
    fn name(&self) -> &str;

    /// True when this disassembler understands `kind`. Today every
    /// concrete impl supports ElfRiscv + RawBytes (both feed into
    /// llvm-objdump via different entry points), but isolating the
    /// capability check on the trait lets future Aegis-bitstream or
    /// SPICE-netlist "disassemblers" join the family without expanding
    /// the artifact-kind switch in every caller.
    fn supports(&self, kind: heimdall_core::ArtifactKind) -> bool;

    async fn disassemble(&self, artifact: &Artifact, opts: &DisasmOpts) -> Result<DisasmListing>;
}
