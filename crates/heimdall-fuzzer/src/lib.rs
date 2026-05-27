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
pub use generator::CraneliftGen;
pub use generator::{RawAsmGen, Rv64};
pub use mutator::{BitFlipMutator, ByteFlipMutator, SpliceMutator};
pub use scheduler::{PowerScheduler, RoundRobinScheduler};

pub use engine::{DivergenceFinding, FuzzReport, FuzzerEngine, FuzzerEngineBuilder};
pub use fuzz_test::AdHocFuzzTest;
#[cfg(feature = "aegis")]
pub use generator::BitstreamGen;
pub use heimdall_golden::CoverageSource;
pub use traits::{Generator, Mutator, Scheduler, SchedulerChoice};
