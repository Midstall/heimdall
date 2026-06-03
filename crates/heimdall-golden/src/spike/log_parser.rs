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
        // Spike's log emits the destination as a single `xN` token (no
        // space) in current builds, but older versions / log formatters
        // emit `x` and `N` as separate tokens. Accept both.
        let Some(kind) = toks.next() else { continue };
        let reg = if kind == "x" {
            let Some(reg_tok) = toks.next() else { continue };
            let Ok(r) = reg_tok.parse::<usize>() else {
                continue;
            };
            r
        } else if let Some(rest) = kind.strip_prefix('x') {
            let Ok(r) = rest.parse::<usize>() else {
                continue;
            };
            r
        } else {
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
    fn parses_minimal_commit_line_spaced_form() {
        // Older / synthetic spike logs emit the register as `x 2`.
        let log = "core   0: 3 0x0000000080000010 (0xfe010113) x 2 0x000000007ffffff0\n";
        let s = parse_final_state(log).unwrap();
        assert_eq!(s.fields.get("x2"), Some(&ValueRepr::U64(0x7fff_fff0)));
        assert_eq!(s.fields.get("pc"), Some(&ValueRepr::U64(0x8000_0010)));
    }

    #[test]
    fn parses_minimal_commit_line_packed_form() {
        // Current upstream / nixpkgs spike emits `x10` as one token.
        // This is the form that surfaced when wiring SpikeOneShot to the
        // real binary.
        let log = "core   0: 3 0x00000000000100b0 (0x04200513) x10 0x0000000000000042\n";
        let s = parse_final_state(log).unwrap();
        assert_eq!(s.fields.get("x10"), Some(&ValueRepr::U64(0x42)));
        assert_eq!(s.fields.get("pc"), Some(&ValueRepr::U64(0x0001_00b0)));
    }

    #[test]
    fn latest_write_wins() {
        let log = "\
core   0: 3 0x80000010 (0x...) x1 0x1
core   0: 3 0x80000014 (0x...) x1 0x2
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

    /// Verbatim slice of a real spike `--log-commits` output for the
    /// program `addi x10, x0, 0; addi x10, x0, 0x42; ebreak` loaded at
    /// 0x10000. Captured from spike 1.1.0-unstable-2024-09-21. spike's
    /// `-d` debug mode interleaves pre-commit lines (no priv level, no
    /// register write, just disasm) with the actual `--log-commits`
    /// retire lines (with priv level + the `xN 0x...` write). The
    /// parser MUST take the LAST `xN` value seen, otherwise the
    /// register-zeroing prologue baked into fuzz programs shadows the
    /// real body result and golden state looks like the program never
    /// ran past the prologue.
    const REAL_SPIKE_LOG_TWO_WRITES_X10: &str = "\
core   0: 3 0x0000000000001000 (0x00000297) x5  0x0000000000001000
core   0: 3 0x0000000000001004 (0x02028593) x11 0x0000000000001020
core   0: 3 0x0000000000001008 (0xf1402573) x10 0x0000000000000000
core   0: 3 0x000000000000100c (0x0182b283) x5  0x0000000000010000 mem 0x0000000000001018
core   0: 3 0x0000000000001010 (0x00028067)
core   0: 0x0000000000010000 (0x00000513) li      a0, 0
core   0: 3 0x0000000000010000 (0x00000513) x10 0x0000000000000000
core   0: 0x0000000000010004 (0x04200513) li      a0, 66
core   0: 3 0x0000000000010004 (0x04200513) x10 0x0000000000000042
core   0: 0x0000000000010008 (0x00100073) ebreak
core   0: exception trap_breakpoint, epc 0x0000000000010008
";

    #[test]
    fn real_spike_log_keeps_last_write_per_register() {
        // Body's x10=0x42 must override:
        // 1. the bootrom's csrr a0,mhartid (writes x10=0)
        // 2. the prologue-equivalent first body insn (writes x10=0)
        // Same expectation for x11 / x5 written by the bootrom.
        let s = parse_final_state(REAL_SPIKE_LOG_TWO_WRITES_X10).unwrap();
        assert_eq!(
            s.fields.get("x10"),
            Some(&ValueRepr::U64(0x42)),
            "x10 must reflect the LAST write (body's 0x42), not the prologue/bootrom 0",
        );
        assert_eq!(
            s.fields.get("x11"),
            Some(&ValueRepr::U64(0x1020)),
            "x11 must reflect bootrom's dtb pointer",
        );
        // x5 has two bootrom writes: AUIPC -> 0x1000, then ld -> 0x10000.
        // Last-write-wins must yield 0x10000.
        assert_eq!(
            s.fields.get("x5"),
            Some(&ValueRepr::U64(0x10000)),
            "x5 must reflect the LAST bootrom write (the loaded entry)",
        );
    }
}
