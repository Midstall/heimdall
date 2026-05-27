//! Minimal SPICE netlist parser sufficient for layout-extracted netlists
//! (R, C, L, V, I, D plus MOSFETs M*/Q*). Subcircuit bodies are skipped.
//! Directives (`.tran`, `.ic`, `.options`, `.end`, `.model`, ...) are ignored.

use std::collections::BTreeSet;

use crate::error::GoldenError;
use crate::trait_def::Result;

/// Categorical device kind. Determines fixed terminal count for layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    Resistor,
    Capacitor,
    Inductor,
    VoltageSource,
    CurrentSource,
    Diode,
    Mosfet,
    Bjt,
    Subcircuit,
    /// Anything we don't recognize. Terminals are best-effort.
    Other,
}

impl DeviceKind {
    /// Number of terminal nodes the device consumes from the head of the
    /// line. MOSFETs and BJTs consume 4 (drain/gate/source/body for M,
    /// collector/base/emitter/substrate for Q). Two-terminal devices consume 2.
    pub fn n_terminals(self) -> usize {
        match self {
            Self::Resistor
            | Self::Capacitor
            | Self::Inductor
            | Self::VoltageSource
            | Self::CurrentSource
            | Self::Diode => 2,
            Self::Mosfet | Self::Bjt => 4,
            Self::Subcircuit => 0, // X devices have variable terminal count handled separately
            Self::Other => 0,
        }
    }

    pub fn from_first_char(c: char) -> Option<Self> {
        match c.to_ascii_uppercase() {
            'R' => Some(Self::Resistor),
            'C' => Some(Self::Capacitor),
            'L' => Some(Self::Inductor),
            'V' => Some(Self::VoltageSource),
            'I' => Some(Self::CurrentSource),
            'D' => Some(Self::Diode),
            'M' => Some(Self::Mosfet),
            'Q' => Some(Self::Bjt),
            'X' => Some(Self::Subcircuit),
            _ => None,
        }
    }

    /// Short glyph used in SVG rendering.
    pub fn glyph(self) -> &'static str {
        match self {
            Self::Resistor => "R",
            Self::Capacitor => "C",
            Self::Inductor => "L",
            Self::VoltageSource => "V",
            Self::CurrentSource => "I",
            Self::Diode => "D",
            Self::Mosfet => "M",
            Self::Bjt => "Q",
            Self::Subcircuit => "X",
            Self::Other => "?",
        }
    }
}

/// One device in the netlist.
#[derive(Debug, Clone)]
pub struct SpiceDevice {
    pub name: String,
    pub kind: DeviceKind,
    pub terminals: Vec<String>,
    /// The remainder of the line after the terminal nodes (value + model + params),
    /// joined with single spaces. Kept verbatim for label rendering.
    pub tail: String,
}

/// Parsed graph: a flat list of devices plus the set of all distinct nets.
#[derive(Debug, Clone, Default)]
pub struct SpiceGraph {
    pub devices: Vec<SpiceDevice>,
    pub nets: Vec<String>,
}

impl SpiceGraph {
    pub fn n_devices(&self) -> usize {
        self.devices.len()
    }

    pub fn n_nets(&self) -> usize {
        self.nets.len()
    }

    /// Find the index of a net by name. Case-insensitive (SPICE is too).
    pub fn net_index(&self, name: &str) -> Option<usize> {
        self.nets.iter().position(|n| n.eq_ignore_ascii_case(name))
    }
}

/// Strip a trailing `; ...` comment and surrounding whitespace.
fn strip_inline_comment(line: &str) -> &str {
    if let Some(idx) = line.find(';') {
        line[..idx].trim_end()
    } else {
        line.trim_end()
    }
}

/// Lex a line into whitespace-separated tokens, lowercased to follow SPICE
/// case-insensitivity.
fn tokens(line: &str) -> Vec<String> {
    line.split_whitespace().map(|t| t.to_lowercase()).collect()
}

/// Pre-process: join continuation lines (`+` at start) into their predecessor,
/// drop pure `*` comment lines, drop blank lines.
fn coalesce_lines(src: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in src.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('*') {
            continue;
        }
        let stripped = strip_inline_comment(line);
        if stripped.is_empty() {
            continue;
        }
        if let Some(rest) = stripped.strip_prefix('+') {
            if let Some(last) = out.last_mut() {
                last.push(' ');
                last.push_str(rest.trim_start());
                continue;
            }
            // Continuation with no predecessor: treat as standalone.
            out.push(rest.trim_start().to_string());
            continue;
        }
        out.push(stripped.to_string());
    }
    out
}

/// Parse the netlist into a [`SpiceGraph`]. Best-effort: lines we can't make
/// sense of become `DeviceKind::Other` with their raw tokens as terminals so
/// the caller still sees something rather than nothing.
pub fn parse_netlist(src: &str) -> Result<SpiceGraph> {
    let mut devices: Vec<SpiceDevice> = Vec::new();
    let mut nets: BTreeSet<String> = BTreeSet::new();

    let mut in_subckt = false;

    for line in coalesce_lines(src) {
        // Directives skipped. Track .subckt nesting so devices inside
        // subcircuit bodies are not parsed without first being expanded.
        if line.starts_with('.') {
            let lower = line.to_lowercase();
            if lower.starts_with(".subckt") {
                in_subckt = true;
            } else if lower.starts_with(".ends") {
                in_subckt = false;
            }
            continue;
        }
        if in_subckt {
            continue;
        }

        let toks = tokens(&line);
        if toks.is_empty() {
            continue;
        }

        let name = &toks[0];
        let Some(first) = name.chars().next() else {
            continue;
        };
        let kind = DeviceKind::from_first_char(first).unwrap_or(DeviceKind::Other);

        let device = match kind {
            DeviceKind::Subcircuit => {
                // `X<name> n1 n2 ... <subckt_model>`: last token is the
                // model, the rest after the device name are terminals.
                if toks.len() < 3 {
                    return Err(GoldenError::NetlistParse(format!(
                        "X-device `{name}` needs at least 1 terminal + a model"
                    )));
                }
                let terminals: Vec<String> = toks[1..toks.len() - 1].to_vec();
                let tail = toks.last().cloned().unwrap_or_default();
                SpiceDevice {
                    name: name.clone(),
                    kind,
                    terminals,
                    tail,
                }
            }
            DeviceKind::Other => SpiceDevice {
                name: name.clone(),
                kind,
                terminals: toks[1..].to_vec(),
                tail: String::new(),
            },
            known => {
                let n = known.n_terminals();
                // Need n terminals + at least one value/model token after them.
                // Without this, `R1 onlyone 1k` would silently swallow the
                // value `1k` as a second terminal, which is wrong.
                if toks.len() < 1 + n + 1 {
                    return Err(GoldenError::NetlistParse(format!(
                        "device `{name}` ({:?}) needs {n} terminals + a value, got {} token(s) after the name",
                        known,
                        toks.len() - 1
                    )));
                }
                let terminals: Vec<String> = toks[1..1 + n].to_vec();
                let tail = toks[1 + n..].join(" ");
                SpiceDevice {
                    name: name.clone(),
                    kind: known,
                    terminals,
                    tail,
                }
            }
        };

        for net in &device.terminals {
            nets.insert(net.clone());
        }
        devices.push(device);
    }

    let mut nets_vec: Vec<String> = nets.into_iter().collect();
    // Ensure deterministic ordering with ground first if present (SPICE
    // convention puts node 0 first in legend tables).
    nets_vec.sort_by(|a, b| match (a.as_str(), b.as_str()) {
        ("0", "0") => std::cmp::Ordering::Equal,
        ("0", _) => std::cmp::Ordering::Less,
        (_, "0") => std::cmp::Ordering::Greater,
        _ => a.cmp(b),
    });

    Ok(SpiceGraph {
        devices,
        nets: nets_vec,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rc_divider() {
        let src = "* RC divider\n\
                   R1 in mid 1k\n\
                   R2 mid 0 1k\n\
                   C1 mid 0 1n\n\
                   .ic v(mid)=0\n\
                   .end\n";
        let g = parse_netlist(src).expect("parse");
        assert_eq!(g.n_devices(), 3);
        assert_eq!(g.nets, vec!["0", "in", "mid"]);
        assert_eq!(g.devices[0].name, "r1");
        assert_eq!(g.devices[0].kind, DeviceKind::Resistor);
        assert_eq!(g.devices[0].terminals, vec!["in", "mid"]);
        assert_eq!(g.devices[0].tail, "1k");
        assert_eq!(g.devices[2].kind, DeviceKind::Capacitor);
    }

    #[test]
    fn parses_inverter_mosfets() {
        let src = "* Synthetic CMOS inverter\n\
                   M1 out in vdd vdd PMOS L=180n W=2u\n\
                   M2 out in 0   0   NMOS L=180n W=1u\n\
                   .end\n";
        let g = parse_netlist(src).expect("parse");
        assert_eq!(g.n_devices(), 2);
        assert_eq!(g.devices[0].kind, DeviceKind::Mosfet);
        assert_eq!(g.devices[0].terminals, vec!["out", "in", "vdd", "vdd"]);
        assert!(g.devices[0].tail.contains("pmos"));
        assert!(g.nets.contains(&"vdd".to_string()));
        assert!(g.nets.contains(&"out".to_string()));
    }

    #[test]
    fn handles_continuation_lines() {
        let src = "R1 in mid\n\
                   + 1k\n\
                   .end\n";
        let g = parse_netlist(src).expect("parse");
        assert_eq!(g.n_devices(), 1);
        assert_eq!(g.devices[0].tail, "1k");
    }

    #[test]
    fn skips_subckt_bodies() {
        let src = "R1 a b 1k\n\
                   .subckt inv vdd in out gnd\n\
                   M1 out in vdd vdd PMOS\n\
                   M2 out in gnd gnd NMOS\n\
                   .ends inv\n\
                   R2 b c 2k\n\
                   .end\n";
        let g = parse_netlist(src).expect("parse");
        assert_eq!(g.n_devices(), 2);
        assert_eq!(g.devices[0].name, "r1");
        assert_eq!(g.devices[1].name, "r2");
    }

    #[test]
    fn rejects_too_few_terminals() {
        let src = "R1 onlyone 1k\n.end\n";
        let err = parse_netlist(src).expect_err("should reject");
        let msg = format!("{err}");
        assert!(msg.contains("needs 2 terminals"), "unexpected: {msg}");
    }

    #[test]
    fn rejects_resistor_missing_value() {
        let src = "R1 a b\n.end\n";
        let err = parse_netlist(src).expect_err("should reject missing value");
        let msg = format!("{err}");
        assert!(msg.contains("value"), "unexpected: {msg}");
    }

    #[test]
    fn ground_is_first_in_nets() {
        let src = "R1 a 0 1k\nR2 0 b 1k\n.end\n";
        let g = parse_netlist(src).expect("parse");
        assert_eq!(g.nets[0], "0");
    }

    #[test]
    fn case_insensitive_lookup() {
        let src = "R1 IN MID 1k\nR2 mid 0 1k\n.end\n";
        let g = parse_netlist(src).expect("parse");
        assert!(g.net_index("IN").is_some());
        assert!(g.net_index("in").is_some());
        assert!(g.net_index("mid").is_some());
    }

    #[test]
    fn handles_inline_comments() {
        let src = "R1 in mid 1k ; small bias resistor\nR2 mid 0 1k\n.end\n";
        let g = parse_netlist(src).expect("parse");
        assert_eq!(g.n_devices(), 2);
        assert_eq!(g.devices[0].tail, "1k");
    }

    #[test]
    fn unknown_kind_becomes_other() {
        let src = "Z1 a b 1k\n.end\n";
        let g = parse_netlist(src).expect("parse");
        assert_eq!(g.devices[0].kind, DeviceKind::Other);
        // Other captures all post-name tokens as best-effort terminals.
        assert_eq!(g.devices[0].terminals, vec!["a", "b", "1k"]);
    }
}
