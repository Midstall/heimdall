//! Minimal ngspice ASCII .raw parser. Extracts per-variable traces.

#[derive(Debug, Clone)]
pub struct RawTrace {
    pub name: String,
    pub kind: String,
    pub values: Vec<f64>,
}

impl RawTrace {
    /// Match by name OR by `v(<node>)` for voltage signals.
    pub fn matches_node(&self, node: &str) -> bool {
        let lower = self.name.to_ascii_lowercase();
        let want = node.to_ascii_lowercase();
        lower == want || lower == format!("v({want})")
    }

    pub fn range(&self) -> f64 {
        if self.values.is_empty() {
            return 0.0;
        }
        let mut min = f64::INFINITY;
        let mut max = f64::NEG_INFINITY;
        for &v in &self.values {
            if v < min {
                min = v;
            }
            if v > max {
                max = v;
            }
        }
        max - min
    }

    pub fn last_value(&self) -> f64 {
        self.values.last().copied().unwrap_or(0.0)
    }
}

/// Parse the ASCII section of an ngspice .raw file. The format has:
///   Title: ...
///   Plotname: ...
///   Flags: real
///   No. Variables: N
///   No. Points: P
///   Variables:
///       0  time  time
///       1  v(foo) voltage
///       ...
///   Values:
///     <idx> <time> <var1> <var2> ...
///
/// We collect per-variable Vec<f64>.
pub fn parse_raw_ascii(text: &str) -> Result<Vec<RawTrace>, String> {
    let mut n_vars = 0usize;
    let mut n_points = 0usize;
    let mut variables: Vec<(String, String)> = Vec::new(); // (name, kind)
    let mut in_variables = false;
    let mut in_values = false;
    let mut values_rows: Vec<Vec<f64>> = Vec::new();
    let mut current_row: Vec<f64> = Vec::new();
    let mut current_row_size = 0usize;

    for line in text.lines() {
        let l = line.trim();
        if l.is_empty() {
            continue;
        }
        if !in_values {
            if let Some(rest) = l.strip_prefix("No. Variables:") {
                n_vars = rest.trim().parse().map_err(|e| format!("n_vars: {e}"))?;
            } else if let Some(rest) = l.strip_prefix("No. Points:") {
                n_points = rest.trim().parse().map_err(|e| format!("n_points: {e}"))?;
            } else if l == "Variables:" {
                in_variables = true;
            } else if l == "Values:" {
                in_variables = false;
                in_values = true;
                current_row_size = 1 + n_vars; // index + N vars per row
            } else if in_variables {
                // "  0  time  time"
                let toks: Vec<&str> = l.split_whitespace().collect();
                if toks.len() >= 3 {
                    variables.push((toks[1].to_string(), toks[2].to_string()));
                }
            }
        } else {
            // Values section.
            for tok in l.split_whitespace() {
                if let Ok(v) = tok.parse::<f64>() {
                    current_row.push(v);
                }
                if current_row.len() == current_row_size {
                    values_rows.push(std::mem::take(&mut current_row));
                }
            }
        }
    }

    let _ = n_points; // non-fatal if mismatch

    let mut traces: Vec<RawTrace> = variables
        .iter()
        .map(|(name, kind)| RawTrace {
            name: name.clone(),
            kind: kind.clone(),
            values: Vec::with_capacity(values_rows.len()),
        })
        .collect();
    for row in &values_rows {
        // row[0] = index, row[1..] = variable values (time is var 0 at row[1])
        for (i, v) in row.iter().skip(1).enumerate() {
            if let Some(t) = traces.get_mut(i) {
                t.values.push(*v);
            }
        }
    }

    Ok(traces)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "Title: heimdall test
Plotname: Transient Analysis
Flags: real
No. Variables: 3
No. Points: 4
Variables:
        0       time    time
        1       v(in)   voltage
        2       v(out)  voltage
Values:
 0      0.000e+00       0.000e+00       0.000e+00
 1      1.000e-09       1.800e+00       0.500e+00
 2      2.000e-09       1.800e+00       1.700e+00
 3      3.000e-09       1.800e+00       1.800e+00
";

    #[test]
    fn parse_sample() {
        let traces = parse_raw_ascii(SAMPLE).expect("parse");
        assert_eq!(traces.len(), 3);
        let time = &traces[0];
        let v_in = &traces[1];
        let v_out = &traces[2];
        assert_eq!(time.name, "time");
        assert_eq!(v_in.name, "v(in)");
        assert_eq!(v_out.name, "v(out)");
        assert_eq!(v_in.values.len(), 4);
        assert!((v_in.range() - 1.8).abs() < 1e-9);
        assert!((v_out.last_value() - 1.8).abs() < 1e-9);
    }

    #[test]
    fn matches_node_with_v_prefix() {
        let t = RawTrace {
            name: "v(io_2)".into(),
            kind: "voltage".into(),
            values: vec![],
        };
        assert!(t.matches_node("io_2"));
        assert!(t.matches_node("IO_2"));
        assert!(!t.matches_node("io_3"));
    }
}
