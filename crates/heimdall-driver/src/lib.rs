//! Per-target test drivers. Each driver composes a Transport set + a ToolChain
//! + a GoldenModel into a compile-load-run-observe-diff flow.

pub mod error;
pub mod mock;
pub mod trait_def;

#[cfg(feature = "river")]
pub mod river;

#[cfg(feature = "aegis")]
pub mod aegis;

pub use error::DriverError;
pub use mock::MockDriver;
pub use trait_def::{Dut, Result, TestDriver};
