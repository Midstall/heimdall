//! Deterministic Fruchterman-Reingold layout for SpiceGraph.
//!
//! Nets are placed as graph nodes; devices act as edges between their
//! terminal nets. A multi-terminal device (e.g., 4-terminal MOSFET) is
//! expanded into pairwise edges among its terminals so the springs pull
//! the whole device's nets together.
//!
//! Determinism: the initial RNG is seeded with a fixed constant, so the
//! same netlist always produces the same coordinates.

use super::parse::SpiceGraph;

/// 2-D position for a single net.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NetPosition {
    pub x: f64,
    pub y: f64,
}

/// Computed layout: net index -> position. Bounded by the requested
/// canvas width/height with a small margin reserved for labels.
#[derive(Debug, Clone)]
pub struct Layout {
    pub positions: Vec<NetPosition>,
    pub width: f64,
    pub height: f64,
}

impl Layout {
    pub fn position(&self, net_idx: usize) -> NetPosition {
        self.positions[net_idx]
    }

    /// Midpoint between several net positions, used to place device boxes.
    pub fn centroid_of(&self, net_indices: &[usize]) -> NetPosition {
        if net_indices.is_empty() {
            return NetPosition {
                x: self.width / 2.0,
                y: self.height / 2.0,
            };
        }
        let mut x = 0.0;
        let mut y = 0.0;
        for &i in net_indices {
            x += self.positions[i].x;
            y += self.positions[i].y;
        }
        NetPosition {
            x: x / net_indices.len() as f64,
            y: y / net_indices.len() as f64,
        }
    }
}

/// Deterministic xorshift64 RNG. Avoids pulling in `rand` just for layout
/// and guarantees byte-identical SVG output across builds.
#[derive(Debug, Clone, Copy)]
struct Xorshift64 {
    state: u64,
}

impl Xorshift64 {
    fn new(seed: u64) -> Self {
        // Seed must be non-zero for xorshift. Fold a constant in to be safe.
        Self {
            state: seed.wrapping_add(0x9E3779B97F4A7C15),
        }
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    /// Uniform f64 in [0, 1).
    fn next_unit(&mut self) -> f64 {
        // Use the top 53 bits as the mantissa.
        ((self.next_u64() >> 11) as f64) / ((1u64 << 53) as f64)
    }
}

/// Run Fruchterman-Reingold for a fixed number of iterations.
///
/// The classic FR formulation: nodes repel each other (Coulomb-like), and
/// edges attract via spring forces. A linearly cooling "temperature" caps
/// the per-step displacement so the layout settles.
pub fn layout_force_directed(graph: &SpiceGraph, width: f64, height: f64) -> Layout {
    let n = graph.n_nets().max(1);
    let margin = 30.0_f64;
    let inner_w = (width - 2.0 * margin).max(50.0);
    let inner_h = (height - 2.0 * margin).max(50.0);

    let area = inner_w * inner_h;
    let k = (area / n as f64).sqrt();

    // Deterministic initial placement.
    let mut rng = Xorshift64::new(0xD157A11C);
    let mut positions: Vec<NetPosition> = (0..n)
        .map(|_| NetPosition {
            x: margin + rng.next_unit() * inner_w,
            y: margin + rng.next_unit() * inner_h,
        })
        .collect();

    // Pin ground (net 0 in our sorted list, if present) to bottom-center
    // so layouts feel schematic-like rather than rotational.
    let pin: Option<usize> = graph.net_index("0");
    if let Some(idx) = pin {
        positions[idx] = NetPosition {
            x: margin + inner_w / 2.0,
            y: margin + inner_h - 10.0,
        };
    }

    // Build edge list. Each device contributes pairwise edges among its
    // terminal nets: two-terminal devices give one edge, MOSFETs give six.
    let mut edges: Vec<(usize, usize)> = Vec::new();
    for d in &graph.devices {
        let idxs: Vec<usize> = d
            .terminals
            .iter()
            .filter_map(|n| graph.net_index(n))
            .collect();
        for i in 0..idxs.len() {
            for j in (i + 1)..idxs.len() {
                if idxs[i] != idxs[j] {
                    edges.push((idxs[i], idxs[j]));
                }
            }
        }
    }

    // FR parameters.
    let iterations = 200usize;
    let mut temperature = inner_w.min(inner_h) / 4.0;
    let cool = temperature / iterations as f64;

    let mut disp: Vec<(f64, f64)> = vec![(0.0, 0.0); n];

    for _ in 0..iterations {
        // Repulsive forces.
        for d in disp.iter_mut().take(n) {
            *d = (0.0, 0.0);
        }
        for v in 0..n {
            for u in 0..n {
                if u == v {
                    continue;
                }
                let dx = positions[v].x - positions[u].x;
                let dy = positions[v].y - positions[u].y;
                let dist = (dx * dx + dy * dy).sqrt().max(0.01);
                let force = (k * k) / dist;
                disp[v].0 += (dx / dist) * force;
                disp[v].1 += (dy / dist) * force;
            }
        }

        // Attractive forces along edges.
        for &(a, b) in &edges {
            let dx = positions[a].x - positions[b].x;
            let dy = positions[a].y - positions[b].y;
            let dist = (dx * dx + dy * dy).sqrt().max(0.01);
            let force = (dist * dist) / k;
            let fx = (dx / dist) * force;
            let fy = (dy / dist) * force;
            disp[a].0 -= fx;
            disp[a].1 -= fy;
            disp[b].0 += fx;
            disp[b].1 += fy;
        }

        // Apply, capped by temperature, then clamp within the canvas.
        for v in 0..n {
            if Some(v) == pin {
                continue;
            }
            let (dx, dy) = disp[v];
            let mag = (dx * dx + dy * dy).sqrt().max(0.01);
            let step = mag.min(temperature);
            positions[v].x += (dx / mag) * step;
            positions[v].y += (dy / mag) * step;
            positions[v].x = positions[v].x.clamp(margin, margin + inner_w);
            positions[v].y = positions[v].y.clamp(margin, margin + inner_h);
        }

        temperature -= cool;
        if temperature < 0.5 {
            temperature = 0.5;
        }
    }

    // Final dedupe pass: if two nets ended up too close, nudge them apart
    // by a deterministic small amount keyed off their indices. This keeps
    // small graphs (where FR has little to push against) readable.
    for v in 0..n {
        for u in 0..v {
            let dx = positions[v].x - positions[u].x;
            let dy = positions[v].y - positions[u].y;
            let dist = (dx * dx + dy * dy).sqrt();
            if dist < 24.0 {
                let nudge = 24.0 - dist;
                let ang = ((v as f64) * 1.13 + (u as f64) * 0.71).sin();
                positions[v].x =
                    (positions[v].x + nudge * ang.cos()).clamp(margin, margin + inner_w);
                positions[v].y =
                    (positions[v].y + nudge * ang.sin()).clamp(margin, margin + inner_h);
            }
        }
    }

    Layout {
        positions,
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spice::render::parse::parse_netlist;

    #[test]
    fn layout_is_deterministic() {
        let src = "R1 in mid 1k\nR2 mid 0 1k\nC1 mid 0 1n\n.end\n";
        let g = parse_netlist(src).expect("parse");
        let a = layout_force_directed(&g, 400.0, 300.0);
        let b = layout_force_directed(&g, 400.0, 300.0);
        assert_eq!(a.positions, b.positions);
    }

    #[test]
    fn ground_is_pinned_bottom_center() {
        let src = "R1 a 0 1k\nR2 b 0 1k\n.end\n";
        let g = parse_netlist(src).expect("parse");
        let l = layout_force_directed(&g, 400.0, 300.0);
        let idx = g.net_index("0").unwrap();
        let p = l.positions[idx];
        assert!((p.x - 200.0).abs() < 1.0, "ground x near center: {p:?}");
        assert!(p.y > 200.0, "ground y near bottom: {p:?}");
    }

    #[test]
    fn positions_stay_inside_canvas() {
        let src = "R1 a b 1k\nR2 b c 1k\nR3 c d 1k\nR4 d a 1k\n.end\n";
        let g = parse_netlist(src).expect("parse");
        let l = layout_force_directed(&g, 500.0, 400.0);
        for p in &l.positions {
            assert!(p.x >= 0.0 && p.x <= 500.0, "x in bounds: {p:?}");
            assert!(p.y >= 0.0 && p.y <= 400.0, "y in bounds: {p:?}");
        }
    }

    #[test]
    fn nodes_separated_after_layout() {
        let src = "R1 a b 1k\nR2 c d 1k\n.end\n";
        let g = parse_netlist(src).expect("parse");
        let l = layout_force_directed(&g, 400.0, 300.0);
        for i in 0..l.positions.len() {
            for j in (i + 1)..l.positions.len() {
                let dx = l.positions[i].x - l.positions[j].x;
                let dy = l.positions[i].y - l.positions[j].y;
                let d = (dx * dx + dy * dy).sqrt();
                assert!(d >= 20.0, "nets {i} and {j} too close: d={d}");
            }
        }
    }
}
