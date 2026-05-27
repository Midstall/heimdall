//! Heimdall umbrella crate. Re-exports the public surface so third-party
//! programs add one dep and write `use heimdall::*`.

pub use heimdall_config as config;
pub use heimdall_core as core;
pub use heimdall_driver as driver;
pub use heimdall_golden as golden;
pub use heimdall_test as test;
pub use heimdall_tools as tools;
pub use heimdall_transport as transport;

pub use heimdall_core::{
    Artifact, ArtifactKind, DutId, DutKind, Evidence, FailureKind, Observation, RunId, SeedId,
    SkipReason, State, StepBudget, Stimulus, ValueRepr, Verdict,
};
pub use heimdall_driver::{Dut, MockDriver, TestDriver};
pub use heimdall_golden::{GoldenModel, MockGoldenModel};
pub use heimdall_test::{BuildCtx, Plan, RunResult, Runner, Test};
pub use heimdall_tools::{TargetSpec, Tool, ToolChain, ToolOpts};
pub use heimdall_transport::{JtagOps, ResetTarget, SerialOps, Transport, TransportKind};

#[cfg(feature = "aegis")]
pub use heimdall_golden::AegisGoldenModel;

#[cfg(feature = "fuzzer")]
pub use heimdall_fuzzer as fuzzer;

#[cfg(feature = "daemon")]
pub use heimdall_daemon as daemon;

#[cfg(feature = "tui")]
pub use heimdall_tui as tui;

#[cfg(feature = "fuzzer")]
pub use heimdall_fuzzer::{
    AdHocFuzzTest, BitFlipMutator, ByteFlipMutator, FuzzReport, FuzzerEngine, FuzzerEngineBuilder,
    RawAsmGen, RoundRobinScheduler, Rv64, SpliceMutator,
};

/// A single error type for users who want to collapse the per-crate errors at
/// their own boundary.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("config: {0}")]
    Config(#[from] heimdall_config::ConfigError),
    #[error("transport: {0}")]
    Transport(#[from] heimdall_transport::TransportError),
    #[error("tool: {0}")]
    Tool(#[from] heimdall_tools::ToolError),
    #[error("golden: {0}")]
    Golden(#[from] heimdall_golden::GoldenError),
    #[error("driver: {0}")]
    Driver(#[from] heimdall_driver::DriverError),
    #[error("test: {0}")]
    Test(#[from] heimdall_test::TestError),
}

pub type Result<T> = std::result::Result<T, Error>;
