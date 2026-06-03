//! Heimdall coverage-guided fuzzer. MVP: random generation, classic mutation,
//! differential oracle via the same Runner directed tests use.

pub mod corpus;
pub mod coverage;
pub mod engine;
pub mod error;
pub mod fuzz_test;
pub mod generator; // FZ3/FZ4
pub mod mutator;
pub mod scheduler;
pub mod traits;

pub use corpus::{Corpus, CorpusEntry, VerdictTag};
pub use coverage::{CoverageDiff, CoverageMap, CoverageSnapshot, DEFAULT_BUCKETS};
pub use error::{FuzzerError, Result};
#[cfg(feature = "cranelift")]
pub use generator::{CraneliftGen, CraneliftInitError};
pub use generator::{
    EBREAK_INSN, IsaStringError, IsaTag, ParsedIsa, RawAsmGen, Rv32, Rv64, RvExtension,
    parse_isa_string,
};
pub use mutator::{BitFlipMutator, ByteFlipMutator, SpliceMutator};
pub use scheduler::{PowerScheduler, RoundRobinScheduler};

pub use engine::{
    DivergenceFinding, FuzzReport, FuzzerEngine, FuzzerEngineBuilder, IterCallback, ProgramCallback,
};
pub use fuzz_test::AdHocFuzzTest;
#[cfg(feature = "aegis")]
pub use generator::BitstreamGen;
pub use heimdall_golden::CoverageSource;
pub use traits::{Generator, Mutator, Scheduler, SchedulerChoice};
