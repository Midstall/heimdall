use heimdall_core::{State, ValueRepr};

use crate::error::GoldenError;

/// Parse a spike `--log-commits` stream and return the final architectural
/// state (final GPR values).
///
/// Spike commit lines look like:
///   core   0: 3 0x0000000080000010 (0xfe010113) x 2 0x000000007ffffff0
///
/// We accumulate the most recent value written to each xN, then emit it as
/// `xN` in the resulting State.
pub fn parse_final_state(log: &str) -> Result<State, GoldenError> {
    let mut xregs: [Option<u64>; 32] = [None; 32];
    let mut last_pc: Option<u64> = None;

    for line in log.lines() {
        let Some(rest) = line.split_once(':').map(|x| x.1.trim()) else {
            continue;
        };
        let mut toks = rest.split_whitespace();
        let Some(_priv) = toks.next() else { continue };
        let Some(pc_tok) = toks.next() else { continue };
        if let Some(stripped) = pc_tok.strip_prefix("0x") {
            if let Ok(pc) = u64::from_str_radix(stripped, 16) {
                last_pc = Some(pc);
            }
        }
        let Some(_insn) = toks.next() else { continue };
        let Some(kind) = toks.next() else { continue };
        if kind != "x" {
            continue;
        }
        let Some(reg_tok) = toks.next() else { continue };
        let Ok(reg): std::result::Result<usize, _> = reg_tok.parse() else {
            continue;
        };
        if reg >= 32 {
            continue;
        }
        let Some(val_tok) = toks.next() else { continue };
        if let Some(stripped) = val_tok.strip_prefix("0x") {
            if let Ok(val) = u64::from_str_radix(stripped, 16) {
                xregs[reg] = Some(val);
            }
        }
    }

    let mut state = State::new();
    for (i, v) in xregs.iter().enumerate() {
        if let Some(v) = v {
            state = state.with(format!("x{i}"), ValueRepr::U64(*v));
        }
    }
    if let Some(pc) = last_pc {
        state = state.with("pc", ValueRepr::U64(pc));
    }
    Ok(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_commit_line() {
        let log = "core   0: 3 0x0000000080000010 (0xfe010113) x 2 0x000000007ffffff0\n";
        let s = parse_final_state(log).unwrap();
        assert_eq!(s.fields.get("x2"), Some(&ValueRepr::U64(0x7fff_fff0)));
        assert_eq!(s.fields.get("pc"), Some(&ValueRepr::U64(0x8000_0010)));
    }

    #[test]
    fn latest_write_wins() {
        let log = "\
core   0: 3 0x80000010 (0x...) x 1 0x1
core   0: 3 0x80000014 (0x...) x 1 0x2
";
        let s = parse_final_state(log).unwrap();
        assert_eq!(s.fields.get("x1"), Some(&ValueRepr::U64(2)));
    }

    #[test]
    fn ignores_garbage_lines() {
        let log = "some\nrandom\nstdout\n";
        let s = parse_final_state(log).unwrap();
        assert!(s.fields.is_empty());
    }
}
