use heimdall_core::{Evidence, FailureKind, State, Verdict};

/// State keys that the architectural diff intentionally ignores. These are
/// observed and recorded on both DUT and golden snapshots for debugging,
/// but they're not comparable as a verdict input because the two sides
/// stop at different points and the value is fundamentally "where am I
/// right now" rather than "what did the program compute".
///
/// `pc` belongs here because:
/// - DUT free-runs and is force-halted by JTAG `wait_halt` at an arbitrary
///   instruction boundary.
/// - Spike runs exactly `max_cycles` and stops.
/// - Random fuzz programs without a clean ebreak halt at different points
///   in the trap path, so PCs reliably disagree even when GPR results
///   match. Comparing pc here would mask every real GPR-level divergence
///   behind a noise-floor pc-mismatch.
///
/// Tests that need a deterministic stopping point (e.g. bring-up firmware
/// with a known ebreak) can still inspect `dut_state.fields.get("pc")`
/// outside the diff.
const DIFF_IGNORED_KEYS: &[&str] = &["pc"];

/// Architectural-state diff. Considers every key in `golden` that is also
/// present in `dut`, except keys in [`DIFF_IGNORED_KEYS`]. Keys only in
/// dut are not checked (they may be auxiliary info the golden does not
/// model).
pub fn diff_states(dut: &State, golden: &State) -> Verdict {
    for (k, expected) in &golden.fields {
        if DIFF_IGNORED_KEYS.contains(&k.as_str()) {
            continue;
        }
        match dut.fields.get(k) {
            Some(got) if got == expected => continue,
            Some(got) => {
                return Verdict::Fail {
                    kind: FailureKind::DiffMismatch {
                        field: k.clone(),
                        got: got.clone(),
                        expected: expected.clone(),
                    },
                    evidence: vec![Evidence {
                        label: "field".into(),
                        detail: k.clone(),
                    }],
                };
            }
            None => {
                return Verdict::Fail {
                    kind: FailureKind::MissingField {
                        field: k.clone(),
                        expected: expected.clone(),
                    },
                    evidence: vec![Evidence {
                        label: "missing-field".into(),
                        detail: k.clone(),
                    }],
                };
            }
        }
    }
    Verdict::Pass
}

#[cfg(test)]
mod tests {
    use super::*;
    use heimdall_core::ValueRepr;

    #[test]
    fn match_passes() {
        let s = State::new().with("x10", ValueRepr::U64(1));
        let v = diff_states(&s, &s);
        assert!(matches!(v, Verdict::Pass));
    }

    #[test]
    fn mismatch_reports_field() {
        let dut = State::new().with("x10", ValueRepr::U64(1));
        let golden = State::new().with("x10", ValueRepr::U64(2));
        let v = diff_states(&dut, &golden);
        assert!(matches!(v, Verdict::Fail { .. }));
    }

    #[test]
    fn pc_mismatch_does_not_fail() {
        // The DUT halted at the entry PC, spike halted somewhere else in
        // its trap path. GPRs match; the verdict must be Pass.
        let dut = State::new()
            .with("x10", ValueRepr::U64(0x42))
            .with("pc", ValueRepr::U64(0x10000));
        let golden = State::new()
            .with("x10", ValueRepr::U64(0x42))
            .with("pc", ValueRepr::U64(0x0));
        let v = diff_states(&dut, &golden);
        assert!(matches!(v, Verdict::Pass), "got {v:?}");
    }

    #[test]
    fn gpr_mismatch_still_fails_even_if_pc_disagrees() {
        // pc skipped, but x11 mismatch survives the diff.
        let dut = State::new()
            .with("x11", ValueRepr::U64(0x1))
            .with("pc", ValueRepr::U64(0x10000));
        let golden = State::new()
            .with("x11", ValueRepr::U64(0x2))
            .with("pc", ValueRepr::U64(0x0));
        match diff_states(&dut, &golden) {
            Verdict::Fail {
                kind: FailureKind::DiffMismatch { field, .. },
                ..
            } => assert_eq!(field, "x11"),
            other => panic!("expected fail on x11, got {other:?}"),
        }
    }
}
