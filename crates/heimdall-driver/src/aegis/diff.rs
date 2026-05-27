//! State diff for Aegis. Identical algorithm to `river::diff` because the
//! State shape is the same; kept as a separate module so per-target driver
//! modules stay independent.

use heimdall_core::{Evidence, FailureKind, State, Verdict};

/// Architectural-state diff. Considers every key in `golden` that is also
/// present in `dut`. Keys only in dut are not checked.
pub fn diff_states(dut: &State, golden: &State) -> Verdict {
    for (k, expected) in &golden.fields {
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
                    kind: FailureKind::DiffMismatch {
                        field: k.clone(),
                        got: heimdall_core::ValueRepr::Bool(false),
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
    fn pad_match_passes() {
        let s = State::new().with("io_0", ValueRepr::Bool(true));
        assert!(matches!(diff_states(&s, &s), Verdict::Pass));
    }

    #[test]
    fn pad_mismatch_reports_field() {
        let dut = State::new().with("io_0", ValueRepr::Bool(false));
        let golden = State::new().with("io_0", ValueRepr::Bool(true));
        assert!(matches!(diff_states(&dut, &golden), Verdict::Fail { .. }));
    }
}
