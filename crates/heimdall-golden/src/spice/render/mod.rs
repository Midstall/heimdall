//! SPICE netlist parser + SVG renderer with optional coverage overlay.
//!
//! The renderer is library-only in v1: callers (daemon, tests, downstream
//! crates) supply a netlist text + an optional [`SpiceCoverage`] snapshot and
//! receive an SVG string.

mod layout;
mod parse;
mod svg;

use crate::spice::SpiceWatch;

pub use layout::{Layout, layout_force_directed};
pub use parse::{DeviceKind, SpiceDevice, SpiceGraph, parse_netlist};
pub use svg::{RenderOpts, render_svg};

/// Convenience entry point: parse + layout + render in one call.
pub fn render_netlist(
    netlist: &str,
    coverage: Option<&crate::spice::SpiceCoverage>,
    watches: &[SpiceWatch],
    width: u32,
    height: u32,
) -> crate::trait_def::Result<String> {
    let graph = parse_netlist(netlist)?;
    let layout = layout_force_directed(&graph, width as f64, height as f64);
    let opts = RenderOpts {
        coverage,
        watches,
        width,
        height,
    };
    Ok(render_svg(&graph, &layout, &opts))
}
