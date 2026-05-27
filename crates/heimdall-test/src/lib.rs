//! Test harness: Test trait + Runner. Composes Driver + GoldenModel + ToolChain.

pub mod error;
pub mod runner;
pub mod test_trait;

pub use error::TestError;
pub use runner::{RunResult, Runner, RunnerBuilder};
pub use test_trait::{BuildCtx, Plan, Test};
