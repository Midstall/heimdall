//! Golden-model abstraction for differential testing against spike, river_emulator,
//! aegis sim. Backends as impls of `GoldenModel`.

pub mod error;
pub mod mock;
pub mod trait_def;

#[cfg(feature = "spike")]
pub mod spike;

#[cfg(feature = "spike")]
pub use spike::{SpikeCoverage, SpikeOneShot};

#[cfg(feature = "aegis")]
pub mod aegis;

#[cfg(feature = "aegis")]
pub use aegis::AegisGoldenModel;

#[cfg(feature = "spice")]
pub mod spice;

#[cfg(feature = "spice")]
pub use spice::{RawTrace, SpiceCoverage, SpiceDir, SpiceGoldenModel, SpiceWatch, parse_raw_ascii};

#[cfg(feature = "spice")]
pub use spice::render::{
    DeviceKind, Layout, RenderOpts, SpiceDevice, SpiceGraph, layout_force_directed, parse_netlist,
    render_netlist, render_svg,
};

pub use error::GoldenError;
pub use mock::{MockCoverage, MockGoldenModel};
pub use trait_def::{CoverageSource, GoldenModel, Result, StepOutcome};
