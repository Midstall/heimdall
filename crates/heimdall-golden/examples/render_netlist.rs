//! Render an arbitrary SPICE netlist to SVG, optionally with a synthetic
//! coverage snapshot. Useful for previewing the renderer output and for
//! debugging new netlists.
//!
//! Usage:
//!   cargo run --example render_netlist --features spice -- \
//!       <netlist.sp> <out.svg> [watched_node[:in|out] ...]
//!
//! If any watched outputs are listed, the example marks them all as
//! "active" in coverage so the green overlay is visible.

#[cfg(feature = "spice")]
fn main() {
    use heimdall_golden::{SpiceCoverage, SpiceDir, SpiceWatch, render_netlist};

    let mut args = std::env::args().skip(1);
    let netlist_path = args
        .next()
        .expect("usage: <netlist.sp> <out.svg> [watch[:in|out] ...]");
    let out_path = args
        .next()
        .expect("usage: <netlist.sp> <out.svg> [watch[:in|out] ...]");

    let mut watches: Vec<SpiceWatch> = Vec::new();
    let mut output_count = 0usize;
    for spec in args {
        let (node, dir) = if let Some((n, d)) = spec.split_once(':') {
            let direction = match d {
                "in" => SpiceDir::In,
                "out" => SpiceDir::Out,
                other => panic!("watch direction must be 'in' or 'out', got `{other}`"),
            };
            (n.to_string(), direction)
        } else {
            (spec, SpiceDir::Out)
        };
        if matches!(dir, SpiceDir::Out) {
            output_count += 1;
        }
        watches.push(SpiceWatch {
            name: node.clone(),
            spice_node: node,
            direction: dir,
        });
    }

    let src = std::fs::read_to_string(&netlist_path).expect("read netlist");

    let coverage = if output_count > 0 {
        let mut bits = vec![0u8; SpiceCoverage::buckets()];
        // Mark every output as active so each one renders green.
        for idx in 0..output_count {
            bits[idx / 8] |= 1 << (idx % 8);
        }
        Some(SpiceCoverage::from_bits_for_test(bits))
    } else {
        None
    };

    let svg = render_netlist(&src, coverage.as_ref(), &watches, 700, 500).expect("render");
    std::fs::write(&out_path, &svg).expect("write svg");
    eprintln!("wrote {} ({} bytes)", out_path, svg.len());
}

#[cfg(not(feature = "spice"))]
fn main() {
    eprintln!("rebuild with --features spice to use this example");
    std::process::exit(2);
}
