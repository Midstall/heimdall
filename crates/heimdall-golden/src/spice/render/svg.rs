//! Render a parsed [`SpiceGraph`] + computed [`Layout`] to an SVG string,
//! optionally overlaying coverage from a [`SpiceCoverage`] snapshot.
//!
//! No external SVG library: the output is hand-written XML so the heimdall
//! crate stays lightweight and the SVG is byte-stable across builds (good
//! for snapshot tests and for the daemon to serve directly).
//!
//! Color palette is Tokyo Night to match the daemon's web UI.

use std::fmt::Write as _;

use super::layout::Layout;
use super::parse::{DeviceKind, SpiceGraph};
use crate::spice::{SpiceCoverage, SpiceDir, SpiceWatch};
use crate::trait_def::CoverageSource;

/// Color tokens. Local to the renderer because it's the only consumer.
mod colors {
    pub const BG: &str = "#1a1b26";
    pub const FG: &str = "#c0caf5";
    pub const GRID: &str = "#414868";
    pub const NET_UNKNOWN: &str = "#565f89";
    pub const NET_ACTIVE: &str = "#9ece6a"; // covered output
    pub const NET_INACTIVE_WATCHED: &str = "#e0af68"; // watched but no activity
    pub const NET_INPUT: &str = "#7aa2f7"; // watched input
    pub const DEVICE_FILL: &str = "#24283b";
    pub const DEVICE_STROKE: &str = "#7aa2f7";
    pub const EDGE: &str = "#414868";
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum NetColorClass {
    Unknown,
    ActiveOutput,
    InactiveWatchedOutput,
    Input,
}

impl NetColorClass {
    fn fill(self) -> &'static str {
        match self {
            Self::Unknown => colors::NET_UNKNOWN,
            Self::ActiveOutput => colors::NET_ACTIVE,
            Self::InactiveWatchedOutput => colors::NET_INACTIVE_WATCHED,
            Self::Input => colors::NET_INPUT,
        }
    }
}

/// Knobs for [`render_svg`]. Borrows the coverage + watch context so the
/// caller doesn't have to allocate auxiliary structures.
pub struct RenderOpts<'a> {
    /// Coverage snapshot to color outputs by. `None` renders all nets gray.
    pub coverage: Option<&'a SpiceCoverage>,
    /// Watches identifying which nets are inputs vs outputs.
    pub watches: &'a [SpiceWatch],
    pub width: u32,
    pub height: u32,
}

impl<'a> Default for RenderOpts<'a> {
    fn default() -> Self {
        Self {
            coverage: None,
            watches: &[],
            width: 600,
            height: 400,
        }
    }
}

/// Classify each net for coloring. We resolve watches against the graph by
/// matching `spice_node` against the parsed net names (case-insensitive).
fn classify_nets(graph: &SpiceGraph, opts: &RenderOpts<'_>) -> Vec<NetColorClass> {
    // Index outputs in declaration order to map them to coverage bits.
    let outputs: Vec<&SpiceWatch> = opts
        .watches
        .iter()
        .filter(|w| matches!(w.direction, SpiceDir::Out))
        .collect();
    let cov_bits = opts.coverage.map(|c| c.snapshot()).unwrap_or_default();

    let bit_set = |idx: usize| -> bool {
        let byte = idx / 8;
        let bit = (idx % 8) as u8;
        cov_bits
            .get(byte)
            .map(|b| b & (1 << bit) != 0)
            .unwrap_or(false)
    };

    graph
        .nets
        .iter()
        .map(|net| {
            // Output match first (so outputs win over inputs if a node is both).
            if let Some(out_idx) = outputs
                .iter()
                .position(|w| w.spice_node.eq_ignore_ascii_case(net))
            {
                if opts.coverage.is_some() && bit_set(out_idx) {
                    return NetColorClass::ActiveOutput;
                }
                return NetColorClass::InactiveWatchedOutput;
            }
            if opts.watches.iter().any(|w| {
                matches!(w.direction, SpiceDir::In) && w.spice_node.eq_ignore_ascii_case(net)
            }) {
                return NetColorClass::Input;
            }
            NetColorClass::Unknown
        })
        .collect()
}

/// Minimal XML escape for label text. We only emit a controlled subset of
/// strings (net + device names + tail tokens), so just handling `<`, `>`,
/// `&`, `"` is sufficient.
fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

/// Truncate label tails (which can include MOSFET parameter strings) so
/// they don't blow up the SVG layout.
fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max.saturating_sub(1)])
    }
}

pub fn render_svg(graph: &SpiceGraph, layout: &Layout, opts: &RenderOpts<'_>) -> String {
    let classes = classify_nets(graph, opts);
    let mut s = String::with_capacity(8 * 1024);

    let w = opts.width;
    let h = opts.height;

    let _ = writeln!(
        s,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}" width="{w}" height="{h}" font-family="ui-monospace, monospace" font-size="11">"#
    );

    // Background.
    let _ = writeln!(
        s,
        r#"<rect width="100%" height="100%" fill="{}"/>"#,
        colors::BG
    );

    // Edges first (under devices/nets).
    for d in &graph.devices {
        let net_idxs: Vec<usize> = d
            .terminals
            .iter()
            .filter_map(|n| graph.net_index(n))
            .collect();
        if net_idxs.is_empty() {
            continue;
        }
        let centroid = layout.centroid_of(&net_idxs);
        for idx in &net_idxs {
            let p = layout.position(*idx);
            let _ = writeln!(
                s,
                r#"<line x1="{:.1}" y1="{:.1}" x2="{:.1}" y2="{:.1}" stroke="{}" stroke-width="1"/>"#,
                centroid.x,
                centroid.y,
                p.x,
                p.y,
                colors::EDGE
            );
        }
    }

    // Devices: small rounded rect at the centroid of their terminals.
    for d in &graph.devices {
        let net_idxs: Vec<usize> = d
            .terminals
            .iter()
            .filter_map(|n| graph.net_index(n))
            .collect();
        if net_idxs.is_empty() {
            continue;
        }
        let c = layout.centroid_of(&net_idxs);
        let box_w = 26.0;
        let box_h = 16.0;
        let _ = writeln!(
            s,
            r#"<rect x="{:.1}" y="{:.1}" width="{}" height="{}" rx="3" ry="3" fill="{}" stroke="{}" stroke-width="1"/>"#,
            c.x - box_w / 2.0,
            c.y - box_h / 2.0,
            box_w,
            box_h,
            colors::DEVICE_FILL,
            colors::DEVICE_STROKE
        );
        // Device label: glyph + truncated name. The name often duplicates
        // the glyph (e.g., "R1" has glyph "R") so just show the name itself.
        let label = xml_escape(&truncate(&d.name, 6));
        let _ = writeln!(
            s,
            r#"<text x="{:.1}" y="{:.1}" text-anchor="middle" dominant-baseline="central" fill="{}">{}</text>"#,
            c.x,
            c.y,
            colors::FG,
            label
        );
        // Glyph in upper-right corner for the kind at a glance.
        if d.kind != DeviceKind::Other {
            let _ = writeln!(
                s,
                r#"<text x="{:.1}" y="{:.1}" text-anchor="end" dominant-baseline="hanging" fill="{}" font-size="8" opacity="0.7">{}</text>"#,
                c.x + box_w / 2.0 - 2.0,
                c.y - box_h / 2.0 + 2.0,
                colors::FG,
                d.kind.glyph()
            );
        }
    }

    // Net dots + labels.
    for (i, net) in graph.nets.iter().enumerate() {
        let p = layout.position(i);
        let class = classes[i];
        let _ = writeln!(
            s,
            r#"<circle cx="{:.1}" cy="{:.1}" r="6" fill="{}" stroke="{}" stroke-width="1.5"/>"#,
            p.x,
            p.y,
            class.fill(),
            colors::GRID
        );
        let _ = writeln!(
            s,
            r#"<text x="{:.1}" y="{:.1}" fill="{}">{}</text>"#,
            p.x + 9.0,
            p.y + 3.0,
            colors::FG,
            xml_escape(net)
        );
    }

    // Legend (always rendered; small enough to coexist with dense graphs).
    let legend_x = 10.0;
    let legend_y = (h as f64) - 64.0;
    let entries: &[(NetColorClass, &str)] = &[
        (NetColorClass::ActiveOutput, "covered"),
        (NetColorClass::InactiveWatchedOutput, "watched"),
        (NetColorClass::Input, "input"),
        (NetColorClass::Unknown, "no info"),
    ];
    for (i, (cls, label)) in entries.iter().enumerate() {
        let y = legend_y + (i as f64) * 14.0;
        let _ = writeln!(
            s,
            r#"<circle cx="{:.1}" cy="{:.1}" r="5" fill="{}"/>"#,
            legend_x,
            y,
            cls.fill()
        );
        let _ = writeln!(
            s,
            r#"<text x="{:.1}" y="{:.1}" fill="{}">{}</text>"#,
            legend_x + 10.0,
            y + 3.0,
            colors::FG,
            label
        );
    }

    s.push_str("</svg>\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spice::render::layout::layout_force_directed;
    use crate::spice::render::parse::parse_netlist;
    use crate::spice::{SpiceDir, SpiceWatch};

    fn rc_divider_graph() -> SpiceGraph {
        let src = "R1 in mid 1k\nR2 mid 0 1k\nC1 mid 0 1n\n.end\n";
        parse_netlist(src).expect("parse")
    }

    #[test]
    fn svg_contains_expected_structure() {
        let g = rc_divider_graph();
        let l = layout_force_directed(&g, 400.0, 300.0);
        let svg = render_svg(&g, &l, &RenderOpts::default());
        assert!(svg.starts_with("<svg"));
        assert!(svg.ends_with("</svg>\n"));
        // SPICE is case-insensitive. The parser lowercases identifiers.
        assert!(svg.contains(">r1<"));
        assert!(svg.contains(">r2<"));
        assert!(svg.contains(">c1<"));
        assert!(svg.contains(">in<"));
        assert!(svg.contains(">mid<"));
        assert!(svg.contains(">0<"));
    }

    #[test]
    fn coverage_overlay_colors_active_output_green() {
        let g = rc_divider_graph();
        let l = layout_force_directed(&g, 400.0, 300.0);

        // Build watches: in is input, mid is output.
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

        // Coverage where the single output (index 0) is set.
        let mut bits = vec![0u8; SpiceCoverage::buckets()];
        bits[0] = 1; // bit 0 set
        let cov = SpiceCoverage::from_bits_for_test(bits);

        let opts = RenderOpts {
            coverage: Some(&cov),
            watches: &watches,
            width: 400,
            height: 300,
        };
        let svg = render_svg(&g, &l, &opts);
        assert!(
            svg.contains("#9ece6a"),
            "expected active-output green in: {svg}"
        );
        assert!(svg.contains("#7aa2f7"), "expected input blue in: {svg}");
    }

    #[test]
    fn watched_output_without_coverage_renders_amber() {
        let g = rc_divider_graph();
        let l = layout_force_directed(&g, 400.0, 300.0);
        let watches = vec![SpiceWatch {
            name: "io_mid".into(),
            spice_node: "mid".into(),
            direction: SpiceDir::Out,
        }];
        let bits = vec![0u8; SpiceCoverage::buckets()];
        let cov = SpiceCoverage::from_bits_for_test(bits);
        let opts = RenderOpts {
            coverage: Some(&cov),
            watches: &watches,
            width: 400,
            height: 300,
        };
        let svg = render_svg(&g, &l, &opts);
        assert!(svg.contains("#e0af68"), "expected watched-but-cold amber");
    }

    #[test]
    fn xml_escape_handles_specials() {
        assert_eq!(xml_escape("a<b&c>\"d"), "a&lt;b&amp;c&gt;&quot;d");
    }

    #[test]
    fn truncate_adds_ellipsis() {
        assert_eq!(truncate("abcd", 10), "abcd");
        assert_eq!(truncate("abcdefghij", 5), "abcd…");
    }
}
