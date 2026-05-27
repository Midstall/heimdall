use heimdall_core::{Evidence, FailureKind, State, Verdict};

/// Architectural-state diff. Considers every key in `golden` that is also
/// present in `dut`. Keys only in dut are not checked (they may be auxiliary
/// info the golden does not model).
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
    fn match_passes() {
        let s = State::new().with("a0", ValueRepr::U64(1));
        let v = diff_states(&s, &s);
        assert!(matches!(v, Verdict::Pass));
    }

    #[test]
    fn mismatch_reports_field() {
        let dut = State::new().with("a0", ValueRepr::U64(1));
        let golden = State::new().with("a0", ValueRepr::U64(2));
        let v = diff_states(&dut, &golden);
        assert!(matches!(v, Verdict::Fail { .. }));
    }
}
