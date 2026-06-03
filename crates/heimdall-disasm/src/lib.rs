//! Human-readable disassembly of Heimdall program artifacts.
//!
//! The MVP wraps `llvm-objdump` as an out-of-process disassembler. The
//! trait surface is shaped so an in-process Rust decoder (e.g. a
//! workspace-internal RV64 disassembler) can drop in later without
//! touching callers. The intent is "view what the DUT is executing"
//! for the web UI and TUI: render the instruction stream with the
//! current observe PC highlighted.
//!
//! Library-first: callers are the daemon route + future TUI; this crate
//! has zero web/HTTP surface of its own.

pub mod error;
pub mod listing;
pub mod llvm_objdump;
pub mod trait_def;

pub use error::DisasmError;
pub use listing::{DisasmLine, DisasmListing};
pub use llvm_objdump::LlvmObjdumpDisassembler;
pub use trait_def::{DisasmOpts, Disassembler, Isa, Result};
