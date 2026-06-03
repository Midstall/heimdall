use serde::{Deserialize, Serialize};

/// One decoded instruction. `addr` is the architectural address the
/// instruction lives at (so the UI can highlight the line whose addr
/// matches the observed pc). `bytes` is the raw machine code in little-
/// endian order; useful for both render and bug-bounty triage. The
/// mnemonic and operands are pre-split so the UI can style them
/// independently (mnemonics bold, operands dim, register names with
/// the ABI overlay we already use for State rendering).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS), ts(export))]
pub struct DisasmLine {
    pub addr: u64,
    pub bytes: Vec<u8>,
    pub mnemonic: String,
    pub operands: String,
}

/// Structured output of [`crate::Disassembler::disassemble`]. `name`
/// is the disassembler identifier so the UI / API consumer can show
/// "decoded by llvm-objdump 21.1.8" or similar. `isa` is the ISA the
/// listing was decoded against, so a downstream consumer doesn't have
/// to thread DisasmOpts through to render.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "bindings", derive(ts_rs::TS), ts(export))]
pub struct DisasmListing {
    pub name: String,
    pub isa: crate::trait_def::Isa,
    pub lines: Vec<DisasmLine>,
}

impl DisasmListing {
    /// Find the listing line for a given architectural address. Returns
    /// the index into `lines` so the caller can render and scroll to
    /// it. Linear scan; lines are typically < 1k entries for fuzz
    /// programs and < 100k even for bring-up firmware.
    pub fn line_index_for_addr(&self, addr: u64) -> Option<usize> {
        self.lines.iter().position(|l| l.addr == addr)
    }
}
