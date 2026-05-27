//! Integration tests for the SPICE netlist parser + SVG renderer in
//! heimdall-golden::spice::render. These run unconditionally (no ngspice
//! required): the renderer is pure-Rust.

#![cfg(feature = "spice")]

use std::path::PathBuf;

use heimdall_golden::{
    RenderOpts, SpiceCoverage, SpiceDir, SpiceWatch, layout_force_directed, parse_netlist,
    render_netlist, render_svg,
};

fn workspace_testdata(rel: &str) -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .join("..")
        .join("..")
        .join("testdata")
        .join(rel)
}

#[test]
fn rc_divider_round_trip_renders_valid_svg() {
    let path = workspace_testdata("spice/rc_divider.sp");
    let src = std::fs::read_to_string(&path).expect("read netlist");
    let svg = render_netlist(&src, None, &[], 500, 400).expect("render");
    assert!(svg.starts_with("<svg"));
    assert!(svg.ends_with("</svg>\n"));
    // Three devices, three nets.
    assert!(svg.contains(">r1<"));
    assert!(svg.contains(">r2<"));
    assert!(svg.contains(">c1<"));
    assert!(svg.contains(">in<"));
    assert!(svg.contains(">mid<"));
    assert!(svg.contains(">0<"));
    // Tokyo Night background.
    assert!(svg.contains("#1a1b26"));
}

#[test]
fn layout_is_stable_across_invocations() {
    let path = workspace_testdata("spice/rc_divider.sp");
    let src = std::fs::read_to_string(&path).expect("read");
    let a = render_netlist(&src, None, &[], 500, 400).expect("render a");
    let b = render_netlist(&src, None, &[], 500, 400).expect("render b");
    assert_eq!(a, b, "render output must be deterministic for caching");
}

#[test]
fn coverage_overlay_changes_node_color() {
    let path = workspace_testdata("spice/rc_divider.sp");
    let src = std::fs::read_to_string(&path).expect("read");

    let watches = vec![
        SpiceWatch {
            name: "io_in".into(),
            spice_node: "in".into(),
            direction: SpiceDir::In,
        },
        SpiceWatch {
            name: "io_mid".into(),
            spice_node: "mid".into(),
            direction: SpiceDir::Out,
        },
    ];

    // Coverage where mid (output index 0) is active.
    let mut bits = vec![0u8; SpiceCoverage::buckets()];
    bits[0] = 1;
    let cov_active = SpiceCoverage::from_bits_for_test(bits);

    let graph = parse_netlist(&src).expect("parse");
    let layout = layout_force_directed(&graph, 500.0, 400.0);

    let svg_active = render_svg(
        &graph,
        &layout,
        &RenderOpts {
            coverage: Some(&cov_active),
            watches: &watches,
            width: 500,
            height: 400,
        },
    );
    let svg_cold = render_svg(
        &graph,
        &layout,
        &RenderOpts {
            coverage: Some(&SpiceCoverage::from_bits_for_test(
                vec![0u8; SpiceCoverage::buckets()],
            )),
            watches: &watches,
            width: 500,
            height: 400,
        },
    );

    assert_ne!(svg_active, svg_cold, "coverage flip should change output");
    assert!(
        svg_active.contains("#9ece6a"),
        "active coverage should color mid green"
    );
    assert!(
        svg_cold.contains("#e0af68"),
        "cold coverage on a watched output should be amber"
    );
}

#[test]
fn inverter_renders_with_mosfet_glyphs() {
    let src = "M1 out in vdd vdd PMOS L=180n W=2u\n\
               M2 out in 0   0   NMOS L=180n W=1u\n\
               .end\n";
    let svg = render_netlist(src, None, &[], 500, 400).expect("render");
    assert!(svg.contains(">m1<"));
    assert!(svg.contains(">m2<"));
    // MOSFET glyph in the device upper-right.
    assert!(svg.contains(">M<"));
    // Both supplies + signals as nets.
    assert!(svg.contains(">vdd<"));
    assert!(svg.contains(">out<"));
    assert!(svg.contains(">in<"));
    assert!(svg.contains(">0<"));
}

#[test]
fn legend_is_present() {
    let src = "R1 a b 1k\n.end\n";
    let svg = render_netlist(src, None, &[], 400, 300).expect("render");
    assert!(svg.contains(">covered<"));
    assert!(svg.contains(">watched<"));
    assert!(svg.contains(">input<"));
    assert!(svg.contains(">no info<"));
}

#[test]
fn render_handles_empty_netlist_directives_only() {
    let src = "* nothing but directives\n.tran 1n 100n\n.end\n";
    let svg = render_netlist(src, None, &[], 200, 150).expect("render");
    assert!(svg.starts_with("<svg"));
    assert!(svg.ends_with("</svg>\n"));
}
