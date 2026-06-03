//! Tool dispatch: recognize input kinds, shell out to clang/yosys/dart.

pub mod cache;
pub mod chain;
pub mod error;
pub mod mock;
pub mod raw_rv64_elf;
pub mod trait_def;

#[cfg(feature = "clang-asm")]
pub mod clang_asm;

pub use chain::ToolChain;
pub use error::ToolError;
pub use mock::MockTool;
pub use raw_rv64_elf::{FUZZ_LOAD_ADDR, RawBytesToElfRiscv};
pub use trait_def::{Result, TargetSpec, Tool, ToolOpts};
